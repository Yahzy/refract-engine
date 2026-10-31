//! Textures, all bound at once (bindless): materials refer to them by index.
//! Every texture is a 2D array (animation frames are layers).

use refract_asset::{TextureData, TextureFormat};

/// Index of a texture in the bindless array.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextureId(pub(crate) u32);

impl TextureId {
    /// Plain white, for materials without a texture.
    pub const WHITE: TextureId = TextureId(0);
}

/// Most textures a scene can hold at once.
pub const MAX_TEXTURES: u32 = 16384;

pub struct TextureStore {
    views: Vec<wgpu::TextureView>,
}

impl TextureStore {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let mut store = Self { views: Vec::new() };
        let white = TextureData { format: TextureFormat::Rgba8, width: 1, height: 1, layers: 1, faces: 1, mips: vec![vec![255u8; 4].into()] };
        store.add(device, queue, &white, "white");
        store
    }

    pub fn add(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &TextureData, label: &str) -> TextureId {
        assert!((self.views.len() as u32) < MAX_TEXTURES, "too many textures");
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: data.width, height: data.height, depth_or_array_layers: data.layers * data.faces },
            mip_level_count: data.mips.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu_format(data.format),
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, bytes) in data.mips.iter().enumerate() {
            let (width, height) = data.mip_size(level);
            let (columns, rows) = if data.format.is_compressed() { (width.div_ceil(4), height.div_ceil(4)) } else { (width, height) };
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: level as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                bytes,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(columns * data.format.block_bytes() as u32), rows_per_image: Some(rows) },
                // Compressed mips are copied in whole blocks.
                wgpu::Extent3d { width: columns * block_size(data.format), height: rows * block_size(data.format), depth_or_array_layers: data.layers * data.faces },
            );
        }
        self.views.push(texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() }));
        TextureId(self.views.len() as u32 - 1)
    }

    pub fn views(&self) -> Vec<&wgpu::TextureView> {
        self.views.iter().collect()
    }
}

fn block_size(format: TextureFormat) -> u32 {
    if format.is_compressed() { 4 } else { 1 }
}

fn wgpu_format(format: TextureFormat) -> wgpu::TextureFormat {
    use wgpu::TextureFormat as W;
    match format {
        TextureFormat::Rgba8 => W::Rgba8Unorm,
        TextureFormat::Rgba8Srgb => W::Rgba8UnormSrgb,
        TextureFormat::Bc1 => W::Bc1RgbaUnorm,
        TextureFormat::Bc1Srgb => W::Bc1RgbaUnormSrgb,
        TextureFormat::Bc2 => W::Bc2RgbaUnorm,
        TextureFormat::Bc2Srgb => W::Bc2RgbaUnormSrgb,
        TextureFormat::Bc3 => W::Bc3RgbaUnorm,
        TextureFormat::Bc3Srgb => W::Bc3RgbaUnormSrgb,
        TextureFormat::Bc4 => W::Bc4RUnorm,
        TextureFormat::Bc5 => W::Bc5RgUnorm,
        TextureFormat::Bc6h => W::Bc6hRgbUfloat,
        TextureFormat::Bc7 => W::Bc7RgbaUnorm,
        TextureFormat::Bc7Srgb => W::Bc7RgbaUnormSrgb,
        TextureFormat::Rgba16Float => W::Rgba16Float,
    }
}
