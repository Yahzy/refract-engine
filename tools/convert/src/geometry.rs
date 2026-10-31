//! Triangle batches to native geometry: encoded normals, generated tangents,
//! one submesh per batch with its bounding sphere.

use glam::{Vec2, Vec3};
use refract_asset::encoding::{oct_encode, snorm16};
use refract_asset::geometry::GeometryData;
use refract_asset::{Submesh, VertexAttributes};
use refract_formats::bsp::MeshBatch;

/// Appends `batch` as one submesh using `material` (a native path; empty for
/// none). `uv_scale` divides its UVs.
pub fn append(geometry: &mut GeometryData, batch: &MeshBatch, material: &str, uv_scale: [f32; 2], flags: u32) {
    if batch.indices.is_empty() {
        return;
    }
    let base = geometry.positions.len() as u32;
    let first_index = geometry.indices.len() as u32;
    let uvs: Vec<Vec2> = batch.uvs.iter().map(|uv| Vec2::new(uv[0] / uv_scale[0], uv[1] / uv_scale[1])).collect();
    let tangents = tangents(&batch.positions, &batch.normals, &uvs, &batch.indices);

    for (i, position) in batch.positions.iter().enumerate() {
        let normal = Vec3::from(batch.normals[i]).normalize_or(Vec3::Z);
        let (tangent, sign) = tangents[i];
        let blend = batch.blend.get(i).map_or(0, |b| (b.clamp(0.0, 1.0) * 255.0).round() as u8);
        geometry.positions.push(*position);
        geometry.attributes.push(VertexAttributes {
            normal: oct_encode(normal),
            tangent: [snorm16(tangent.x), snorm16(tangent.y), snorm16(tangent.z), snorm16(sign)],
            uv: uvs[i].to_array(),
            color: [255, 255, 255, blend],
        });
    }
    geometry.indices.extend(batch.indices.iter().map(|i| i + base));

    let material = geometry.material_index(material);
    let (center, radius) = bounds(&batch.positions);
    geometry.submeshes.push(Submesh { first_index, index_count: batch.indices.len() as u32, material, flags, center: center.to_array(), radius });
}

/// Per-vertex tangent (along increasing U) orthogonal to the normal, and the
/// sign of the bitangent (along increasing V) relative to normal × tangent.
fn tangents(positions: &[[f32; 3]], normals: &[[f32; 3]], uvs: &[Vec2], indices: &[u32]) -> Vec<(Vec3, f32)> {
    let mut tan = vec![Vec3::ZERO; positions.len()];
    let mut bitan = vec![Vec3::ZERO; positions.len()];
    for tri in indices.as_chunks::<3>().0 {
        let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| i as usize);
        let (e1, e2) = (Vec3::from(positions[b]) - Vec3::from(positions[a]), Vec3::from(positions[c]) - Vec3::from(positions[a]));
        let (d1, d2) = (uvs[b] - uvs[a], uvs[c] - uvs[a]);
        let det = d1.x * d2.y - d2.x * d1.y;
        if det.abs() < 1e-12 {
            continue;
        }
        let t = (e1 * d2.y - e2 * d1.y) / det;
        let bt = (e2 * d1.x - e1 * d2.x) / det;
        for v in [a, b, c] {
            tan[v] += t;
            bitan[v] += bt;
        }
    }
    (0..positions.len())
        .map(|i| {
            let n = Vec3::from(normals[i]).normalize_or(Vec3::Z);
            let t = (tan[i] - n * n.dot(tan[i])).try_normalize().unwrap_or_else(|| n.any_orthonormal_vector());
            let sign = if n.cross(t).dot(bitan[i]) < 0.0 { -1.0 } else { 1.0 };
            (t, sign)
        })
        .collect()
}

fn bounds(positions: &[[f32; 3]]) -> (Vec3, f32) {
    let (min, max) = positions.iter().fold((Vec3::MAX, Vec3::MIN), |(min, max), p| (min.min(Vec3::from(*p)), max.max(Vec3::from(*p))));
    let center = (min + max) * 0.5;
    let radius = positions.iter().map(|p| Vec3::from(*p).distance(center)).fold(0.0, f32::max);
    (center, radius)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quad_gets_tangents_along_u() {
        let batch = MeshBatch {
            material: "a".into(),
            positions: vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 2.0, 0.0], [0.0, 2.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            uvs: vec![[0.0, 0.0], [64.0, 0.0], [64.0, 64.0], [0.0, 64.0]],
            texel_uvs: true,
            blend: vec![],
            indices: vec![0, 1, 2, 0, 2, 3],
        };
        let mut geometry = GeometryData::default();
        append(&mut geometry, &batch, "materials/a.rmat", [64.0, 64.0], 0);
        assert_eq!(geometry.attributes[2].uv, [1.0, 1.0]);
        assert_eq!(&geometry.attributes[0].tangent[..3], &[32767, 0, 0]);
        assert_eq!(geometry.attributes[0].tangent[3], 32767);
        assert_eq!(geometry.submeshes[0].index_count, 6);
        assert!((geometry.submeshes[0].radius - 2f32.sqrt()).abs() < 1e-5);
    }
}
