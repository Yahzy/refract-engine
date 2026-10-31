//! The renderer: owns the GPU, the scene's resources and the frame's passes.

use glam::{Mat4, Vec3};
use refract_core::axes::METERS_PER_UNIT;

use crate::ambient::{ProbeGrid, ProbeVolume};
use crate::buffer::GrowableBuffer;
use crate::culling::{Frustum, Sphere};
use crate::gpu::Gpu;
use crate::gpu_types::{GpuInstance, GpuLight, GpuMaterial, GpuView, NO_REFLECTION};
use crate::material::BlendMode;
use crate::mesh::{MeshArena, MeshPart, VertexAttributes};
use crate::occluders::Occluders;
use crate::pass::light_cull::LightCullPass;
use crate::pass::scene::{Draw, Pass, SceneResources, ScenePass};
use crate::pass::shadow::{ShadowPass, ShadowTarget, ShadowView};
use crate::pass::tonemap::TonemapPass;
use crate::readback::{self, Image};
use crate::shadow::{self, CASCADES, LOCAL_LAYERS, LOCAL_RESOLUTION, SUN_RESOLUTION};
use crate::targets::Targets;
use crate::texture::TextureStore;
use crate::{Camera, Material, MaterialId, MeshData, MeshId, PointLight, RenderError, Sky, SpotLight, Sun, TextureId};

/// Most local lights in a frame: the visible ones that light the camera's
/// surroundings most. Each pixel only loops over those of its cluster.
const MAX_LIGHTS: usize = 4096;
/// Where the light clusters' last depth slice ends, world units.
const CLUSTER_FAR: f32 = 16384.0;

#[derive(Clone, Copy, Debug)]
pub struct RendererConfig {
    /// Waits for the display's refresh before presenting.
    pub vsync: bool,
    /// Uses hardware ray tracing when the GPU has it (shadows of local lights).
    pub ray_tracing: bool,
}

/// What changes from one frame to the next.
pub struct Frame<'a> {
    pub camera: Camera,
    pub sun: Sun,
    pub sky: Sky,
    pub point_lights: &'a [PointLight],
    pub spot_lights: &'a [SpotLight],
    /// Light reaching everything from everywhere, linear RGB.
    pub ambient: Vec3,
    /// Multiplies the HDR image before tonemapping.
    pub exposure: f32,
    /// Seconds, for animated effects.
    pub time: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InstanceId(u32);

enum Output {
    Surface { surface: wgpu::Surface<'static>, config: wgpu::SurfaceConfiguration },
    Offscreen { texture: wgpu::Texture },
}

struct Instance {
    mesh: MeshId,
    gpu: GpuInstance,
    pass: Pass,
    /// World-space bounds, for culling.
    bounds: Sphere,
    /// Drawn in the view (shadow-only geometry is not).
    visible: bool,
    casts_shadows: bool,
    /// Its material has holes where its texture is transparent.
    alpha_tested: bool,
}

pub struct Renderer {
    gpu: Gpu,
    output: Output,
    width: u32,
    height: u32,
    targets: Targets,

    meshes: MeshArena,
    textures: TextureStore,
    probes: ProbeVolume,
    /// The sky cubemap; a black one when the sky is procedural.
    sky: (wgpu::TextureView, bool),
    /// Reflection cubemaps (a cube array) and where each was taken.
    reflections: (wgpu::TextureView, Vec<Vec3>),
    materials: Vec<GpuMaterial>,
    material_passes: Vec<Pass>,
    material_alpha_tested: Vec<bool>,
    materials_dirty: bool,
    instances: Vec<Instance>,
    instances_dirty: bool,

    view_buffer: wgpu::Buffer,
    instance_buffer: GrowableBuffer,
    material_buffer: GrowableBuffer,
    light_buffer: GrowableBuffer,
    local_shadow_views: wgpu::Buffer,

