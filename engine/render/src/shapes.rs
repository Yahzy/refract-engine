//! Simple meshes built in code: test scenes, debugging, placeholders.

use std::f32::consts::{PI, TAU};

use glam::{Vec2, Vec3};
use refract_asset::encoding::{oct_encode, snorm16};

use crate::{MeshData, VertexAttributes};

/// A square in the XY plane facing +Z, `size` wide, with its texture repeated
/// every `uv_scale` units.
pub fn plane(size: f32, uv_scale: f32) -> MeshData {
    let mut mesh = MeshData::default();
    add_quad(&mut mesh, Vec3::ZERO, Vec3::X * size * 0.5, Vec3::Y * size * 0.5, size / uv_scale);
    mesh
}

/// A box centered on the origin.
pub fn cuboid(half_extents: Vec3) -> MeshData {
    let mut mesh = MeshData::default();
    let h = half_extents;
    // Each face: normal axis, then two axes whose cross product is the normal.
    let faces = [
        (Vec3::X * h.x, Vec3::Y * h.y, Vec3::Z * h.z),
        (-Vec3::X * h.x, -Vec3::Y * h.y, Vec3::Z * h.z),
        (Vec3::Y * h.y, Vec3::Z * h.z, Vec3::X * h.x),
        (-Vec3::Y * h.y, -Vec3::Z * h.z, Vec3::X * h.x),
        (Vec3::Z * h.z, Vec3::X * h.x, Vec3::Y * h.y),
        (-Vec3::Z * h.z, -Vec3::X * h.x, Vec3::Y * h.y),
    ];
    for (center, u, v) in faces {
        add_quad(&mut mesh, center, u, v, 1.0);
    }
    mesh
}

/// A UV sphere centered on the origin, poles on the Z axis.
pub fn sphere(radius: f32, segments: u32, rings: u32) -> MeshData {
    let mut mesh = MeshData::default();
    for ring in 0..=rings {
        let theta = PI * ring as f32 / rings as f32;
        for segment in 0..=segments {
            let phi = TAU * segment as f32 / segments as f32;
            let normal = Vec3::new(theta.sin() * phi.cos(), theta.sin() * phi.sin(), theta.cos());
            push(&mut mesh, normal * radius, normal, Vec2::new(segment as f32 / segments as f32, ring as f32 / rings as f32));
        }
    }
    let row = segments + 1;
    for ring in 0..rings {
        for segment in 0..segments {
            let a = ring * row + segment;
            let b = a + row;
            mesh.indices.extend_from_slice(&[a, b, b + 1, a, b + 1, a + 1]);
        }
    }
    mesh
}

/// A rectangle around `center` spanning `±u` and `±v`, facing `u × v`.
fn add_quad(mesh: &mut MeshData, center: Vec3, u: Vec3, v: Vec3, uv_repeat: f32) {
    let normal = u.cross(v).normalize();
    let first = mesh.positions.len() as u32;
    for corner in [Vec2::new(-1.0, -1.0), Vec2::new(1.0, -1.0), Vec2::new(1.0, 1.0), Vec2::new(-1.0, 1.0)] {
        push(mesh, center + u * corner.x + v * corner.y, normal, (corner * 0.5 + 0.5) * uv_repeat);
    }
    mesh.indices.extend_from_slice(&[first, first + 1, first + 2, first, first + 2, first + 3]);
}

fn push(mesh: &mut MeshData, position: Vec3, normal: Vec3, uv: Vec2) {
    let tangent = normal.any_orthonormal_vector();
    mesh.positions.push(position.into());
    mesh.attributes.push(VertexAttributes {
        normal: oct_encode(normal),
        tangent: [snorm16(tangent.x), snorm16(tangent.y), snorm16(tangent.z), i16::MAX],
        uv: uv.into(),
        color: [255; 4],
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every triangle must wind counter-clockwise seen from outside, i.e. its
    /// geometric normal must point away from the center.
    fn assert_outward(mesh: &MeshData) {
        for tri in mesh.indices.chunks_exact(3) {
            let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| Vec3::from(mesh.positions[i as usize]));
            let normal = (b - a).cross(c - a);
            if normal.length_squared() > 1e-8 {
                assert!(normal.dot(a + b + c) > 0.0, "inward triangle {tri:?}");
            }
        }
    }

    #[test]
    fn cuboid_faces_outward() {
        assert_outward(&cuboid(Vec3::new(1.0, 2.0, 3.0)));
    }

    #[test]
    fn sphere_faces_outward() {
        assert_outward(&sphere(1.0, 16, 8));
    }

    #[test]
    fn plane_faces_up() {
        let mesh = plane(2.0, 1.0);
        let [a, b, c] = [0, 1, 2].map(|i| Vec3::from(mesh.positions[i]));
        assert!((b - a).cross(c - a).z > 0.0);
    }
}
