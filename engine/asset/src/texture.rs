//! Textures, stored as standard KTX2 files: GPU formats (mostly block
//! compressed), full mip chains, array layers (animation frames) and cube faces.

use std::borrow::Cow;

use ktx2::Format;

use crate::AssetError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureFormat {
    Rgba8,
    Rgba8Srgb,
    Bc1,
    Bc1Srgb,
    Bc2,
    Bc2Srgb,
    Bc3,
    Bc3Srgb,
    Bc4,
    Bc5,
    Bc6h,
    Bc7,
    Bc7Srgb,
    Rgba16Float,
}

impl TextureFormat {
    const ALL: [(TextureFormat, Format); 14] = [
        (Self::Rgba8, Format::R8G8B8A8_UNORM),
        (Self::Rgba8Srgb, Format::R8G8B8A8_SRGB),
        (Self::Bc1, Format::BC1_RGBA_UNORM_BLOCK),
        (Self::Bc1Srgb, Format::BC1_RGBA_SRGB_BLOCK),
        (Self::Bc2, Format::BC2_UNORM_BLOCK),
        (Self::Bc2Srgb, Format::BC2_SRGB_BLOCK),
        (Self::Bc3, Format::BC3_UNORM_BLOCK),
        (Self::Bc3Srgb, Format::BC3_SRGB_BLOCK),
        (Self::Bc4, Format::BC4_UNORM_BLOCK),
        (Self::Bc5, Format::BC5_UNORM_BLOCK),
        (Self::Bc6h, Format::BC6H_UFLOAT_BLOCK),
        (Self::Bc7, Format::BC7_UNORM_BLOCK),
        (Self::Bc7Srgb, Format::BC7_SRGB_BLOCK),
        (Self::Rgba16Float, Format::R16G16B16A16_SFLOAT),
    ];

    fn ktx(self) -> Format {
        Self::ALL.iter().find(|(f, _)| *f == self).unwrap().1
    }

    fn from_ktx(format: Format) -> Option<Self> {
        Self::ALL.iter().find(|(_, k)| *k == format).map(|(f, _)| *f)
    }

    /// Bytes per 4x4 block, or per pixel for uncompressed formats.
    pub fn block_bytes(self) -> usize {
        match self {
            Self::Bc1 | Self::Bc1Srgb | Self::Bc4 => 8,
            Self::Bc2 | Self::Bc2Srgb | Self::Bc3 | Self::Bc3Srgb | Self::Bc5 | Self::Bc6h | Self::Bc7 | Self::Bc7Srgb => 16,
            Self::Rgba8 | Self::Rgba8Srgb => 4,
            Self::Rgba16Float => 8,
        }
    }

    pub fn is_compressed(self) -> bool {
        !matches!(self, Self::Rgba8 | Self::Rgba8Srgb | Self::Rgba16Float)
    }

    /// Bytes of one image (one layer, one face) of the given size.
    pub fn image_bytes(self, width: u32, height: u32) -> usize {
        if self.is_compressed() {
            width.div_ceil(4) as usize * height.div_ceil(4) as usize * self.block_bytes()
        } else {
            width as usize * height as usize * self.block_bytes()
        }
    }
}

/// A texture, borrowed from a file or owned by the converter.
#[derive(Clone, Debug)]
pub struct TextureData<'a> {
    pub format: TextureFormat,
    pub width: u32,
    pub height: u32,
    /// Array layers (animation frames); 1 for a plain texture.
    pub layers: u32,
    /// 6 for a cubemap (+X, -X, +Y, -Y, +Z, -Z), else 1.
    pub faces: u32,
    /// Mip levels, largest first. Each holds every layer, and within a layer
    /// every face, one after the other.
    pub mips: Vec<Cow<'a, [u8]>>,
}

const LEVEL_ALIGN: usize = 16;

impl<'a> TextureData<'a> {
    pub fn mip_size(&self, level: usize) -> (u32, u32) {
        ((self.width >> level).max(1), (self.height >> level).max(1))
    }

