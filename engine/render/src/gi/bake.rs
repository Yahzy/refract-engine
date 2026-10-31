//! Offline baking of light on the GPU, with ray queries.
//!
//! Probes: every pass traces the same rays from every probe; what a ray hits
//! is lit by the sun and the lights near it, plus the previous pass's probes,
//! so each pass adds one bounce. Reflection cubemaps are then traced from
//! their positions, their hits lit the same way with the finished probes.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use refract_asset::map::{LIGHT_SPOT, Light, Probe, ProbeGrid};
use wgpu::util::DeviceExt;

use super::rays::RayScene;
use crate::gpu::{Gpu, RAY_QUERY};
use crate::shader::{self, spirv};
use crate::{RenderError, Sky};

/// Size of the cells that sort lights for the bake, world units.
const LIGHT_CELL: f32 = 512.0;

pub struct BakeInput<'a> {
    /// Triangles blocking and bouncing light, world space.
    pub positions: &'a [[f32; 3]],
    pub indices: &'a [u32],
    /// Linear albedo of each triangle; alpha 1 for thin two-sided surfaces
    /// (leaves, fences), lit on both sides: rays reaching their back are not
    /// inside anything.
    pub albedo: &'a [[f32; 4]],
    /// Direction the sunlight travels, and its colour times intensity.
    pub sun: Option<(Vec3, Vec3)>,
    pub sky: Sky,
    pub lights: &'a [Light],
    pub grid: ProbeGrid,
    /// Light bounces to compute (passes).
    pub bounces: u32,
    /// Where to bake reflection cubemaps, and their face size.
    pub reflections: &'a [Vec3],
    pub reflection_size: u32,
    /// The sky as a cubemap (`Rgba16Float`), seen by reflection rays that hit nothing.
    pub sky_cubemap: Option<&'a refract_asset::TextureData<'a>>,
}

pub struct Baked {
    /// x fastest, then y, then z.
    pub probes: Vec<Probe>,
    /// Linear RGBA per cubemap, face (+X, -X, +Y, -Y, +Z, -Z), row, column.
    pub reflections: Vec<[f32; 4]>,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ReflectionParams {
    size: u32,
    count: u32,
    _pad: [u32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    grid_origin: [f32; 3],
    cell_size: f32,
    resolution: [u32; 3],
    pass_index: u32,
    sun_direction: [f32; 3],
    has_sun: f32,
    sun_color: [f32; 3],
    _pad0: f32,
    sky_zenith: [f32; 3],
    _pad1: f32,
    sky_horizon: [f32; 3],
    _pad2: f32,
    sky_ground: [f32; 3],
    _pad3: f32,
    light_grid_origin: [f32; 3],
    light_grid_cell: f32,
    light_grid_resolution: [u32; 3],
    _pad4: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuLight {
    position: [f32; 3],
    range: f32,
    color: [f32; 3],
    kind: u32,
    direction: [f32; 3],
    cos_outer: f32,
    cos_inner: f32,
    clearance: f32,
    _pad: [f32; 2],
}

/// Whether this machine's GPU can bake (ray queries).
pub fn available() -> bool {
    pollster::block_on(Gpu::instance().enumerate_adapters(wgpu::Backends::VULKAN)).iter().any(|a| a.features().contains(RAY_QUERY))
}

/// Bakes every probe of `input.grid`, then the reflection cubemaps.
pub fn bake(input: &BakeInput) -> Result<Baked, RenderError> {
    let gpu = Gpu::with_features(Gpu::instance(), None, RAY_QUERY)?;
    let (device, queue) = (&gpu.device, &gpu.queue);
    let scene = RayScene::new(device, queue, input.positions, input.indices);
    let probe_count = input.grid.resolution.iter().map(|&r| r as u64).product::<u64>();

    let storage = |label: &str, contents: &[u8]| {
        // Empty buffers cannot be bound: keep at least one element.
        let contents = if contents.is_empty() { &[0u8; 64][..] } else { contents };
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents, usage: wgpu::BufferUsages::STORAGE })
    };
    let albedo = storage("albedo", bytemuck::cast_slice(input.albedo));
    let lights: Vec<GpuLight> = input.lights.iter().map(gpu_light).collect();
    let (light_grid, cells, light_indices) = sort_lights(input.lights);
    let lights = storage("lights", bytemuck::cast_slice(&lights));
    let cells = storage("light cells", bytemuck::cast_slice(&cells));
    let light_indices = storage("light indices", bytemuck::cast_slice(&light_indices));
    let probes_size = probe_count * size_of::<Probe>() as u64;
    let probe_buffers: [wgpu::Buffer; 2] = std::array::from_fn(|_| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("probes"),
            size: probes_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    });

