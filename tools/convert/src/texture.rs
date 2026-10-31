//! Textures: VTF to KTX2.
//!
//! Block-compressed VTFs (DXT1/3/5) are copied as they are: no quality is
//! lost and nothing is encoded. Other VTFs are decoded and encoded to BC7.
//! Animation frames become array layers.

use std::borrow::Cow;

use refract_asset::{TextureData, TextureFormat};
use refract_formats::material::{self, BlockFormat, CompressedTexture, Texture};

use crate::{Converter, texture_path};

/// Animated textures keep at most this many frames.
const MAX_FRAMES: u32 = 64;

/// How the texture's values are read: as colours (sRGB) or as data (linear:
/// normal maps, masks).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Color,
    Data,
    /// A self-shadowed bump map (`$ssbump`), stored as a normal map.
    SelfShadowedBump,
}

impl Converter {
    /// Converts `materials/<name>.vtf`; returns its native path, or `None`
    /// when there is no such texture.
    pub(crate) fn texture(&self, name: &str, role: Role) -> Option<String> {
        let path = texture_path(name);
        if !self.claim(&path) {
            // Exists, or another thread converts it. If the source turns out
            // to be missing, the engine shows its placeholder texture.
            return Some(path);
        }
        let compressed = if role == Role::SelfShadowedBump { None } else { material::load_compressed_frames(&self.fs, name, MAX_FRAMES) };
        let data = match compressed {
            Some(frames) if !frames.is_empty() => from_compressed(&frames, role),
            _ => {
                let mut frames = material::load_texture_frames(&self.fs, name, MAX_FRAMES);
                if frames.is_empty() {
                    log::warn!("missing texture {name}");
                    return None;
                }
                if role == Role::SelfShadowedBump {
                    frames.iter_mut().for_each(Texture::ssbump_to_normal);
                }
                from_rgba(&frames, role)
            }
        };
        match self.write(&path, &data.write()) {
            Ok(()) => Some(path),
            Err(err) => {
                log::warn!("{err:#}");
                None
            }
        }
    }
}

fn from_compressed(frames: &[CompressedTexture], role: Role) -> TextureData<'static> {
    let first = &frames[0];
    let srgb = role == Role::Color;
    let format = match (first.format, srgb) {
        (BlockFormat::Bc1, true) => TextureFormat::Bc1Srgb,
        (BlockFormat::Bc1, false) => TextureFormat::Bc1,
        (BlockFormat::Bc2, true) => TextureFormat::Bc2Srgb,
        (BlockFormat::Bc2, false) => TextureFormat::Bc2,
        (BlockFormat::Bc3, true) => TextureFormat::Bc3Srgb,
        (BlockFormat::Bc3, false) => TextureFormat::Bc3,
    };
    let mips = (0..first.mips.len()).map(|level| Cow::Owned(frames.iter().flat_map(|f| f.mips[level].iter().copied()).collect())).collect();
    TextureData { format, width: first.width, height: first.height, layers: frames.len() as u32, faces: 1, mips }
}

fn from_rgba(frames: &[Texture], role: Role) -> TextureData<'static> {
    let first = &frames[0];
    let srgb = role == Role::Color;
    // The GPU wants block-compressed textures to be whole blocks wide and high.
    let compress = first.width.is_multiple_of(4) && first.height.is_multiple_of(4);
    let format = match (compress, srgb) {
        (true, true) => TextureFormat::Bc7Srgb,
        (true, false) => TextureFormat::Bc7,
        (false, true) => TextureFormat::Rgba8Srgb,
        (false, false) => TextureFormat::Rgba8,
    };
    let opaque = frames.iter().all(|f| f.mips[0].as_chunks::<4>().0.iter().all(|p| p[3] == 255));
    let settings = if opaque { intel_tex_2::bc7::opaque_fast_settings() } else { intel_tex_2::bc7::alpha_fast_settings() };
    let mips = (0..first.mips.len())
        .map(|level| {
            let (w, h) = ((first.width >> level).max(1), (first.height >> level).max(1));
            let mut bytes = Vec::new();
            for frame in frames {
                let pixels = &frame.mips[level];
                if compress {
                    bytes.extend(encode_bc7(&settings, pixels, w, h));
                } else {
                    bytes.extend_from_slice(pixels);
                }
            }
            Cow::Owned(bytes)
        })
        .collect();
    TextureData { format, width: first.width, height: first.height, layers: frames.len() as u32, faces: 1, mips }
}

/// BC7 blocks of an RGBA8 image; images smaller than a block are padded by
/// repeating their edges.
fn encode_bc7(settings: &intel_tex_2::bc7::EncodeSettings, pixels: &[u8], width: u32, height: u32) -> Vec<u8> {
    let (padded_w, padded_h) = (width.next_multiple_of(4), height.next_multiple_of(4));
    let padded: Cow<[u8]> = if (padded_w, padded_h) == (width, height) {
        Cow::Borrowed(pixels)
    } else {
        let mut out = Vec::with_capacity((padded_w * padded_h * 4) as usize);
        for y in 0..padded_h {
            for x in 0..padded_w {
                let at = ((y.min(height - 1) * width + x.min(width - 1)) * 4) as usize;
                out.extend_from_slice(&pixels[at..at + 4]);
            }
        }
        Cow::Owned(out)
    };
    let surface = intel_tex_2::RgbaSurface { data: &padded, width: padded_w, height: padded_h, stride: padded_w * 4 };
    intel_tex_2::bc7::compress_blocks(settings, &surface)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_textures_become_bc7_with_every_mip() {
        // 8x8 with mips 8, 4, 2, 1; two frames.
        let frame = Texture { width: 8, height: 8, mips: [8u32, 4, 2, 1].iter().map(|s| vec![200u8; (s * s * 4) as usize]).collect() };
        let data = from_rgba(&[frame.clone(), frame], Role::Color);
        assert_eq!(data.format, TextureFormat::Bc7Srgb);
        assert_eq!(data.layers, 2);
        for (level, mip) in data.mips.iter().enumerate() {
            let (w, h) = data.mip_size(level);
            assert_eq!(mip.len(), TextureFormat::Bc7Srgb.image_bytes(w, h) * 2);
        }
    }

    #[test]
    fn odd_sizes_stay_uncompressed() {
        let frame = Texture { width: 6, height: 2, mips: vec![vec![0u8; 6 * 2 * 4], vec![0u8; 3 * 4], vec![0u8; 4]] };
        assert_eq!(from_rgba(&[frame], Role::Data).format, TextureFormat::Rgba8);
    }
}
