//! The passes of a frame, in order: shadow maps, light culling, the scene
//! (sky, depth prepass, opaque then blended geometry) into the HDR target,
//! then tonemapping into the output.

pub mod light_cull;
pub mod scene;
pub mod shadow;
pub mod tonemap;

/// A bind group layout entry visible to vertex and fragment shaders.
fn entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty, count: None }
}

fn uniform() -> wgpu::BindingType {
    wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }
}

fn storage() -> wgpu::BindingType {
    wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }
}

fn pipeline_layout(device: &wgpu::Device, label: &str, layout: &wgpu::BindGroupLayout) -> wgpu::PipelineLayout {
    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: &[Some(layout)], immediate_size: 0 })
}