    let module = shader::load(device, "probe_bake", spirv!("probe_bake"));
    let layout = bind_group_layout(device, false);
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("probe bake"),
        layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&layout)], immediate_size: 0 })),
        module: &module,
        entry_point: Some("probe_bake"),
        compilation_options: Default::default(),
        cache: None,
    });

    let groups_x = probe_count.min(65535) as u32;
    let groups_y = probe_count.div_ceil(65535) as u32;
    let bounces = input.bounces.max(1);
    let params_for = |pass: u32| {
        let params = Params {
            grid_origin: input.grid.origin,
            cell_size: input.grid.cell_size,
            resolution: input.grid.resolution,
            pass_index: pass,
            sun_direction: input.sun.map_or([0.0, 0.0, -1.0], |(d, _)| d.normalize().to_array()),
            has_sun: input.sun.is_some() as u32 as f32,
            sun_color: input.sun.map_or([0.0; 3], |(_, c)| c.to_array()),
            sky_zenith: input.sky.zenith.to_array(),
            sky_horizon: input.sky.horizon.to_array(),
            sky_ground: input.sky.ground.to_array(),
            light_grid_origin: light_grid.0,
            light_grid_cell: LIGHT_CELL,
            light_grid_resolution: light_grid.1,
            _pad0: 0.0,
            _pad1: 0.0,
            _pad2: 0.0,
            _pad3: 0.0,
            _pad4: 0,
        };
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("bake params"), contents: bytemuck::bytes_of(&params), usage: wgpu::BufferUsages::UNIFORM })
    };
    let shared = SharedBindings { scene: &scene, albedo: &albedo, lights: &lights, cells: &cells, light_indices: &light_indices };
    for pass in 0..bounces {
        let params = params_for(pass);
        let (previous, current) = (&probe_buffers[(pass as usize + 1) % 2], &probe_buffers[pass as usize % 2]);
        let mut entries = shared.entries(&params, previous);
        entries.push(wgpu::BindGroupEntry { binding: 9, resource: current.as_entire_binding() });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("probe bake"), layout: &layout, entries: &entries });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("probe bake") });
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(groups_x, groups_y, 1);
        }
        queue.submit([encoder.finish()]);
    }

    let finished = &probe_buffers[(bounces as usize - 1) % 2];
    let probes = read_back(device, queue, finished, probes_size)?;

    let mut reflections = Vec::new();
    if !input.reflections.is_empty() {
        let size = input.reflection_size;
        let count = input.reflections.len() as u32;
        let texels_size = u64::from(count) * 6 * u64::from(size * size) * 16;
        let texels = device.create_buffer(&wgpu::BufferDescriptor { label: Some("reflection texels"), size: texels_size, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let positions: Vec<[f32; 4]> = input.reflections.iter().map(|p| p.extend(0.0).to_array()).collect();
        let positions = storage("reflection positions", bytemuck::cast_slice(&positions));
        let reflection_params = ReflectionParams { size, count, _pad: [0; 2] };
        let reflection_params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::bytes_of(&reflection_params), usage: wgpu::BufferUsages::UNIFORM });
        let sky = sky_texture(device, queue, input.sky_cubemap);
        let sky_sampler = device.create_sampler(&wgpu::SamplerDescriptor { mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        let layout = bind_group_layout(device, true);
        let module = shader::load(device, "reflection_bake", spirv!("reflection_bake"));
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("reflection bake"),
            layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&layout)], immediate_size: 0 })),
            module: &module,
            entry_point: Some("reflection_bake"),
            compilation_options: Default::default(),
            cache: None,
        });
        // Any pass but the first reads the probes.
        let params = params_for(bounces);
        let mut entries = shared.entries(&params, finished);
        entries.extend([
            wgpu::BindGroupEntry { binding: 9, resource: texels.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 10, resource: positions.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(&sky) },
            wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::Sampler(&sky_sampler) },
            wgpu::BindGroupEntry { binding: 13, resource: reflection_params.as_entire_binding() },
        ]);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("reflection bake"), layout: &layout, entries: &entries });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("reflection bake") });
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(size.div_ceil(8), size.div_ceil(8), count * 6);
        }
        queue.submit([encoder.finish()]);
        reflections = read_back(device, queue, &texels, texels_size)?;
    }
    Ok(Baked { probes, reflections })
}

