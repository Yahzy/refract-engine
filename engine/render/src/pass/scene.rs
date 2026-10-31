//! The scene pass: the sky over the whole screen, a depth prepass of opaque
//! surfaces, then every visible instance shaded forward into the HDR target:
//! opaque surfaces first (each pixel once, where the prepass left its depth),
//! then blended ones.

use std::num::NonZeroU32;

use super::shadow::ShadowMaps;
use super::{entry, storage, uniform};
use crate::ambient::ProbeVolume;
use crate::mesh::{ATTRIBUTE_LAYOUT, MeshArena, POSITION_LAYOUT};
use crate::shader::{self, spirv};
use crate::targets::{DEPTH_FORMAT, HDR_FORMAT, Targets};
use crate::texture::{MAX_TEXTURES, TextureStore};

/// The pipeline an instance is drawn with, in drawing order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pass {
    Opaque,
    OpaqueTwoSided,
    Translucent,
    Additive,
}

pub const PASSES: [Pass; 4] = [Pass::Opaque, Pass::OpaqueTwoSided, Pass::Translucent, Pass::Additive];

/// One instance to draw: its mesh's index range, and its slot in the instance buffer.
pub struct Draw {
    pub first_index: u32,
    pub index_count: u32,
    pub base_vertex: i32,
    pub instance: u32,
    /// Its material has holes (alpha test): the depth prepass reads its texture.
    pub alpha_tested: bool,
}

pub struct ScenePass {
    layout: wgpu::BindGroupLayout,
    /// Group 1: the arrays of textures.
    textures_layout: wgpu::BindGroupLayout,
    background: wgpu::RenderPipeline,
    /// Indexed like [`PASSES`].
    pipelines: [wgpu::RenderPipeline; 4],
    /// Depth prepass of the two opaque passes: [solid, alpha-tested] each.
    prepass: [[wgpu::RenderPipeline; 2]; 2],
    sky_sampler: wgpu::Sampler,
    reflection_sampler: wgpu::Sampler,
    sampler: wgpu::Sampler,
}

pub struct SceneResources<'a> {
    pub view: &'a wgpu::Buffer,
    pub instances: &'a wgpu::Buffer,
    pub materials: &'a wgpu::Buffer,
    pub lights: &'a wgpu::Buffer,
    pub shadow_maps: &'a ShadowMaps,
    pub local_shadow_views: &'a wgpu::Buffer,
    pub textures: &'a TextureStore,
    pub probes: &'a ProbeVolume,
    pub cluster_counts: &'a wgpu::Buffer,
    pub cluster_lights: &'a wgpu::Buffer,
    pub sky: &'a wgpu::TextureView,
    pub reflections: &'a wgpu::TextureView,
    /// Opaque geometry for shadow rays, when the pass traces them.
    pub occluders: Option<&'a wgpu::Tlas>,
}

