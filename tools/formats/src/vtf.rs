//! VTF textures, versions 7.0 to 7.5: every mip level, animation frame and
//! cubemap face, decoded to RGBA8.
//!
//! Layout: a header, a low-resolution thumbnail, then the high-resolution data
//! ordered smallest mip first, and within a mip by frame, face, then depth slice.

use anyhow::{Context, Result, ensure};

const FLAG_ENVMAP: u32 = 0x4000;
/// Resource tag of the high-resolution image data (7.3+).
const HIGHRES_TAG: [u8; 3] = [0x30, 0, 0];

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Format {
    Rgba8888,
    Abgr8888,
    Rgb888,
    Bgr888,
    Rgb565,
    I8,
    Ia88,
    A8,
    Rgb888Bluescreen,
    Bgr888Bluescreen,
    Argb8888,
    Bgra8888,
    Dxt1,
    Dxt3,
    Dxt5,
    Bgrx8888,
    Bgr565,
    Bgrx5551,
    Bgra4444,
    Dxt1OneBitAlpha,
    Bgra5551,
    Uv88,
    Uvwq8888,
    Rgba16161616F,
    Rgba16161616,
}

impl Format {
    fn from_id(id: i32) -> Option<Format> {
        use Format::*;
        Some(match id {
            0 => Rgba8888,
            1 => Abgr8888,
            2 => Rgb888,
            3 => Bgr888,
            4 => Rgb565,
            5 => I8,
            6 => Ia88,
            8 => A8,
            9 => Rgb888Bluescreen,
            10 => Bgr888Bluescreen,
            11 => Argb8888,
            12 => Bgra8888,
            13 => Dxt1,
            14 => Dxt3,
            15 => Dxt5,
            16 => Bgrx8888,
            17 => Bgr565,
            18 => Bgrx5551,
            19 => Bgra4444,
            20 => Dxt1OneBitAlpha,
            21 => Bgra5551,
            22 => Uv88,
            23 => Uvwq8888,
            24 => Rgba16161616F,
            25 => Rgba16161616,
            _ => return None,
        })
    }

    /// Bytes for a `width` x `height` image.
    fn size(self, width: u32, height: u32) -> usize {
        let (w, h) = (width as usize, height as usize);
        let blocks = w.div_ceil(4) * h.div_ceil(4);
        use Format::*;
        match self {
            Dxt1 | Dxt1OneBitAlpha => blocks * 8,
            Dxt3 | Dxt5 => blocks * 16,
            I8 | A8 => w * h,
            Ia88 | Rgb565 | Bgr565 | Bgrx5551 | Bgra4444 | Bgra5551 | Uv88 => w * h * 2,
            Rgb888 | Bgr888 | Rgb888Bluescreen | Bgr888Bluescreen => w * h * 3,
            Rgba16161616F | Rgba16161616 => w * h * 8,
            _ => w * h * 4,
        }
    }
}

/// A parsed VTF; pixel data stays in the borrowed file bytes until decoded.
pub struct Vtf<'a> {
    pub width: u32,
    pub height: u32,
    pub flags: u32,
    pub frames: u32,
    pub faces: u32,
    pub mip_count: u32,
    pub format: Format,
    depth: u32,
    data: &'a [u8],
    /// Offset of the high-resolution data (smallest mip first).
    offset: usize,
}

impl<'a> Vtf<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Vtf<'a>> {
        ensure!(data.len() >= 64 && &data[..4] == b"VTF\0", "not a VTF file");
        let u16_at = |o: usize| u16::from_le_bytes([data[o], data[o + 1]]);
        let u32_at = |o: usize| u32::from_le_bytes(data[o..o + 4].try_into().unwrap());
        let minor = u32_at(8);
        let header_size = u32_at(12) as usize;
        let width = u16_at(16) as u32;
        let height = u16_at(18) as u32;
        let flags = u32_at(20);
        let frames = (u16_at(24) as u32).max(1);
        let first_frame = u16_at(26);
        let format_id = u32_at(52) as i32;
        let format = Format::from_id(format_id).with_context(|| format!("unsupported VTF format {format_id}"))?;
        let mip_count = (data[56] as u32).max(1);
        let low_format = u32_at(57) as i32;
        let (low_width, low_height) = (data[61] as u32, data[62] as u32);
        let depth = if minor >= 2 && data.len() > 64 { (u16_at(63) as u32).max(1) } else { 1 };

