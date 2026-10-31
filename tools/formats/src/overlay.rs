//! `info_overlay`s: decals that the compiler projects onto map faces (road
//! markings, posters, cracks). vbsp removes the entities and stores each
//! overlay in the OVERLAYS lump as a quad in its own basis plus the faces it
//! covers; the engine clips the quad against each face and lays the pieces on
//! the surface, as done here.

use vbsp::{Bsp, Vector};

use crate::mesh::MeshBatch;

/// How far overlay pieces float above their surface, in world units
/// (depth bias does the rest).
const SURFACE_OFFSET: f32 = 0.25;

/// One overlay of the OVERLAYS lump (`doverlay_t`).
#[derive(Debug, Clone)]
pub struct Overlay {
    pub texinfo: i16,
    pub faces: Vec<i32>,
    /// Texture coordinates: `u[0]..u[1]` and `v[0]..v[1]` across the quad.
    pub u: [f32; 2],
    pub v: [f32; 2],
    /// Quad corners in the overlay's (U, V) basis.
    pub corners: [[f32; 2]; 4],
    pub origin: [f32; 3],
    /// Basis: U axis, V axis, normal.
    pub basis: [[f32; 3]; 3],
}

const OVERLAY_SIZE: usize = 352;
const FACE_COUNT_MASK: u16 = 0x3fff;

/// Reads the OVERLAYS lump (45).
pub fn read_overlays(data: &[u8]) -> Vec<Overlay> {
    let field = |at: usize| data.get(at..at + 4).map(|b| i32::from_le_bytes(b.try_into().unwrap()));
    let header = 8 + 45 * 16;
    let (Some(offset), Some(length)) = (field(header), field(header + 4)) else { return Vec::new() };
    let Some(lump) = data.get(offset.max(0) as usize..(offset.max(0) + length.max(0)) as usize) else {
        return Vec::new();
    };
    lump.chunks_exact(OVERLAY_SIZE)
        .map(|o| {
            let f32_at = |at: usize| f32::from_le_bytes(o[at..at + 4].try_into().unwrap());
            let vec_at = |at: usize| [f32_at(at), f32_at(at + 4), f32_at(at + 8)];
            let count = (u16::from_le_bytes([o[6], o[7]]) & FACE_COUNT_MASK).min(64) as usize;
            let faces = (0..count).map(|i| i32::from_le_bytes(o[8 + i * 4..12 + i * 4].try_into().unwrap())).collect();
            let points: [[f32; 3]; 4] = std::array::from_fn(|i| vec_at(280 + i * 12));
            let normal = vec_at(340);
            // vbsp packs the U axis into the corners' z.
            let u_axis = [points[0][2], points[1][2], points[2][2]];
            let v_axis = cross(normal, u_axis);
            Overlay {
                texinfo: i16::from_le_bytes([o[4], o[5]]),
                faces,
                u: [f32_at(264), f32_at(268)],
                v: [f32_at(272), f32_at(276)],
                corners: points.map(|p| [p[0], p[1]]),
                origin: vec_at(328),
                basis: [u_axis, v_axis, normal],
            }
        })
        .collect()
}

/// A triangle of the surface an overlay lies on.
type Triangle = [[f32; 3]; 3];

/// Lays `overlay` on `surface` (the triangles of the faces it covers) and
/// appends the pieces to `batch`, with UVs in the overlay texture's 0..1 space.
pub fn build_overlay(overlay: &Overlay, surface: &[Triangle], batch: &mut MeshBatch) {
    let [u_axis, v_axis, normal] = overlay.basis;
    let to_2d = |p: [f32; 3]| {
        let d = sub(p, overlay.origin);
        [dot(d, u_axis), dot(d, v_axis)]
    };
    let quad = overlay.corners;
    // Corner texture coordinates, in vbsp's corner order.
    let corner_uv = [
        [overlay.u[0], overlay.v[0]],
        [overlay.u[0], overlay.v[1]],
        [overlay.u[1], overlay.v[1]],
        [overlay.u[1], overlay.v[0]],
    ];
    for triangle in surface {
        let face_normal = normalize(cross(sub(triangle[1], triangle[0]), sub(triangle[2], triangle[0])));
        // Skip surfaces seen edge-on from the overlay, and back faces.
        if dot(face_normal, normal) <= 0.01 {
            continue;
        }
        let tri_2d = triangle.map(to_2d);
        let mut piece: Vec<[f32; 2]> = quad.to_vec();
        for i in 0..3 {
            piece = clip_polygon(&piece, tri_2d[i], tri_2d[(i + 1) % 3], tri_2d[(i + 2) % 3]);
            if piece.len() < 3 {
                break;
            }
        }
        if piece.len() < 3 {
            continue;
        }
        // Wind the piece counter-clockwise around the surface normal.
        let area: f32 = (0..piece.len())
            .map(|i| {
                let (a, b) = (piece[i], piece[(i + 1) % piece.len()]);
                a[0] * b[1] - b[0] * a[1]
            })
            .sum();
        if area < 0.0 {
            piece.reverse();
        }
        let base = batch.positions.len() as u32;
        for p in &piece {
            // Back onto the triangle's plane along the overlay normal, then lift off it.
            let on_basis = add(overlay.origin, add(scale(u_axis, p[0]), scale(v_axis, p[1])));
            let t = dot(sub(triangle[0], on_basis), face_normal) / dot(normal, face_normal);
            let position = add(add(on_basis, scale(normal, t)), scale(face_normal, SURFACE_OFFSET));
            batch.positions.push(position);
            batch.normals.push(face_normal);
            batch.uvs.push(quad_uv(&quad, &corner_uv, *p));
        }
        for i in 1..piece.len() as u32 - 1 {
            batch.indices.extend([base, base + i, base + i + 1]);
        }
    }
}

