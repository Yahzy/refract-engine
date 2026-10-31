//! Screen-sized render targets, recreated when the output is resized.

pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Reversed: cleared to 0 (infinitely far), nearer is greater.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

pub struct Targets {
    pub hdr: wgpu::TextureView,
    pub depth: wgpu::TextureView,
    /// The opaque scene's image and depth, copied before water is drawn:
    /// water shows them through its surface and reflects them.
    pub opaque_color: wgpu::TextureView,
    pub opaque_depth: wgpu::TextureView,
    hdr_texture: wgpu::Texture,
    depth_texture: wgpu::Texture,
    opaque_color_texture: wgpu::Texture,
    opaque_depth_texture: wgpu::Texture,
}

impl Targets {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        use wgpu::TextureUsages as U;
        let hdr_texture = texture(device, "hdr", HDR_FORMAT, width, height, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING | U::COPY_SRC);
        let depth_texture = texture(device, "depth", DEPTH_FORMAT, width, height, U::RENDER_ATTACHMENT | U::COPY_SRC);
        let opaque_color_texture = texture(device, "opaque color", HDR_FORMAT, width, height, U::TEXTURE_BINDING | U::COPY_DST);
        let opaque_depth_texture = texture(device, "opaque depth", DEPTH_FORMAT, width, height, U::TEXTURE_BINDING | U::COPY_DST);
        Self {
            hdr: hdr_texture.create_view(&Default::default()),
            depth: depth_texture.create_view(&Default::default()),
            opaque_color: opaque_color_texture.create_view(&Default::default()),
            opaque_depth: opaque_depth_texture.create_view(&Default::default()),
            hdr_texture,
            depth_texture,
            opaque_color_texture,
            opaque_depth_texture,
        }
    }

    /// Copies the image and depth drawn so far to `opaque_color` and `opaque_depth`.
    pub fn copy_opaque(&self, encoder: &mut wgpu::CommandEncoder) {
        let size = self.hdr_texture.size();
        encoder.copy_texture_to_texture(self.hdr_texture.as_image_copy(), self.opaque_color_texture.as_image_copy(), size);
        encoder.copy_texture_to_texture(self.depth_texture.as_image_copy(), self.opaque_depth_texture.as_image_copy(), size);
    }
}

fn texture(device: &wgpu::Device, label: &str, format: wgpu::TextureFormat, width: u32, height: u32, usage: wgpu::TextureUsages) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}