/// The bindings both bake programs share (0-8).
struct SharedBindings<'a> {
    scene: &'a RayScene,
    albedo: &'a wgpu::Buffer,
    lights: &'a wgpu::Buffer,
    cells: &'a wgpu::Buffer,
    light_indices: &'a wgpu::Buffer,
}

impl<'a> SharedBindings<'a> {
    fn entries(&self, params: &'a wgpu::Buffer, probes: &'a wgpu::Buffer) -> Vec<wgpu::BindGroupEntry<'a>> {
        vec![
            wgpu::BindGroupEntry { binding: 0, resource: self.scene.tlas.as_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: self.scene.positions.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: self.scene.indices.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: self.albedo.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 4, resource: self.lights.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 5, resource: self.cells.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 6, resource: self.light_indices.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 7, resource: params.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 8, resource: probes.as_entire_binding() },
        ]
    }
}

/// Copies a GPU buffer back to memory.
fn read_back<T: Pod>(device: &wgpu::Device, queue: &wgpu::Queue, buffer: &wgpu::Buffer, size: u64) -> Result<Vec<T>, RenderError> {
    let readback = device.create_buffer(&wgpu::BufferDescriptor { label: Some("bake readback"), size, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, 0, &readback, 0, size);
    queue.submit([encoder.finish()]);
    let (sender, receiver) = std::sync::mpsc::channel();
    readback.map_async(wgpu::MapMode::Read, .., move |r| {
        let _ = sender.send(r);
    });
    device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| RenderError::Readback(e.to_string()))?;
    receiver.recv().map_err(|e| RenderError::Readback(e.to_string()))?.map_err(|e| RenderError::Readback(e.to_string()))?;
    let data = bytemuck::cast_slice(&readback.get_mapped_range(..)).to_vec();
    Ok(data)
}

/// The sky cubemap for reflection rays, or a black one.
fn sky_texture(device: &wgpu::Device, queue: &wgpu::Queue, cubemap: Option<&refract_asset::TextureData>) -> wgpu::TextureView {
    let black = [0u8; 6 * 8];
    let (size, data): (u32, &[u8]) = match cubemap {
        Some(c) if c.faces == 6 && c.format == refract_asset::TextureFormat::Rgba16Float => (c.width, &c.mips[0]),
        _ => (1, &black),
    };
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("bake sky"),
            size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 6 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        data,
    );
    texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::Cube), ..Default::default() })
}

fn gpu_light(light: &Light) -> GpuLight {
    let spot = light.kind == LIGHT_SPOT;
    GpuLight {
        position: light.position,
        range: light.range,
        color: (Vec3::from(light.color) * light.intensity).to_array(),
        kind: light.kind,
        direction: light.direction,
        cos_outer: if spot { light.outer_angle.to_radians().cos() } else { -1.0 },
        cos_inner: if spot { light.inner_angle.to_radians().cos() } else { -1.0 },
        clearance: light.clearance,
        _pad: [0.0; 2],
    }
}

/// (origin, resolution) of the light grid, per cell (first, count), and the listed indices.
type LightGrid = (([f32; 3], [u32; 3]), Vec<[u32; 2]>, Vec<u32>);

/// A grid of cells listing the lights whose range reaches into them:
/// (origin, resolution), per cell (first, count), and the listed indices.
fn sort_lights(lights: &[Light]) -> LightGrid {
    if lights.is_empty() {
        return (([0.0; 3], [1, 1, 1]), vec![[0, 0]], Vec::new());
    }
    let (min, max) = lights.iter().fold((Vec3::MAX, Vec3::MIN), |(min, max), l| {
        let p = Vec3::from(l.position);
        (min.min(p - l.range), max.max(p + l.range))
    });
    let resolution = ((max - min) / LIGHT_CELL).ceil().as_uvec3().max(glam::UVec3::ONE);
    let mut lists: Vec<Vec<u32>> = vec![Vec::new(); (resolution.x * resolution.y * resolution.z) as usize];
    for (index, light) in lights.iter().enumerate() {
        let p = Vec3::from(light.position);
        let low = ((p - light.range - min) / LIGHT_CELL).floor().as_ivec3().max(glam::IVec3::ZERO);
        let high = ((p + light.range - min) / LIGHT_CELL).floor().as_ivec3().min(resolution.as_ivec3() - 1);
        for z in low.z..=high.z {
            for y in low.y..=high.y {
                for x in low.x..=high.x {
                    lists[(x as u32 + resolution.x * (y as u32 + resolution.y * z as u32)) as usize].push(index as u32);
                }
            }
        }
    }
    let mut cells = Vec::with_capacity(lists.len());
    let mut indices = Vec::new();
    for list in lists {
        cells.push([indices.len() as u32, list.len() as u32]);
        indices.extend(list);
    }
    ((min.to_array(), resolution.to_array()), cells, indices)
}