    pub fn read(bytes: &'a [u8]) -> Result<Self, AssetError> {
        let reader = ktx2::Reader::new(bytes).map_err(|err| AssetError::Texture(err.to_string()))?;
        let header = reader.header();
        if header.supercompression_scheme.is_some() {
            return Err(AssetError::Texture("supercompressed textures are not supported".into()));
        }
        let format = header.format.and_then(TextureFormat::from_ktx).ok_or_else(|| AssetError::Texture(format!("unsupported format {:?}", header.format)))?;
        let texture = Self {
            format,
            width: header.pixel_width,
            height: header.pixel_height.max(1),
            layers: header.layer_count.max(1),
            faces: header.face_count,
            // The same slices, borrowed from `bytes` rather than from the reader.
            mips: reader
                .levels()
                .map(|level| {
                    let start = level.data.as_ptr() as usize - bytes.as_ptr() as usize;
                    Cow::Borrowed(&bytes[start..start + level.data.len()])
                })
                .collect(),
        };
        for (level, data) in texture.mips.iter().enumerate() {
            let (w, h) = texture.mip_size(level);
            if data.len() != format.image_bytes(w, h) * (texture.layers * texture.faces) as usize {
                return Err(AssetError::Texture(format!("mip {level} has the wrong size")));
            }
        }
        Ok(texture)
    }

    /// The texture as a KTX2 file.
    pub fn write(&self) -> Vec<u8> {
        let format = self.format.ktx();
        let (dfd, type_size) = ktx2::dfd::Basic::from_format(format).expect("supported formats have a data format descriptor");
        let dfd = ktx2::dfd::Block::Basic(dfd).to_vec();
        let levels_start = ktx2::Header::LENGTH;
        let dfd_start = levels_start + self.mips.len() * ktx2::LevelIndex::LENGTH;
        let dfd_length = 4 + dfd.len();

        // Level data goes smallest first, as the specification recommends.
        let mut offset = dfd_start + dfd_length;
        let mut level_index = vec![ktx2::LevelIndex { byte_offset: 0, byte_length: 0, uncompressed_byte_length: 0 }; self.mips.len()];
        for (level, data) in self.mips.iter().enumerate().rev() {
            offset = offset.next_multiple_of(LEVEL_ALIGN);
            level_index[level] = ktx2::LevelIndex { byte_offset: offset as u64, byte_length: data.len() as u64, uncompressed_byte_length: data.len() as u64 };
            offset += data.len();
        }

        let header = ktx2::Header {
            format: Some(format),
            type_size,
            pixel_width: self.width,
            pixel_height: self.height,
            pixel_depth: 0,
            layer_count: if self.layers > 1 { self.layers } else { 0 },
            face_count: self.faces,
            level_count: self.mips.len() as u32,
            supercompression_scheme: None,
            index: ktx2::Index {
                dfd_byte_offset: dfd_start as u32,
                dfd_byte_length: dfd_length as u32,
                kvd_byte_offset: 0,
                kvd_byte_length: 0,
                sgd_byte_offset: 0,
                sgd_byte_length: 0,
            },
        };
        let mut bytes = Vec::with_capacity(offset);
        bytes.extend_from_slice(&header.as_bytes());
        for entry in &level_index {
            bytes.extend_from_slice(&entry.as_bytes());
        }
        bytes.extend_from_slice(&(dfd_length as u32).to_le_bytes());
        bytes.extend_from_slice(&dfd);
        for (level, data) in self.mips.iter().enumerate().rev() {
            bytes.resize(level_index[level].byte_offset as usize, 0);
            bytes.extend_from_slice(data);
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        // 8x8 BC1, two layers, full mip chain (8, 4, 2, 1).
        let mips: Vec<Cow<[u8]>> =
            (0..4).map(|level| Cow::Owned((0..TextureFormat::Bc1Srgb.image_bytes(8 >> level, 8 >> level) * 2).map(|i| (i * 7 + level) as u8).collect())).collect();
        let texture = TextureData { format: TextureFormat::Bc1Srgb, width: 8, height: 8, layers: 2, faces: 1, mips };
        let bytes = texture.write();
        let read = TextureData::read(&bytes).unwrap();
        assert_eq!((read.format, read.width, read.height, read.layers, read.faces), (TextureFormat::Bc1Srgb, 8, 8, 2, 1));
        assert_eq!(read.mips, texture.mips);
    }

    #[test]
    fn cubemap_round_trip() {
        let mips = vec![Cow::Owned(vec![1u8; 6 * TextureFormat::Bc6h.image_bytes(4, 4)])];
        let texture = TextureData { format: TextureFormat::Bc6h, width: 4, height: 4, layers: 1, faces: 6, mips };
        let read = TextureData::read(&texture.write()).map(|t| (t.faces, t.layers)).unwrap();
        assert_eq!(read, (6, 1));
    }
}