    shadow_pass: ShadowPass,
    light_cull_pass: LightCullPass,
    scene_pass: ScenePass,
    tonemap_pass: TonemapPass,
    /// Bind groups are rebuilt when one of their resources is replaced; these
    /// are the resource generations they were built with.
    scene_bind_groups: Option<([wgpu::BindGroup; 2], [u32; 4])>,
    shadow_bind_group: Option<(wgpu::BindGroup, [u32; 2])>,
    light_cull_bind_group: Option<(wgpu::BindGroup, u32)>,
    tonemap_bind_group: wgpu::BindGroup,
    /// Shadow ray geometry, when the GPU traces rays; rebuilt when the
    /// shadow casters change.
    occluders: Option<Occluders>,
    occluders_dirty: bool,
    /// Changes when textures or the probes change.
    resources_generation: u32,
}

/// What a frame draws, worked out on the CPU before recording.
struct FramePlan {
    /// Per pass, in drawing order.
    draws: [Vec<Draw>; 4],
    shadow_views: Vec<ShadowView>,
}

const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

impl Renderer {
    /// Renders into a window (anything wgpu can make a surface of).
    pub fn with_surface(target: impl Into<wgpu::SurfaceTarget<'static>>, width: u32, height: u32, config: RendererConfig) -> Result<Self, RenderError> {
        let instance = Gpu::instance();
        let surface = instance.create_surface(target)?;
        let gpu = Gpu::new(instance, Some(&surface), config.ray_tracing)?;
        let mut surface_config = surface.get_default_config(&gpu.adapter, width.max(1), height.max(1)).ok_or(RenderError::SurfaceLost)?;
        let caps = surface.get_capabilities(&gpu.adapter);
        // An sRGB format, so that the tonemapper's output is encoded by the hardware.
        surface_config.format = caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(surface_config.format);
        surface_config.present_mode = present_mode(&caps, config.vsync);
        surface_config.desired_maximum_frame_latency = 2;
        surface.configure(&gpu.device, &surface_config);
        let format = surface_config.format;
        Ok(Self::new(gpu, Output::Surface { surface, config: surface_config }, format, width.max(1), height.max(1)))
    }

    /// Renders into an image only, read with [`Renderer::read_image`].
    pub fn offscreen(width: u32, height: u32, config: RendererConfig) -> Result<Self, RenderError> {
        let gpu = Gpu::new(Gpu::instance(), None, config.ray_tracing)?;
        let texture = offscreen_texture(&gpu.device, width, height);
        Ok(Self::new(gpu, Output::Offscreen { texture }, OFFSCREEN_FORMAT, width, height))
    }

    fn new(gpu: Gpu, output: Output, output_format: wgpu::TextureFormat, width: u32, height: u32) -> Self {
        let (device, queue) = (&gpu.device, &gpu.queue);
        let targets = Targets::new(device, width, height);
        let view_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("view"),
            size: size_of::<GpuView>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let local_shadow_views = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("local shadow views"),
            size: LOCAL_LAYERS as u64 * size_of::<Mat4>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let storage = wgpu::BufferUsages::STORAGE;
        let tonemap_pass = TonemapPass::new(device, output_format);
        let scene_pass = ScenePass::new(device, gpu.ray_query);
        if gpu.ray_query {
            log::info!("local light shadows: ray traced");
        }
        let tonemap_bind_group = tonemap_pass.bind_group(device, &view_buffer, &targets.hdr);
        Self {
            meshes: MeshArena::new(device, gpu.ray_query),
            textures: TextureStore::new(device, queue),
            probes: ProbeVolume::none(device),
            sky: (sky_cube(device, queue, None), false),
            reflections: (reflection_cubes(device, queue, None), Vec::new()),
            instance_buffer: GrowableBuffer::new(device, "instances", storage, 1024 * size_of::<GpuInstance>() as u64),
            material_buffer: GrowableBuffer::new(device, "materials", storage, 256 * size_of::<GpuMaterial>() as u64),
            light_buffer: GrowableBuffer::new(device, "lights", storage, MAX_LIGHTS as u64 * size_of::<GpuLight>() as u64),
            materials: Vec::new(),
            material_passes: Vec::new(),
            material_alpha_tested: Vec::new(),
            materials_dirty: false,
            instances: Vec::new(),
            instances_dirty: false,
            view_buffer,
            local_shadow_views,
            shadow_pass: ShadowPass::new(device, scene_pass.textures_layout()),
            light_cull_pass: LightCullPass::new(device),
            scene_pass,
            tonemap_pass,
            scene_bind_groups: None,
            shadow_bind_group: None,
            light_cull_bind_group: None,
            tonemap_bind_group,
            occluders: gpu.ray_query.then(|| Occluders::new(device)),
            occluders_dirty: true,
            resources_generation: 0,
            output,
            width,
            height,
            targets,
            gpu,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));
        if (width, height) == (self.width, self.height) {
            return;
        }
        (self.width, self.height) = (width, height);
        let device = &self.gpu.device;
        match &mut self.output {
            Output::Surface { surface, config } => {
                (config.width, config.height) = (width, height);
                surface.configure(device, config);
            }
            Output::Offscreen { texture } => *texture = offscreen_texture(device, width, height),
        }
        self.targets = Targets::new(device, width, height);
        self.tonemap_bind_group = self.tonemap_pass.bind_group(device, &self.view_buffer, &self.targets.hdr);
    }

