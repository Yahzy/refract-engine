//! Shadow-only geometry from the map's solid brushes.
//!
//! Compiled maps drop faces nobody can see (`tools/toolsnodraw` roofs, wall backs,
//! `tools/toolsblocklight`), so a renderer that only draws the remaining faces
//! lets sunlight through them. Here those hidden brush sides are rebuilt from
//! the brush planes, to be drawn into shadow maps only.

use vbsp::{Bsp, BrushFlags, TextureFlags};

use crate::mesh::MeshBatch;

/// Half-size of the starting polygon for a brush side, larger than any map.
const HUGE: f64 = 65536.0;
/// Clipping tolerance, in world units.
const EPSILON: f64 = 0.01;

pub type V3 = [f64; 3];

/// The hidden sides of the solid, opaque brushes `include_brush` accepts, as one batch. `keep` gets
/// each side's centre and outward normal, to leave out e.g. the 3D skybox,
/// whose miniature buildings would otherwise shadow the real map.
pub fn hidden_brush_sides(
    bsp: &Bsp,
    include_brush: impl Fn(usize) -> bool,
    keep: impl Fn([f32; 3], [f32; 3]) -> bool,
) -> MeshBatch {
    let mut batch = MeshBatch { material: String::new(), ..Default::default() };
    for (index, brush) in bsp.brushes.iter().enumerate() {
        if !include_brush(index)
            || !brush.flags.contains(BrushFlags::SOLID)
            || brush.flags.intersects(BrushFlags::WINDOW | BrushFlags::GRATE | BrushFlags::TRANSLUCENT)
        {
            continue;
        }
        let start = brush.brush_side as usize;
        let Some(sides) = bsp.brush_sides.get(start..start + brush.num_brush_sides as usize) else { continue };

        // Skybox shells and triggers never block light.
        let textures: Vec<Option<(TextureFlags, String)>> = sides
            .iter()
            .map(|side| {
                let info = bsp.texture_info(usize::try_from(side.texture_info).ok()?)?;
                Some((info.flags, info.name().to_ascii_lowercase().replace('\\', "/")))
            })
            .collect();
        let excluded = textures.iter().flatten().any(|(flags, name)| {
            flags.intersects(TextureFlags::SKY | TextureFlags::SKY2D | TextureFlags::TRIGGER)
                || name.contains("toolsskybox")
                || name.contains("toolstrigger")
        });
        if excluded {
            continue;
        }

        let planes: Vec<(V3, f64)> = sides
            .iter()
            .filter_map(|side| {
                let plane = bsp.planes.get(side.plane as usize)?;
                Some(([plane.normal.x as f64, plane.normal.y as f64, plane.normal.z as f64], plane.dist as f64))
            })
            .collect();
        if planes.len() != sides.len() {
            continue;
        }
        for (i, side) in sides.iter().enumerate() {
            let hidden = textures[i].as_ref().is_some_and(|(flags, name)| {
                flags.contains(TextureFlags::NODRAW) || name.starts_with("tools/")
            });
            if side.bevel != 0 || !hidden {
                continue;
            }
            let polygon = side_polygon(&planes, i);
            if polygon.len() < 3 {
                continue;
            }
            let center = scale(polygon.iter().fold([0.0; 3], |sum, p| add(sum, *p)), 1.0 / polygon.len() as f64);
            let normal = planes[i].0;
            let to_f32 = |v: V3| [v[0] as f32, v[1] as f32, v[2] as f32];
            if keep(to_f32(center), to_f32(normal)) {
                add_polygon(&mut batch, &polygon, normal);
            }
        }
    }
    batch
}

/// The polygon of brush side `index`: its plane, clipped by every other side.
pub fn side_polygon(planes: &[(V3, f64)], index: usize) -> Vec<V3> {
    let (normal, dist) = planes[index];
    let up = if normal[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
    let u = normalize(cross(up, normal));
    let v = cross(normal, u);
    let center = scale(normal, dist);
    // Counter-clockwise seen from outside (along +normal).
    let mut polygon: Vec<V3> = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
        .iter()
        .map(|(a, b)| add(center, add(scale(u, a * HUGE), scale(v, b * HUGE))))
        .collect();
    for (j, (n, d)) in planes.iter().enumerate() {
        if j == index || polygon.len() < 3 {
            continue;
        }
        polygon = clip(&polygon, *n, *d);
    }
    polygon
}

/// Keeps the part of `polygon` behind the plane (`n·p <= d`), the inside of the brush.
fn clip(polygon: &[V3], n: V3, d: f64) -> Vec<V3> {
    let mut out = Vec::with_capacity(polygon.len() + 1);
    for i in 0..polygon.len() {
        let a = polygon[i];
        let b = polygon[(i + 1) % polygon.len()];
        let da = dot(n, a) - d;
        let db = dot(n, b) - d;
        if da <= EPSILON {
            out.push(a);
        }
        if (da < -EPSILON && db > EPSILON) || (da > EPSILON && db < -EPSILON) {
            let t = da / (da - db);
            out.push(add(a, scale(sub(b, a), t)));
        }
    }
    out
}

fn add_polygon(batch: &mut MeshBatch, polygon: &[V3], normal: V3) {
    if polygon.len() < 3 {
        return;
    }
    let base = batch.positions.len() as u32;
    let normal = [normal[0] as f32, normal[1] as f32, normal[2] as f32];
    for p in polygon {
        batch.positions.push([p[0] as f32, p[1] as f32, p[2] as f32]);
        batch.normals.push(normal);
        batch.uvs.push([0.0, 0.0]);
    }
    for i in 1..polygon.len() as u32 - 1 {
        batch.indices.extend([base, base + i, base + i + 1]);
    }
}

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn normalize(a: V3) -> V3 {
    let length = dot(a, a).sqrt();
    scale(a, 1.0 / length)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_side() {
        // A 2x2x2 cube centred on the origin.
        let planes: Vec<(V3, f64)> = vec![
            ([1.0, 0.0, 0.0], 1.0),
            ([-1.0, 0.0, 0.0], 1.0),
            ([0.0, 1.0, 0.0], 1.0),
            ([0.0, -1.0, 0.0], 1.0),
            ([0.0, 0.0, 1.0], 1.0),
            ([0.0, 0.0, -1.0], 1.0),
        ];
        let top = side_polygon(&planes, 4);
        assert_eq!(top.len(), 4);
        for p in &top {
            assert!((p[2] - 1.0).abs() < 1e-6 && p[0].abs() <= 1.0 + 1e-6 && p[1].abs() <= 1.0 + 1e-6);
        }
        // Counter-clockwise around +Z.
        let n = cross(sub(top[1], top[0]), sub(top[2], top[0]));
        assert!(n[2] > 0.0);
    }
}