/// Keeps the part of `polygon` on the same side of edge `a`-`b` as `inside`.
fn clip_polygon(polygon: &[[f32; 2]], a: [f32; 2], b: [f32; 2], inside: [f32; 2]) -> Vec<[f32; 2]> {
    let edge = [b[0] - a[0], b[1] - a[1]];
    let side = |p: [f32; 2]| edge[0] * (p[1] - a[1]) - edge[1] * (p[0] - a[0]);
    let sign = if side(inside) >= 0.0 { 1.0 } else { -1.0 };
    let mut out = Vec::with_capacity(polygon.len() + 2);
    for i in 0..polygon.len() {
        let p = polygon[i];
        let q = polygon[(i + 1) % polygon.len()];
        let (dp, dq) = (side(p) * sign, side(q) * sign);
        if dp >= 0.0 {
            out.push(p);
        }
        if (dp >= 0.0) != (dq >= 0.0) {
            let t = dp / (dp - dq);
            out.push([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]);
        }
    }
    out
}

/// Texture coordinates of `p` inside the overlay quad, interpolated over the
/// quad's two triangles (0, 1, 2) and (0, 2, 3).
fn quad_uv(quad: &[[f32; 2]; 4], uvs: &[[f32; 2]; 4], p: [f32; 2]) -> [f32; 2] {
    let barycentric = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| {
        let det = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
        if det.abs() < 1e-8 {
            return None;
        }
        let l1 = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / det;
        let l2 = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / det;
        Some([l1, l2, 1.0 - l1 - l2])
    };
    let interpolate = |[i, j, k]: [usize; 3], l: [f32; 3]| {
        [0, 1].map(|c| uvs[i][c] * l[0] + uvs[j][c] * l[1] + uvs[k][c] * l[2])
    };
    match barycentric(quad[0], quad[1], quad[2]) {
        Some(l) if l.iter().all(|w| *w >= -1e-3) => interpolate([0, 1, 2], l),
        _ => match barycentric(quad[0], quad[2], quad[3]) {
            Some(l) => interpolate([0, 2, 3], l),
            None => uvs[0],
        },
    }
}

/// The triangles of a map face, displaced if it is a displacement.
pub fn face_triangles(bsp: &Bsp, face_index: usize) -> Vec<Triangle> {
    let Some(face) = bsp.faces.get(face_index) else { return Vec::new() };
    let face = vbsp::Handle::new(bsp, face);
    let to_array = |v: Vector| [v.x, v.y, v.z];
    if let Some(displacement) = face.displacement() {
        let size = 2usize.pow(displacement.power as u32) + 1;
        let positions: Vec<[f32; 3]> = displacement.displaced_vertices().map(to_array).collect();
        if positions.len() != size * size {
            return Vec::new();
        }
        let normal = to_array(face.normal());
        let mut triangles = Vec::new();
        for x in 0..size - 1 {
            for y in 0..size - 1 {
                let i = |x: usize, y: usize| positions[y * size + x];
                for mut t in [[i(x, y), i(x + 1, y), i(x, y + 1)], [i(x + 1, y), i(x + 1, y + 1), i(x, y + 1)]] {
                    // Face the same way as the base face.
                    if dot(cross(sub(t[1], t[0]), sub(t[2], t[0])), normal) < 0.0 {
                        t.swap(1, 2);
                    }
                    triangles.push(t);
                }
            }
        }
        return triangles;
    }
    let points: Vec<[f32; 3]> = face.vertex_positions().map(to_array).collect();
    let normal = to_array(face.normal());
    points
        .chunks_exact(3)
        .map(|t| {
            let mut t = [t[0], t[1], t[2]];
            if dot(cross(sub(t[1], t[0]), sub(t[2], t[0])), normal) < 0.0 {
                t.swap(1, 2);
            }
            t
        })
        .collect()
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn normalize(a: [f32; 3]) -> [f32; 3] {
    let length = dot(a, a).sqrt().max(1e-12);
    scale(a, 1.0 / length)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_on_floor() {
        // A 64x64 overlay on a 32x32 floor triangle pair: the piece is the floor itself.
        let overlay = Overlay {
            texinfo: 0,
            faces: vec![],
            u: [0.0, 1.0],
            v: [0.0, 1.0],
            corners: [[-32.0, -32.0], [-32.0, 32.0], [32.0, 32.0], [32.0, -32.0]],
            origin: [0.0, 0.0, 0.0],
            basis: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        };
        let floor = [[[-16.0, -16.0, 0.0], [16.0, -16.0, 0.0], [16.0, 16.0, 0.0]]];
        let mut batch = MeshBatch::default();
        build_overlay(&overlay, &floor, &mut batch);
        assert_eq!(batch.indices.len(), 3);
        for (p, uv) in batch.positions.iter().zip(&batch.uvs) {
            assert!((p[2] - SURFACE_OFFSET).abs() < 1e-4);
            // u follows the U axis from -32 to 32.
            assert!((uv[0] - (p[0] + 32.0) / 64.0).abs() < 1e-4, "{p:?} {uv:?}");
            assert!((uv[1] - (p[1] + 32.0) / 64.0).abs() < 1e-4, "{p:?} {uv:?}");
        }
    }
}