    pub fn add_texture(&mut self, texture: &refract_asset::TextureData, label: &str) -> TextureId {
        self.resources_generation += 1;
        self.textures.add(&self.gpu.device, &self.gpu.queue, texture, label)
    }

    /// Replaces the baked indirect light; `None` lights with the sky instead.
    pub fn set_probes(&mut self, grid: Option<&ProbeGrid>) {
        let device = &self.gpu.device;
        self.probes = grid.map_or_else(|| ProbeVolume::none(device), |grid| ProbeVolume::upload(device, grid));
        self.resources_generation += 1;
    }

    /// The sky's cubemap (faces +X, -X, +Y, -Y, +Z, -Z of world directions);
    /// `None` for the procedural sky.
    pub fn set_sky(&mut self, cubemap: Option<&refract_asset::TextureData>) {
        self.sky = (sky_cube(&self.gpu.device, &self.gpu.queue, cubemap), cubemap.is_some());
        self.resources_generation += 1;
    }

    /// Reflection cubemaps (a cube array, `Rgba16Float`) and where each was
    /// taken; every instance reflects the one nearest to it.
    pub fn set_reflections(&mut self, cubemaps: &refract_asset::TextureData, positions: &[Vec3]) {
        self.reflections = (reflection_cubes(&self.gpu.device, &self.gpu.queue, Some(cubemaps)), positions.to_vec());
        for instance in &mut self.instances {
            instance.gpu.reflection = nearest(positions, instance.bounds.center);
        }
        self.instances_dirty = true;
        self.resources_generation += 1;
    }

    pub fn add_mesh(&mut self, mesh: &MeshData) -> MeshId {
        self.add_geometry(&mesh.positions, &mesh.attributes, &mesh.indices, &[mesh.whole()])[0]
    }

    /// Uploads one geometry and returns a mesh for each of its parts.
    pub fn add_geometry(&mut self, positions: &[[f32; 3]], attributes: &[VertexAttributes], indices: &[u32], parts: &[MeshPart]) -> Vec<MeshId> {
        self.meshes.add(&self.gpu.device, &self.gpu.queue, positions, attributes, indices, parts)
    }

    pub fn add_material(&mut self, material: &Material) -> MaterialId {
        self.materials.push(material.into());
        self.material_passes.push(match material.blend {
            BlendMode::Opaque | BlendMode::AlphaTest if material.two_sided => Pass::OpaqueTwoSided,
            BlendMode::Opaque | BlendMode::AlphaTest => Pass::Opaque,
            BlendMode::Translucent => Pass::Translucent,
            BlendMode::Additive => Pass::Additive,
        });
        self.material_alpha_tested.push(material.blend == BlendMode::AlphaTest);
        self.materials_dirty = true;
        MaterialId(self.materials.len() as u32 - 1)
    }

