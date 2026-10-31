//! Light culling: a compute pass sorting the frame's lights into clusters
//! (screen tiles times depth slices), before the scene pass shades with them.

use crate::gpu_types::{CLUSTER_MAX_LIGHTS, CLUSTERS};
use crate::shader::{self, spirv};

pub struct LightCullPass {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    /// Per cluster, how many lights it lists.
    pub counts: wgpu::Buffer,
    /// Per cluster, `CLUSTER_MAX_LIGHTS` light indices.
    pub lights: wgpu::Buffer,
}

impl LightCullPass {
    pub fn new(device: &wgpu::Device) -> Self {
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty, count: None };
        let buffer = |ty| wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("light cull"),
            entries: &[
                entry(0, buffer(wgpu::BufferBindingType::Uniform)),
                entry(1, buffer(wgpu::BufferBindingType::Storage { read_only: true })),
                entry(2, buffer(wgpu::BufferBindingType::Storage { read_only: false })),
                entry(3, buffer(wgpu::BufferBindingType::Storage { read_only: false })),
            ],
        });
        let module = shader::load(device, "light_cull", spirv!("light_cull"));
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("light cull"),
            layout: Some(&super::pipeline_layout(device, "light cull", &layout)),
            module: &module,
            entry_point: Some("light_cull"),
            compilation_options: Default::default(),
            cache: None,
        });
        let buffer = |label, size: u32| {
            device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: u64::from(size) * 4, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false })
        };
        Self { layout, pipeline, counts: buffer("cluster counts", CLUSTERS), lights: buffer("cluster lights", CLUSTERS * CLUSTER_MAX_LIGHTS) }
    }

    pub fn bind_group(&self, device: &wgpu::Device, view: &wgpu::Buffer, lights: &wgpu::Buffer) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("light cull"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: view.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: lights.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.counts.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: self.lights.as_entire_binding() },
            ],
        })
    }

    pub fn record(&self, encoder: &mut wgpu::CommandEncoder, bind_group: &wgpu::BindGroup) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("light cull"), timestamp_writes: None });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.dispatch_workgroups(CLUSTERS.div_ceil(64), 1, 1);
    }
}