        let faces = if flags & FLAG_ENVMAP == 0 {
            1
        } else if minor >= 5 || first_frame == 0xffff {
            6
        } else {
            // Before 7.5, cubemaps carry a seventh face: a sphere map.
            7
        };

        let offset = if minor >= 3 {
            let count = u32_at(68) as usize;
            (0..count)
                .map(|i| 80 + i * 8)
                .filter(|o| o + 8 <= data.len())
                .find(|o| data[*o..*o + 3] == HIGHRES_TAG)
                .map(|o| u32_at(o + 4) as usize)
                .context("VTF without image data")?
        } else {
            let low_size = Format::from_id(low_format).map_or(0, |f| f.size(low_width, low_height));
            header_size + if low_format >= 0 { low_size } else { 0 }
        };
        let vtf = Vtf { width, height, flags, frames, faces, mip_count, format, depth, data, offset };
        ensure!(vtf.offset + vtf.total_size() <= data.len(), "VTF data truncated");
        Ok(vtf)
    }

    fn mip_dimensions(&self, mip: u32) -> (u32, u32) {
        ((self.width >> mip).max(1), (self.height >> mip).max(1))
    }

    fn mip_size(&self, mip: u32) -> usize {
        let (w, h) = self.mip_dimensions(mip);
        self.format.size(w, h) * (self.depth >> mip).max(1) as usize
    }

    fn total_size(&self) -> usize {
        (0..self.mip_count).map(|m| self.mip_size(m) * (self.frames * self.faces) as usize).sum()
    }

    /// RGBA8 pixels of one mip level (0 is the largest) of one frame and face.
    pub fn decode(&self, mip: u32, frame: u32, face: u32) -> Result<Vec<u8>> {
        let (w, h) = self.mip_dimensions(mip);
        decode(self.format, self.raw(mip, frame, face)?, w, h)
    }

    /// One mip level of one frame and face as stored, in [`Vtf::format`].
    pub fn raw(&self, mip: u32, frame: u32, face: u32) -> Result<&'a [u8]> {
        ensure!(mip < self.mip_count && frame < self.frames && face < self.faces, "VTF image out of range");
        // Smaller mips come first.
        let before: usize =
            (mip + 1..self.mip_count).map(|m| self.mip_size(m) * (self.frames * self.faces) as usize).sum();
        let start = self.offset + before + self.mip_size(mip) * (frame * self.faces + face) as usize;
        let (w, h) = self.mip_dimensions(mip);
        Ok(&self.data[start..start + self.format.size(w, h)])
    }

    /// Dimensions of a mip level.
    pub fn dimensions(&self, mip: u32) -> (u32, u32) {
        self.mip_dimensions(mip)
    }
}