    /// Places `mesh` in the world. Transforms may translate, rotate and scale
    /// uniformly.
    pub fn add_instance(&mut self, mesh: MeshId, material: MaterialId, transform: Mat4) -> InstanceId {
        let bounds = self.meshes.range(mesh).bounds.transformed(&transform);
        let reflection = nearest(&self.reflections.1, bounds.center);
        let gpu = GpuInstance { model: transform.to_cols_array_2d(), material: material.0, reflection, _pad: [0; 2] };
        let pass = self.material_passes[material.0 as usize];
        // Blended surfaces let light through.
        let casts_shadows = matches!(pass, Pass::Opaque | Pass::OpaqueTwoSided);
        let alpha_tested = self.material_alpha_tested[material.0 as usize];
        self.instances.push(Instance { mesh, gpu, pass, bounds, visible: true, casts_shadows, alpha_tested });
        self.occluders_dirty |= casts_shadows;
        self.instances_dirty = true;
        InstanceId(self.instances.len() as u32 - 1)
    }

    pub fn set_transform(&mut self, id: InstanceId, transform: Mat4) {
        let instance = &mut self.instances[id.0 as usize];
        self.occluders_dirty |= instance.casts_shadows;
        instance.gpu.model = transform.to_cols_array_2d();
        instance.bounds = self.meshes.range(instance.mesh).bounds.transformed(&transform);
        self.instances_dirty = true;
    }

    /// The reflection cubemap the instance mirrors (an index into those given
    /// to [`Renderer::set_reflections`]), instead of the nearest.
    pub fn set_reflection(&mut self, id: InstanceId, cubemap: u32) {
        self.instances[id.0 as usize].gpu.reflection = cubemap;
        self.instances_dirty = true;
    }

    /// Whether the instance appears in shadow maps. On by default for opaque
    /// materials; turn it off for things inside a light, such as its bulb.
    pub fn set_casts_shadows(&mut self, id: InstanceId, casts_shadows: bool) {
        self.instances[id.0 as usize].casts_shadows = casts_shadows;
        self.occluders_dirty = true;
    }

    /// Whether the instance is drawn in the view. Hidden instances may still
    /// cast shadows.
    pub fn set_visible(&mut self, id: InstanceId, visible: bool) {
        self.instances[id.0 as usize].visible = visible;
    }

