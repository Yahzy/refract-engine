//! Per-leaf ambient lighting baked into the map: a few "ambient cubes" (light
//! arriving from each of the six axis directions) at points inside every
//! empty leaf. Here they become the indirect lighting of everything, as an
//! irradiance volume.

/// Light (linear, 1 = full brightness, can exceed it) arriving from +X, -X, +Y, -Y, +Z, -Z (map axes).
pub type AmbientCube = [[f32; 3]; 6];

#[derive(Debug, Clone)]
struct Sample {
    position: [f32; 3],
    cube: AmbientCube,
}

/// Every leaf's ambient samples.
#[derive(Debug, Clone, Default)]
pub struct LeafAmbient {
    /// Per leaf: range into `samples`.
    leaves: Vec<(u32, u32)>,
    samples: Vec<Sample>,
}

/// One `dleafambientlighting_t`: six RGBE colours, then the position as fractions of the leaf bounds.
const SAMPLE_SIZE: usize = 28;
const LUMP_LEAF_AMBIENT_INDEX_HDR: usize = 51;
const LUMP_LEAF_AMBIENT_INDEX: usize = 52;
const LUMP_LEAF_AMBIENT_LIGHTING_HDR: usize = 55;
const LUMP_LEAF_AMBIENT_LIGHTING: usize = 56;

impl LeafAmbient {
    /// Reads the LDR lumps, else the HDR ones: most maps are tuned for LDR,
    /// and some maps ship HDR lighting that was never tuned (all black).
    /// `bounds` are each leaf's (mins, maxs), in file order.
    pub fn read(data: &[u8], bounds: &[([f32; 3], [f32; 3])]) -> Option<LeafAmbient> {
        [(LUMP_LEAF_AMBIENT_INDEX, LUMP_LEAF_AMBIENT_LIGHTING), (LUMP_LEAF_AMBIENT_INDEX_HDR, LUMP_LEAF_AMBIENT_LIGHTING_HDR)]
            .into_iter()
            .find_map(|(index, lighting)| {
                let index = lump(data, index).filter(|l| !l.is_empty())?;
                let lighting = lump(data, lighting).filter(|l| !l.is_empty())?;
                Some(Self::parse(index, lighting, bounds))
            })
    }

    fn parse(index: &[u8], lighting: &[u8], bounds: &[([f32; 3], [f32; 3])]) -> LeafAmbient {
        let leaves: Vec<(u32, u32)> = index
            .chunks_exact(4)
            .map(|c| {
                let count = u16::from_le_bytes([c[0], c[1]]) as u32;
                let first = u16::from_le_bytes([c[2], c[3]]) as u32;
                (first, first + count)
            })
            .collect();
        let raw: Vec<&[u8]> = lighting.chunks_exact(SAMPLE_SIZE).collect();
        let mut samples = vec![Sample { position: [0.0; 3], cube: [[0.0; 3]; 6] }; raw.len()];
        for (leaf, &(first, end)) in leaves.iter().enumerate() {
            let Some((mins, maxs)) = bounds.get(leaf) else { continue };
            for i in first..end.min(raw.len() as u32) {
                let r = raw[i as usize];
                let cube = std::array::from_fn(|face| {
                    let c = &r[face * 4..face * 4 + 4];
                    // Stored as a colour and a shared exponent: c * 2^exponent.
                    let scale = 2f32.powi(c[3] as i8 as i32);
                    [c[0] as f32 * scale, c[1] as f32 * scale, c[2] as f32 * scale]
                });
                let position = std::array::from_fn(|k| mins[k] + (maxs[k] - mins[k]) * r[24 + k] as f32 / 255.0);
                samples[i as usize] = Sample { position, cube };
            }
        }
        LeafAmbient { leaves, samples }
    }

    /// The ambient cube of leaf `leaf` nearest to `point`, if the leaf has any.
    pub fn cube(&self, leaf: usize, point: [f32; 3]) -> Option<AmbientCube> {
        let &(first, end) = self.leaves.get(leaf)?;
        self.samples
            .get(first as usize..end as usize)?
            .iter()
            .min_by(|a, b| distance_squared(a.position, point).total_cmp(&distance_squared(b.position, point)))
            .map(|s| s.cube)
    }
}

fn distance_squared(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|k| (a[k] - b[k]) * (a[k] - b[k])).sum()
}

fn lump(data: &[u8], index: usize) -> Option<&[u8]> {
    let field = |at: usize| data.get(at..at + 4).map(|b| i32::from_le_bytes(b.try_into().unwrap()));
    let header = 8 + index * 16;
    let (offset, length) = (field(header)?, field(header + 4)?);
    data.get(offset.max(0) as usize..(offset.max(0) + length.max(0)) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_samples() {
        // One leaf with one sample: +X is 128 * 2^-8 = 0.5, the rest black.
        let index = [1u8, 0, 0, 0];
        let mut lighting = vec![0u8; SAMPLE_SIZE];
        lighting[..4].copy_from_slice(&[128, 128, 128, (-8i8) as u8]);
        lighting[24..27].copy_from_slice(&[0, 255, 128]);
        let ambient = LeafAmbient::parse(&index, &lighting, &[([0.0; 3], [10.0; 3])]);
        let cube = ambient.cube(0, [0.0; 3]).unwrap();
        assert_eq!(cube[0], [0.5; 3]);
        assert_eq!(cube[1], [0.0; 3]);
        assert_eq!(ambient.samples[0].position[1], 10.0);
    }
}
