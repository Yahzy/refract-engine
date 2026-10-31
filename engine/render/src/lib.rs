//! The renderer: wgpu on Vulkan, shaders written in Slang and compiled to
//! SPIR-V at build time.
//!
//! The scene's geometry, materials and instances are uploaded once and kept on
//! the GPU; what changes every frame (camera, lights, sky) is passed to
//! [`Renderer::render`] in a [`Frame`].
//!
//! Lighting is HDR and physically based (metallic-roughness, GGX), with
//! dynamic shadows from the sun (cascaded shadow maps) and from local lights,
//! then exposed and tonemapped for display.

mod ambient;
mod buffer;
mod camera;
mod culling;
mod error;
pub mod gi;
mod gpu;
mod gpu_types;
mod light;
mod material;
mod mesh;
mod occluders;
mod pass;
mod readback;
mod renderer;
mod shader;
mod shadow;
pub mod shapes;
mod targets;
mod texture;

pub use ambient::ProbeGrid;
pub use camera::Camera;
pub use culling::Sphere;
pub use error::RenderError;
pub use light::{Fog, PointLight, Sky, SpotLight, Sun};
pub use material::{BlendMode, Detail, DetailBlend, Layer, Material, MaterialId};
pub use mesh::{MeshData, MeshId, MeshPart, VertexAttributes};
pub use texture::TextureId;
pub use readback::Image;
pub use renderer::{Frame, InstanceId, Renderer, RendererConfig};