    pub fn render(&mut self, frame: &Frame) -> Result<(), RenderError> {
        let plan = self.prepare(frame);

        let surface_texture = match &self.output {
            Output::Surface { surface, config } => match surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(texture) => Some(texture),
                wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                    surface.configure(&self.gpu.device, config);
                    Some(texture)
                }
                wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                    surface.configure(&self.gpu.device, config);
                    return Ok(());
                }
                // Nothing to show: skip the frame.
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return Ok(()),
                wgpu::CurrentSurfaceTexture::Validation => return Err(RenderError::SurfaceLost),
            },
            Output::Offscreen { .. } => None,
        };
        let output_view = match (&surface_texture, &self.output) {
            (Some(texture), _) => texture.texture.create_view(&Default::default()),
            (None, Output::Offscreen { texture }) => texture.create_view(&Default::default()),
            (None, Output::Surface { .. }) => unreachable!(),
        };

        let (Some((scene_bind_groups, _)), Some((shadow_bind_group, _)), Some((light_cull_bind_group, _))) = (&self.scene_bind_groups, &self.shadow_bind_group, &self.light_cull_bind_group) else {
            unreachable!("built in prepare")
        };
        let mut encoder = self.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        self.shadow_pass.record(&self.gpu.queue, &mut encoder, shadow_bind_group, &scene_bind_groups[1], &self.meshes, &plan.shadow_views);
        self.light_cull_pass.record(&mut encoder, light_cull_bind_group);
        self.scene_pass.record(&mut encoder, &self.targets, scene_bind_groups, &self.meshes, &plan.draws);
        self.tonemap_pass.record(&mut encoder, &output_view, &self.tonemap_bind_group);
        self.gpu.queue.submit([encoder.finish()]);
        if let Some(texture) = surface_texture {
            texture.present();
        }
        Ok(())
    }

    /// Blocks until the GPU has finished all submitted work.
    pub fn wait_idle(&self) {
        if let Err(err) = self.gpu.device.poll(wgpu::PollType::wait_indefinitely()) {
            log::warn!("waiting for the GPU: {err}");
        }
    }

    /// The last rendered image, for an offscreen renderer.
    pub fn read_image(&self) -> Result<Image, RenderError> {
        match &self.output {
            Output::Offscreen { texture } => readback::read_rgba8(&self.gpu.device, &self.gpu.queue, texture),
            Output::Surface { .. } => Err(RenderError::Readback("only offscreen renderers can be read".into())),
        }
    }

    /// Uploads what changed, decides what each view draws, and writes the
    /// frame's constants.
    fn prepare(&mut self, frame: &Frame) -> FramePlan {
        let (device, queue) = (&self.gpu.device, &self.gpu.queue);
        if self.materials_dirty {
            self.material_buffer.replace(device, queue, bytemuck::cast_slice(&self.materials));
            self.materials_dirty = false;
        }
        if self.instances_dirty {
            let gpu: Vec<GpuInstance> = self.instances.iter().map(|i| i.gpu).collect();
            self.instance_buffer.replace(device, queue, bytemuck::cast_slice(&gpu));
            self.instances_dirty = false;
        }

        if self.occluders_dirty
            && let Some(occluders) = &mut self.occluders
        {
            // Opaque casters block light; alpha-tested ones (foliage) let it through their holes.
            let casters: Vec<(MeshId, Mat4)> = self
                .instances
                .iter()
                .filter(|i| i.casts_shadows && !i.alpha_tested)
                .map(|i| (i.mesh, Mat4::from_cols_array_2d(&i.gpu.model)))
                .collect();
            occluders.rebuild(device, queue, &self.meshes, &casters);
            self.resources_generation += 1;
        }
        self.occluders_dirty = false;

        let camera = &frame.camera;
        let aspect = self.width as f32 / self.height as f32;
        let view_proj = camera.projection(aspect) * camera.view();
        let view_frustum = Frustum::from_matrix(&view_proj);

        // The visible local lights that light the surroundings of the camera
        // most, and shadow maps for the nearest of those that cast shadows.
        let mut lights: Vec<(GpuLight, bool)> = frame.point_lights.iter().map(|l| (GpuLight::from(l), l.casts_shadows)).collect();
        lights.extend(frame.spot_lights.iter().map(|l| (GpuLight::from(l), l.casts_shadows)));
        lights.retain(|(light, _)| view_frustum.intersects(&Sphere { center: Vec3::from(light.position), radius: light.range }));
        let weight = |light: &GpuLight| -> f32 {
            let meters = (camera.position.distance(Vec3::from(light.position)) * METERS_PER_UNIT).max(1.0);
            Vec3::from(light.color).max_element() / (meters * meters)
        };
        lights.sort_by(|a, b| weight(&b.0).total_cmp(&weight(&a.0)));
        lights.truncate(MAX_LIGHTS);
        let casts: Vec<bool> = lights.iter().map(|(_, casts)| *casts).collect();
        let mut lights: Vec<GpuLight> = lights.into_iter().map(|(light, _)| light).collect();
        let local_views = shadow::allocate_local_shadows(&mut lights, &casts, camera.position, &view_frustum);
        self.light_buffer.replace(device, queue, bytemuck::cast_slice(&lights));
        if !local_views.is_empty() {
            queue.write_buffer(&self.local_shadow_views, 0, bytemuck::cast_slice(&local_views));
        }

        let towards_sun = -frame.sun.direction.normalize();
        let cascades = shadow::sun_cascades(camera, aspect, towards_sun);
        let mut shadow_views = Vec::with_capacity(CASCADES + local_views.len());
        for (layer, cascade) in cascades.iter().enumerate() {
            let (draws, alpha_draws) = self.shadow_casters(&cascade.view_proj);
            shadow_views.push(ShadowView { target: ShadowTarget::Sun(layer as u32), view_proj: cascade.view_proj, draws, alpha_draws });
        }
        for (layer, view_proj) in local_views.iter().enumerate() {
            let (draws, alpha_draws) = self.shadow_casters(view_proj);
            shadow_views.push(ShadowView { target: ShadowTarget::Local(layer as u32), view_proj: *view_proj, draws, alpha_draws });
        }
        let plan = FramePlan { draws: self.visible(&view_frustum), shadow_views };

        let generations = [self.instance_buffer.generation(), self.material_buffer.generation(), self.light_buffer.generation(), self.resources_generation];
        self.bind(generations);
        let view = GpuView {
            view_proj: view_proj.to_cols_array_2d(),
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            cascades: cascades.each_ref().map(|c| c.view_proj.to_cols_array_2d()),
            cascade_far: cascades.each_ref().map(|c| c.far),
            cascade_texel: cascades.each_ref().map(|c| c.texel_size),
            position: camera.position.into(),
            exposure: frame.exposure,
            forward: camera.angles.forward().into(),
            light_count: lights.len() as u32,
            sun_direction: towards_sun.into(),
            local_shadow_texel: 1.0 / LOCAL_RESOLUTION as f32,
            sun_color: (frame.sun.color * frame.sun.intensity).into(),
            time: frame.time,
            sky_color: frame.sky.zenith.into(),
            sun_shadow_texel: 1.0 / SUN_RESOLUTION as f32,
            horizon_color: frame.sky.horizon.into(),
            _pad0: 0.0,
            ground_color: frame.sky.ground.into(),
            _pad1: 0.0,
            probe_origin: self.probes.origin.into(),
            has_probes: self.probes.present as u32,
            probe_resolution: self.probes.resolution,
            probe_cell: self.probes.cell_size,
            flat_ambient: frame.ambient.into(),
            has_sky: self.sky.1 as u32,
            screen_size: [self.width as f32, self.height as f32],
            cluster_near: camera.near,
            cluster_far: CLUSTER_FAR,
        };
        self.gpu.queue.write_buffer(&self.view_buffer, 0, bytemuck::bytes_of(&view));
        plan
    }

    /// The visible instances the view sees, per pass; alpha-tested ones last,
    /// so the depth prepass switches pipelines once.
    fn visible(&self, frustum: &Frustum) -> [Vec<Draw>; 4] {
        let mut draws: [Vec<Draw>; 4] = Default::default();
        for (index, instance) in self.instances.iter().enumerate() {
            if instance.visible && frustum.intersects(&instance.bounds) {
                draws[instance.pass as usize].push(self.draw(index, instance));
            }
        }
        for list in &mut draws {
            list.sort_by_key(|d| d.alpha_tested);
        }
        draws
    }

    /// The shadow casters a shadow view sees: solid ones, and alpha-tested ones.
    fn shadow_casters(&self, view_proj: &Mat4) -> (Vec<Draw>, Vec<Draw>) {
        let frustum = Frustum::from_matrix(view_proj);
        let (mut solid, mut alpha) = (Vec::new(), Vec::new());
        for (index, instance) in self.instances.iter().enumerate() {
            if instance.casts_shadows && frustum.intersects(&instance.bounds) {
                let list = if instance.alpha_tested { &mut alpha } else { &mut solid };
                list.push(self.draw(index, instance));
            }
        }
        (solid, alpha)
    }

    fn draw(&self, index: usize, instance: &Instance) -> Draw {
        let range = self.meshes.range(instance.mesh);
        Draw { first_index: range.first_index, index_count: range.index_count, base_vertex: range.base_vertex, instance: index as u32, alpha_tested: instance.alpha_tested }
    }

    /// (Re)builds the bind groups whose resources were replaced.
    fn bind(&mut self, generations: [u32; 4]) {
        let device = &self.gpu.device;
        if self.scene_bind_groups.as_ref().is_none_or(|(_, bound)| *bound != generations) {
            let resources = SceneResources {
                view: &self.view_buffer,
                instances: self.instance_buffer.buffer(),
                materials: self.material_buffer.buffer(),
                lights: self.light_buffer.buffer(),
                shadow_maps: &self.shadow_pass.maps,
                local_shadow_views: &self.local_shadow_views,
                textures: &self.textures,
                probes: &self.probes,
                cluster_counts: &self.light_cull_pass.counts,
                cluster_lights: &self.light_cull_pass.lights,
                sky: &self.sky.0,
                reflections: &self.reflections.0,
                occluders: self.occluders.as_ref().map(|o| &o.tlas),
            };
            self.scene_bind_groups = Some((self.scene_pass.bind_groups(device, &resources), generations));
        }
        let shadow_generations = [self.instance_buffer.generation(), self.material_buffer.generation()];
        if self.shadow_bind_group.as_ref().is_none_or(|(_, bound)| *bound != shadow_generations) {
            let bind_group = self.shadow_pass.bind_group(device, self.instance_buffer.buffer(), self.material_buffer.buffer());
            self.shadow_bind_group = Some((bind_group, shadow_generations));
        }
        let lights = self.light_buffer.generation();
        if self.light_cull_bind_group.as_ref().is_none_or(|(_, bound)| *bound != lights) {
            self.light_cull_bind_group = Some((self.light_cull_pass.bind_group(device, &self.view_buffer, self.light_buffer.buffer()), lights));
        }
    }
}

