//! The shadow pass: renders the depth of shadow casters into each shadow map
//! layer, one render pass per layer. Alpha-tested casters have a pipeline of
//! their own that reads their texture, so light passes through their holes.

use glam::Mat4;

use super::pipeline_layout;
use super::scene::Draw;
use crate::mesh::{ATTRIBUTE_LAYOUT, MeshArena, POSITION_LAYOUT};
use crate::shader::{self, spirv};
use crate::shadow::{CASCADES, LOCAL_LAYERS, LOCAL_RESOLUTION, SUN_RESOLUTION};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Each shadow view's matrix sits at a multiple of this offset in the views buffer.
const VIEW_STRIDE: u64 = 256;
const MAX_VIEWS: u64 = CASCADES as u64 + LOCAL_LAYERS as u64;

/// Which map a shadow view renders into.
#[derive(Clone, Copy)]
pub enum ShadowTarget {
    Sun(u32),
    Local(u32),
}

/// A shadow view to render this frame: its matrix and the casters it sees.
pub struct ShadowView {
    pub target: ShadowTarget,
    pub view_proj: Mat4,
    pub draws: Vec<Draw>,
    /// Casters whose texture has holes.
    pub alpha_draws: Vec<Draw>,
}

/// One layer per sun cascade, and one shared array for local lights.
pub struct ShadowMaps {
    pub sun: wgpu::TextureView,
    pub local: wgpu::TextureView,
    pub compare: wgpu::Sampler,
    sun_layers: Vec<wgpu::TextureView>,
    local_layers: Vec<wgpu::TextureView>,
}

pub struct ShadowPass {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    alpha_pipeline: wgpu::RenderPipeline,
    views: wgpu::Buffer,
    pub maps: ShadowMaps,
}

impl ShadowPass {
    /// `textures_layout` is the scene pass's group of textures, bound as is.
    pub fn new(device: &wgpu::Device, textures_layout: &wgpu::BindGroupLayout) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX, ty: super::storage(), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: wgpu::BufferSize::new(64) },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: super::storage(), count: None },
            ],
        });
        let module = shader::load(device, "shadow", spirv!("shadow"));
        let depth = Some(wgpu::DepthStencilState {
            format: FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: Default::default(),
            bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 },
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow"),
            layout: Some(&pipeline_layout(device, "shadow", &layout)),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("shadow_vs"), compilation_options: Default::default(), buffers: &[POSITION_LAYOUT] },
            // Both faces cast: single-sided surfaces (planes, open meshes) must block light too.
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: depth.clone(),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        let alpha_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow alpha"),
            bind_group_layouts: &[Some(&layout), Some(textures_layout)],
            immediate_size: 0,
        });
        let alpha_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow alpha"),
            layout: Some(&alpha_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("shadow_alpha_vs"),
                compilation_options: Default::default(),
                buffers: &[POSITION_LAYOUT, ATTRIBUTE_LAYOUT],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: depth,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("shadow_alpha_fs"), compilation_options: Default::default(), targets: &[] }),
            multiview_mask: None,
            cache: None,
        });
        let views = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow views"),
            size: MAX_VIEWS * VIEW_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self { layout, pipeline, alpha_pipeline, views, maps: ShadowMaps::new(device) }
    }

    pub fn bind_group(&self, device: &wgpu::Device, instances: &wgpu::Buffer, materials: &wgpu::Buffer) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: instances.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &self.views, offset: 0, size: wgpu::BufferSize::new(64) }),
                },
                wgpu::BindGroupEntry { binding: 2, resource: materials.as_entire_binding() },
            ],
        })
    }

    /// `textures` is the scene pass's group of textures.
    pub fn record(&self, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, bind_group: &wgpu::BindGroup, textures: &wgpu::BindGroup, meshes: &MeshArena, views: &[ShadowView]) {
        for (index, view) in views.iter().enumerate() {
            let offset = index as u64 * VIEW_STRIDE;
            queue.write_buffer(&self.views, offset, bytemuck::bytes_of(&view.view_proj.to_cols_array()));
            let target = match view.target {
                ShadowTarget::Sun(layer) => &self.maps.sun_layers[layer as usize],
                ShadowTarget::Local(layer) => &self.maps.local_layers[layer as usize],
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: target,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if view.draws.is_empty() && view.alpha_draws.is_empty() {
                continue;
            }
            pass.set_bind_group(0, bind_group, &[offset as u32]);
            pass.set_bind_group(1, textures, &[]);
            meshes.bind(&mut pass, true);
            for (pipeline, draws) in [(&self.pipeline, &view.draws), (&self.alpha_pipeline, &view.alpha_draws)] {
                if draws.is_empty() {
                    continue;
                }
                pass.set_pipeline(pipeline);
                for draw in draws {
                    pass.draw_indexed(draw.first_index..draw.first_index + draw.index_count, draw.base_vertex, draw.instance..draw.instance + 1);
                }
            }
        }
    }
}

impl ShadowMaps {
    fn new(device: &wgpu::Device) -> Self {
        let (sun_texture, sun_layers) = layered(device, "sun shadows", SUN_RESOLUTION, CASCADES as u32);
        let (local_texture, local_layers) = layered(device, "local shadows", LOCAL_RESOLUTION, LOCAL_LAYERS);
        let array = wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() };
        let compare = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow compare"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        Self { sun: sun_texture.create_view(&array), local: local_texture.create_view(&array), compare, sun_layers, local_layers }
    }
}

/// A depth texture array, and a view of each layer to render into.
fn layered(device: &wgpu::Device, label: &str, resolution: u32, layers: u32) -> (wgpu::Texture, Vec<wgpu::TextureView>) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: resolution, height: resolution, depth_or_array_layers: layers },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let views = (0..layers)
        .map(|layer| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: layer,
                array_layer_count: Some(1),
                ..Default::default()
            })
        })
        .collect();
    (texture, views)
}