impl ScenePass {
    /// `ray_traced_shadows`: local lights cast shadows through rays (needs ray queries).
    pub fn new(device: &wgpu::Device, ray_traced_shadows: bool) -> Self {
        let float_texture = |dimension| wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: dimension, multisampled: false };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene"),
            entries: &[
                entry(0, uniform()),
                entry(1, storage()),
                entry(2, storage()),
                entry(3, storage()),
                entry(4, shadow_maps()),
                entry(5, shadow_maps()),
                entry(6, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison)),
                entry(7, storage()),
                entry(8, storage()),
                entry(9, storage()),
                entry(10, storage()),
                entry(11, float_texture(wgpu::TextureViewDimension::Cube)),
                entry(12, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                entry(13, float_texture(wgpu::TextureViewDimension::CubeArray)),
                entry(14, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
            ][..]
                .iter()
                .cloned()
                .chain(ray_traced_shadows.then(|| entry(15, wgpu::BindingType::AccelerationStructure { vertex_return: false })))
                .collect::<Vec<_>>(),
        });
        // wgpu does not mix binding arrays and uniform buffers in one group.
        let textures_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene textures"),
            entries: &[
                wgpu::BindGroupLayoutEntry { count: NonZeroU32::new(MAX_TEXTURES), ..entry(0, float_texture(wgpu::TextureViewDimension::D2Array)) },
                entry(1, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene"),
            bind_group_layouts: &[Some(&layout), Some(&textures_layout)],
            immediate_size: 0,
        });
        let depth = |write: bool, compare| {
            Some(wgpu::DepthStencilState { format: DEPTH_FORMAT, depth_write_enabled: Some(write), depth_compare: Some(compare), stencil: Default::default(), bias: Default::default() })
        };

        let module = shader::load(device, "background", spirv!("background"));
        let background = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("background"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("background_vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: depth(false, wgpu::CompareFunction::Always),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("background_fs"),
                compilation_options: Default::default(),
                targets: &[Some(HDR_FORMAT.into())],
            }),
            multiview_mask: None,
            cache: None,
        });

        let module = if ray_traced_shadows { shader::load(device, "scene", spirv!("scene_rt")) } else { shader::load(device, "scene", spirv!("scene")) };
        let pipelines = PASSES.map(|pass| {
            // Opaque surfaces only shade where the prepass found them nearest.
            let (cull_mode, blend, compare) = match pass {
                Pass::Opaque => (Some(wgpu::Face::Back), None, wgpu::CompareFunction::Equal),
                Pass::OpaqueTwoSided => (None, None, wgpu::CompareFunction::Equal),
                Pass::Translucent => (None, Some(wgpu::BlendState::ALPHA_BLENDING), wgpu::CompareFunction::GreaterEqual),
                Pass::Additive => (None, Some(ADDITIVE), wgpu::CompareFunction::GreaterEqual),
            };
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("scene"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState { module: &module, entry_point: Some("scene_vs"), compilation_options: Default::default(), buffers: &[POSITION_LAYOUT, ATTRIBUTE_LAYOUT] },
                primitive: wgpu::PrimitiveState { cull_mode, ..Default::default() },
                depth_stencil: depth(false, compare),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("scene_fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: HDR_FORMAT, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        });

        let prepass = [Some(wgpu::Face::Back), None].map(|cull_mode| {
            [false, true].map(|alpha_tested| {
                let (vertex, fragment, buffers): (_, _, &[wgpu::VertexBufferLayout]) =
                    if alpha_tested { ("depth_alpha_vs", "depth_alpha_fs", &[POSITION_LAYOUT, ATTRIBUTE_LAYOUT]) } else { ("depth_vs", "depth_fs", &[POSITION_LAYOUT]) };
                // The colour target is the scene pass's, left untouched.
                let targets = [Some(wgpu::ColorTargetState { format: HDR_FORMAT, blend: None, write_mask: wgpu::ColorWrites::empty() })];
                let fragment = Some(wgpu::FragmentState { module: &module, entry_point: Some(fragment), compilation_options: Default::default(), targets: &targets });
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("depth prepass"),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState { module: &module, entry_point: Some(vertex), compilation_options: Default::default(), buffers },
                    primitive: wgpu::PrimitiveState { cull_mode, ..Default::default() },
                    depth_stencil: depth(true, wgpu::CompareFunction::Greater),
                    multisample: Default::default(),
                    fragment,
                    multiview_mask: None,
                    cache: None,
                })
            })
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("textures"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        });
        let sky_sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("sky"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        let reflection_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("reflections"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        Self { layout, textures_layout, background, pipelines, prepass, sampler, sky_sampler, reflection_sampler }
    }

    /// Layout of the group of textures (group 1), which the shadow pass shares.
    pub fn textures_layout(&self) -> &wgpu::BindGroupLayout {
        &self.textures_layout
    }

    /// The pass's two bind groups.
    pub fn bind_groups(&self, device: &wgpu::Device, resources: &SceneResources) -> [wgpu::BindGroup; 2] {
        let textures = resources.textures.views();
        let mut entries = vec![
                wgpu::BindGroupEntry { binding: 0, resource: resources.view.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: resources.instances.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: resources.materials.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: resources.lights.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&resources.shadow_maps.sun) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&resources.shadow_maps.local) },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::Sampler(&resources.shadow_maps.compare) },
                wgpu::BindGroupEntry { binding: 7, resource: resources.local_shadow_views.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 8, resource: resources.probes.buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 9, resource: resources.cluster_counts.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 10, resource: resources.cluster_lights.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(resources.sky) },
                wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::Sampler(&self.sky_sampler) },
                wgpu::BindGroupEntry { binding: 13, resource: wgpu::BindingResource::TextureView(resources.reflections) },
                wgpu::BindGroupEntry { binding: 14, resource: wgpu::BindingResource::Sampler(&self.reflection_sampler) },
            ];
        if let Some(tlas) = resources.occluders {
            entries.push(wgpu::BindGroupEntry { binding: 15, resource: tlas.as_binding() });
        }
        let scene = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("scene"), layout: &self.layout, entries: &entries });
        let textures = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene textures"),
            layout: &self.textures_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureViewArray(&textures) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        [scene, textures]
    }

    /// `draws` holds the draws of each pass, ordered like [`PASSES`].
    pub fn record(&self, encoder: &mut wgpu::CommandEncoder, targets: &Targets, bind_groups: &[wgpu::BindGroup; 2], meshes: &MeshArena, draws: &[Vec<Draw>; 4]) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scene"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &targets.hdr,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &targets.depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Discard }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &bind_groups[0], &[]);
        pass.set_bind_group(1, &bind_groups[1], &[]);

        pass.set_pipeline(&self.background);
        pass.draw(0..3, 0..1);

        meshes.bind(&mut pass, true);
        for (prepass, draws) in self.prepass.iter().zip(draws) {
            for draw in draws {
                pass.set_pipeline(&prepass[draw.alpha_tested as usize]);
                pass.draw_indexed(draw.first_index..draw.first_index + draw.index_count, draw.base_vertex, draw.instance..draw.instance + 1);
            }
        }
        for (pipeline, draws) in self.pipelines.iter().zip(draws) {
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

const ADDITIVE: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
    alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
};

fn shadow_maps() -> wgpu::BindingType {
    wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false }
}
