//! Screen-sized render targets, recreated when the output is resized.

pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Reversed: cleared to 0 (infinitely far), nearer is greater.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

pub struct Targets {
    pub hdr: wgpu::TextureView,
    pub depth: wgpu::TextureView,
}

impl Targets {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let hdr = texture(device, "hdr", HDR_FORMAT, width, height, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
        let depth = texture(device, "depth", DEPTH_FORMAT, width, height, wgpu::TextureUsages::RENDER_ATTACHMENT);
        Self { hdr, depth }
    }
}

fn texture(device: &wgpu::Device, label: &str, format: wgpu::TextureFormat, width: u32, height: u32, usage: wgpu::TextureUsages) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
        .create_view(&Default::default())
}
