//! The scene's opaque shadow casters as acceleration structures, for shadow
//! rays: one bottom-level structure per mesh (built once), one top-level
//! structure placing every caster (rebuilt when casters change).

use std::collections::HashMap;

use glam::Mat4;

use crate::mesh::{MeshArena, MeshId};

pub struct Occluders {
    blas: HashMap<MeshId, (wgpu::Blas, wgpu::BlasTriangleGeometrySizeDescriptor)>,
    pub tlas: wgpu::Tlas,
}

impl Occluders {
    pub fn new(device: &wgpu::Device) -> Self {
        Self { blas: HashMap::new(), tlas: tlas(device, 1) }
    }

    /// Places `casters` (meshes and transforms), building the meshes not seen yet.
    pub fn rebuild(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, meshes: &MeshArena, casters: &[(MeshId, Mat4)]) {
        let mut new = Vec::new();
        for &(mesh, _) in casters {
            if self.blas.contains_key(&mesh) {
                continue;
            }
            let range = meshes.range(mesh);
            let size = wgpu::BlasTriangleGeometrySizeDescriptor {
                vertex_format: wgpu::VertexFormat::Float32x3,
                vertex_count: range.vertex_count,
                index_format: Some(wgpu::IndexFormat::Uint32),
                index_count: Some(range.index_count),
                flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
            };
            let blas = device.create_blas(
                &wgpu::CreateBlasDescriptor { label: Some("occluder"), flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE, update_mode: wgpu::AccelerationStructureUpdateMode::Build },
                wgpu::BlasGeometrySizeDescriptors::Triangles { descriptors: vec![size.clone()] },
            );
            self.blas.insert(mesh, (blas, size));
            new.push(mesh);
        }

        self.tlas = tlas(device, casters.len().max(1) as u32);
        for (index, &(mesh, transform)) in casters.iter().enumerate() {
            let rows = transform.transpose().to_cols_array();
            let instance = wgpu::TlasInstance::new(&self.blas[&mesh].0, rows[..12].try_into().unwrap(), 0, 0xff);
            self.tlas.get_mut_single(index).unwrap().replace(instance);
        }

        let entries: Vec<wgpu::BlasBuildEntry> = new
            .iter()
            .map(|mesh| {
                let range = meshes.range(*mesh);
                let (blas, size) = &self.blas[mesh];
                wgpu::BlasBuildEntry {
                    blas,
                    geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry {
                        size,
                        vertex_buffer: meshes.positions.buffer(),
                        first_vertex: range.base_vertex as u32,
                        vertex_stride: 12,
                        index_buffer: Some(meshes.indices.buffer()),
                        first_index: Some(range.first_index),
                        transform_buffer: None,
                        transform_buffer_offset: None,
                    }]),
                }
            })
            .collect();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("occluders") });
        encoder.build_acceleration_structures(&entries, [&self.tlas]);
        queue.submit([encoder.finish()]);
    }
}

fn tlas(device: &wgpu::Device, max_instances: u32) -> wgpu::Tlas {
    device.create_tlas(&wgpu::CreateTlasDescriptor {
        label: Some("occluders"),
        max_instances,
        flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
        update_mode: wgpu::AccelerationStructureUpdateMode::Build,
    })
}