/// Index of the position nearest to `point`, or [`NO_REFLECTION`].
fn nearest(positions: &[Vec3], point: Vec3) -> u32 {
    positions.iter().enumerate().min_by(|a, b| a.1.distance_squared(point).total_cmp(&b.1.distance_squared(point))).map_or(NO_REFLECTION, |(i, _)| i as u32)
}

/// A cube array holding `cubemaps` with their mips, or one black cubemap.
fn reflection_cubes(device: &wgpu::Device, queue: &wgpu::Queue, cubemaps: Option<&refract_asset::TextureData>) -> wgpu::TextureView {
    use wgpu::util::DeviceExt;
    let black = [0u8; 6 * 8];
    let (size, layers, mips, data): (u32, u32, u32, Vec<u8>) = match cubemaps {
        Some(c) if c.faces == 6 && c.format == refract_asset::TextureFormat::Rgba16Float => {
            (c.width, c.layers, c.mips.len() as u32, c.mips.iter().flat_map(|m| m.iter().copied()).collect())
        }
        _ => (1, 1, 1, black.to_vec()),
    };
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("reflections"),
            size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: layers * 6 },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        // KTX2 stores all layers of a mip, then the next mip.
        wgpu::util::TextureDataOrder::MipMajor,
        &data,
    );
    texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::CubeArray), ..Default::default() })
}

/// A cube texture holding `cubemap`, or a black one.
fn sky_cube(device: &wgpu::Device, queue: &wgpu::Queue, cubemap: Option<&refract_asset::TextureData>) -> wgpu::TextureView {
    use wgpu::util::DeviceExt;
    let black = [0u8; 6 * 8];
    let (size, data): (u32, &[u8]) = match cubemap {
        Some(c) if c.faces == 6 && c.format == refract_asset::TextureFormat::Rgba16Float => (c.width, &c.mips[0]),
        _ => (1, &black),
    };
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("sky"),
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

fn present_mode(caps: &wgpu::SurfaceCapabilities, vsync: bool) -> wgpu::PresentMode {
    let preferred: &[wgpu::PresentMode] = if vsync {
        &[wgpu::PresentMode::Fifo]
    } else {
        &[wgpu::PresentMode::Immediate, wgpu::PresentMode::Mailbox, wgpu::PresentMode::Fifo]
    };
    preferred.iter().copied().find(|mode| caps.present_modes.contains(mode)).unwrap_or(wgpu::PresentMode::Fifo)
}

fn offscreen_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: OFFSCREEN_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}