/// The bake programs' bindings: 0-8 shared, then the probe outputs or, for
/// `reflections`, the cubemap texels, positions, sky and parameters.
fn bind_group_layout(device: &wgpu::Device, reflections: bool) -> wgpu::BindGroupLayout {
    let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    };
    let uniform = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    };
    let mut entries = vec![
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::AccelerationStructure { vertex_return: false }, count: None },
            storage(1, true),
            storage(2, true),
            storage(3, true),
            storage(4, true),
            storage(5, true),
            storage(6, true),
        uniform(7),
        storage(8, true),
        storage(9, false),
    ];
    if reflections {
        entries.extend([
            storage(10, true),
            wgpu::BindGroupLayoutEntry {
                binding: 11,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::Cube, multisampled: false },
                count: None,
            },
            wgpu::BindGroupLayoutEntry { binding: 12, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            uniform(13),
        ]);
    }
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("bake"), entries: &entries })
}

#[cfg(test)]
mod tests {
    use super::*;
    use refract_asset::map::probe_flags;

    /// A closed room lit by a light, with a solid block in it: probes in
    /// the room are lit on all sides (bounced light reaches the faces away
    /// from the light), the probe inside the block is not used.
    #[test]
    fn light_bounces_in_a_room() {
        if !available() {
            eprintln!("skipped, no ray tracing GPU");
            return;
        }
        // The room faces inwards: its triangles are flipped.
        let room = crate::shapes::cuboid(Vec3::splat(600.0));
        let block = crate::shapes::cuboid(Vec3::splat(100.0));
        let mut positions = room.positions.clone();
        let mut indices: Vec<u32> = room.indices.chunks_exact(3).flat_map(|t| [t[0], t[2], t[1]]).collect();
        let base = positions.len() as u32;
        positions.extend(block.positions.iter().map(|p| [p[0] + 256.0, p[1] + 256.0, p[2]]));
        indices.extend(block.indices.iter().map(|i| i + base));
        let albedo = vec![[0.8, 0.8, 0.8, 0.0]; indices.len() / 3];
        let light = Light { kind: 0, position: [0.0, 0.0, 400.0], range: 2048.0, intensity: 50.0, color: [1.0; 3], ..Default::default() };
        // Probe centres at -512, -256, 0, 256, 512 on each axis.
        let grid = ProbeGrid { origin: [-640.0; 3], cell_size: 256.0, resolution: [5, 5, 5], _pad: 0 };
        let black = Sky { zenith: Vec3::ZERO, horizon: Vec3::ZERO, ground: Vec3::ZERO };
        let reflection_at = [Vec3::new(0.0, 0.0, -300.0)];
        let input = BakeInput {
            positions: &positions,
            indices: &indices,
            albedo: &albedo,
            sun: None,
            sky: black,
            lights: &[light],
            grid,
            bounces: 3,
            reflections: &reflection_at,
            reflection_size: 8,
            sky_cubemap: None,
        };
        let baked = bake(&input).unwrap();
        let probes = baked.probes;
        // Looking up from near the floor, the cubemap sees the lit ceiling.
        assert_eq!(baked.reflections.len(), 6 * 8 * 8);
        let up = &baked.reflections[4 * 64..5 * 64];
        assert!(up.iter().all(|t| t[0] > 0.0), "the +Z face sees the lit room");
        let probe = |x: usize, y: usize, z: usize| probes[x + 5 * (y + 5 * z)];
        let decode = |v: u32| [v & 511, (v >> 9) & 511, (v >> 18) & 511].map(|m| m as f32 * 2f32.powi((v >> 27) as i32 - 24));

        let center = probe(2, 2, 2);
        assert_eq!(center.flags & probe_flags::VALID, probe_flags::VALID);
        // Light from below (the lit floor) and from the side facing away
        // from the light's wall, lit by bounces only.
        let (from_below, from_above) = (decode(center.light[5])[0], decode(center.light[4])[0]);
        assert!(from_below > 0.0 && from_above > 0.0, "below {from_below}, above {from_above}");
        assert_eq!(probe(3, 3, 2).flags & probe_flags::VALID, 0, "inside the block");
    }
}
