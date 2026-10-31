//! A scene rays can be traced against on the GPU: one bottom-level
//! acceleration structure over all triangles, in one top-level structure.

use wgpu::util::DeviceExt;

pub struct RayScene {
    pub tlas: wgpu::Tlas,
    /// The triangles, also readable by shaders (hit normals, materials).
    pub positions: wgpu::Buffer,
    pub indices: wgpu::Buffer,
    _blas: wgpu::Blas,
}

impl RayScene {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, positions: &[[f32; 3]], indices: &[u32]) -> Self {
        let usage = wgpu::BufferUsages::BLAS_INPUT | wgpu::BufferUsages::STORAGE;
        let position_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("ray positions"), contents: bytemuck::cast_slice(positions), usage });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("ray indices"), contents: bytemuck::cast_slice(indices), usage });

        let size = wgpu::BlasTriangleGeometrySizeDescriptor {
            vertex_format: wgpu::VertexFormat::Float32x3,
            vertex_count: positions.len() as u32,
            index_format: Some(wgpu::IndexFormat::Uint32),
            index_count: Some(indices.len() as u32),
            flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
        };
        let blas = device.create_blas(
            &wgpu::CreateBlasDescriptor {
                label: Some("scene"),
                flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                update_mode: wgpu::AccelerationStructureUpdateMode::Build,
            },
            wgpu::BlasGeometrySizeDescriptors::Triangles { descriptors: vec![size.clone()] },
        );
        let mut tlas = device.create_tlas(&wgpu::CreateTlasDescriptor {
            label: Some("scene"),
            max_instances: 1,
            flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
            update_mode: wgpu::AccelerationStructureUpdateMode::Build,
        });
        let identity = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        tlas.get_mut_single(0).unwrap().replace(wgpu::TlasInstance::new(&blas, identity, 0, 0xff));

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("build ray scene") });
        encoder.build_acceleration_structures(
            [&wgpu::BlasBuildEntry {
                blas: &blas,
                geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry {
                    size: &size,
                    vertex_buffer: &position_buffer,
                    first_vertex: 0,
                    vertex_stride: 12,
                    index_buffer: Some(&index_buffer),
                    first_index: Some(0),
                    transform_buffer: None,
                    transform_buffer_offset: None,
                }]),
            }],
            [&tlas],
        );
        queue.submit([encoder.finish()]);
        Self { tlas, positions: position_buffer, indices: index_buffer, _blas: blas }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::{Gpu, RAY_QUERY};
    use crate::shader::{self, spirv};

    #[test]
    fn rays_hit_a_quad() {
        let gpu = match Gpu::with_features(Gpu::instance(), None, RAY_QUERY) {
            Ok(gpu) => gpu,
            Err(err) => {
                eprintln!("skipped, no ray tracing GPU: {err}");
                return;
            }
        };
        let (device, queue) = (&gpu.device, &gpu.queue);
        // A 200x200 quad at z = 0.
        let positions = [[-100.0, -100.0, 0.0], [100.0, -100.0, 0.0], [100.0, 100.0, 0.0], [-100.0, 100.0, 0.0]];
        let scene = RayScene::new(device, queue, &positions, &[0, 1, 2, 0, 2, 3]);

        // Down from 50 above: hits at 50. Sideways and up: misses. Down from 10 above, offset: hits at 10.
        let rays: [[f32; 4]; 8] = [[0.0, 0.0, 50.0, 0.0], [0.0, 0.0, -1.0, 0.0], [0.0, 0.0, 50.0, 0.0], [1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 50.0, 0.0], [0.0, 0.0, 1.0, 0.0], [30.0, -40.0, 10.0, 0.0], [0.0, 0.0, -1.0, 0.0]];
        let ray_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&rays), usage: wgpu::BufferUsages::STORAGE });
        let hits = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 16, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let readback = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 16, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });

        let storage = |read_only| wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::AccelerationStructure { vertex_return: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE, ty: storage(true), count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::COMPUTE, ty: storage(false), count: None },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: scene.tlas.as_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: ray_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: hits.as_entire_binding() },
            ],
        });
        let module = shader::load(device, "ray_test", spirv!("ray_test"));
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&layout)], immediate_size: 0 })),
            module: &module,
            entry_point: Some("ray_test"),
            compilation_options: Default::default(),
            cache: None,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&hits, 0, &readback, 0, 16);
        queue.submit([encoder.finish()]);
        readback.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let result: Vec<f32> = bytemuck::cast_slice(&readback.get_mapped_range(..)).to_vec();
        assert_eq!(result, [50.0, -1.0, -1.0, 10.0]);
    }
}