fn decode(format: Format, bytes: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let pixels = (width * height) as usize;
    let dxt = |kind: texpresso::Format| {
        let mut out = vec![0u8; pixels * 4];
        kind.decompress(bytes, width as usize, height as usize, &mut out);
        out
    };
    let map = |stride: usize, f: &dyn Fn(&[u8]) -> [u8; 4]| -> Vec<u8> {
        bytes.chunks_exact(stride).take(pixels).flat_map(f).collect()
    };
    let r5 = |v: u16, shift: u32| (((v >> shift) & 0x1f) as u32 * 255 / 31) as u8;
    let g6 = |v: u16| (((v >> 5) & 0x3f) as u32 * 255 / 63) as u8;
    let le16 = |p: &[u8], i: usize| u16::from_le_bytes([p[i], p[i + 1]]);
    // Bluescreen formats use pure blue as the transparent colour.
    let bluescreen = |r: u8, g: u8, b: u8| if (r, g, b) == (0, 0, 255) { [0, 0, 0, 0] } else { [r, g, b, 255] };

    use Format::*;
    Ok(match format {
        Dxt1 | Dxt1OneBitAlpha => dxt(texpresso::Format::Bc1),
        Dxt3 => dxt(texpresso::Format::Bc2),
        Dxt5 => dxt(texpresso::Format::Bc3),
        Rgba8888 => map(4, &|p| [p[0], p[1], p[2], p[3]]),
        Abgr8888 => map(4, &|p| [p[3], p[2], p[1], p[0]]),
        Argb8888 => map(4, &|p| [p[1], p[2], p[3], p[0]]),
        Bgra8888 => map(4, &|p| [p[2], p[1], p[0], p[3]]),
        Bgrx8888 => map(4, &|p| [p[2], p[1], p[0], 255]),
        Uvwq8888 => map(4, &|p| [p[0], p[1], p[2], p[3]]),
        Rgb888 => map(3, &|p| [p[0], p[1], p[2], 255]),
        Bgr888 => map(3, &|p| [p[2], p[1], p[0], 255]),
        Rgb888Bluescreen => map(3, &|p| bluescreen(p[0], p[1], p[2])),
        Bgr888Bluescreen => map(3, &|p| bluescreen(p[2], p[1], p[0])),
        I8 => map(1, &|p| [p[0], p[0], p[0], 255]),
        A8 => map(1, &|p| [0, 0, 0, p[0]]),
        Ia88 => map(2, &|p| [p[0], p[0], p[0], p[1]]),
        Uv88 => map(2, &|p| [p[0], p[1], 0, 255]),
        Rgb565 => map(2, &|p| {
            let v = le16(p, 0);
            [r5(v, 11), g6(v), r5(v, 0), 255]
        }),
        Bgr565 => map(2, &|p| {
            let v = le16(p, 0);
            [r5(v, 0), g6(v), r5(v, 11), 255]
        }),
        Bgrx5551 | Bgra5551 => map(2, &|p| {
            let v = le16(p, 0);
            let a = if format == Bgra5551 && v & 0x8000 == 0 { 0 } else { 255 };
            [r5(v, 10), r5(v, 5), r5(v, 0), a]
        }),
        Bgra4444 => map(2, &|p| {
            let v = le16(p, 0);
            let n = |s: u32| ((v >> s) & 0xf) as u8 * 17;
            [n(8), n(4), n(0), n(12)]
        }),
        Rgba16161616 => map(8, &|p| [p[1], p[3], p[5], p[7]]),
        Rgba16161616F => map(8, &|p| {
            let c = |i: usize| (half_to_f32(le16(p, i)).clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8;
            [c(0), c(2), c(4), (half_to_f32(le16(p, 6)).clamp(0.0, 1.0) * 255.0) as u8]
        }),
    })
}

fn half_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = ((h >> 10) & 0x1f) as i32;
    let mantissa = (h & 0x3ff) as f32;
    sign * match exponent {
        0 => mantissa * 2f32.powi(-24),
        31 => f32::INFINITY,
        e => (1.0 + mantissa / 1024.0) * 2f32.powi(e - 15),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(minor: u32, width: u16, height: u16, format: i32, mips: u8) -> Vec<u8> {
        let mut h = vec![0u8; 80];
        h[..4].copy_from_slice(b"VTF\0");
        h[4..8].copy_from_slice(&7u32.to_le_bytes());
        h[8..12].copy_from_slice(&minor.to_le_bytes());
        h[12..16].copy_from_slice(&80u32.to_le_bytes());
        h[16..18].copy_from_slice(&width.to_le_bytes());
        h[18..20].copy_from_slice(&height.to_le_bytes());
        h[24..26].copy_from_slice(&1u16.to_le_bytes());
        h[52..56].copy_from_slice(&format.to_le_bytes());
        h[56] = mips;
        h[57..61].copy_from_slice(&(-1i32).to_le_bytes());
        h
    }

    #[test]
    fn decodes_mips_smallest_first() {
        // 2x2 RGBA8888 with 2 mips: the 1x1 mip comes first in the file.
        let mut file = header(2, 2, 2, 0, 2);
        file.extend_from_slice(&[9, 9, 9, 9]);
        file.extend((0..4).flat_map(|i| [i, i, i, 255]));
        let vtf = Vtf::parse(&file).unwrap();
        assert_eq!(vtf.decode(1, 0, 0).unwrap(), vec![9, 9, 9, 9]);
        assert_eq!(&vtf.decode(0, 0, 0).unwrap()[4..8], &[1, 1, 1, 255]);
        assert_eq!(half_to_f32(0x3c00), 1.0);
    }
}
