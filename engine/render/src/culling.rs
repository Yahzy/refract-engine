//! Visibility tests: bounding spheres against view frustums.

use glam::{Mat4, Vec3, Vec4};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sphere {
    pub center: Vec3,
    pub radius: f32,
}

impl Sphere {
    /// The smallest sphere around `points` centered on their bounding box.
    pub fn around(points: impl Iterator<Item = Vec3> + Clone) -> Self {
        let (min, max) = points.clone().fold((Vec3::MAX, Vec3::MIN), |(min, max), p| (min.min(p), max.max(p)));
        if min.x > max.x {
            return Self::default();
        }
        let center = (min + max) * 0.5;
        let radius = points.map(|p| p.distance_squared(center)).fold(0.0, f32::max).sqrt();
        Self { center, radius }
    }

    /// This sphere moved by `transform` (translation, rotation, uniform or not scale).
    pub fn transformed(&self, transform: &Mat4) -> Self {
        let scale = transform.x_axis.truncate().length().max(transform.y_axis.truncate().length()).max(transform.z_axis.truncate().length());
        Self { center: transform.transform_point3(self.center), radius: self.radius * scale }
    }
}

/// The planes bounding what a view-projection matrix sees, normals inwards.
pub struct Frustum {
    planes: Vec<Vec4>,
}

impl Frustum {
    /// From a projection with depth in [0, 1]. Planes at infinity (from
    /// infinite projections) are left out.
    pub fn from_matrix(view_proj: &Mat4) -> Self {
        let rows = [view_proj.row(0), view_proj.row(1), view_proj.row(2), view_proj.row(3)];
        let candidates = [rows[3] + rows[0], rows[3] - rows[0], rows[3] + rows[1], rows[3] - rows[1], rows[2], rows[3] - rows[2]];
        let planes = candidates
            .into_iter()
            .filter_map(|plane| {
                let length = plane.truncate().length();
                (length > 1e-6).then(|| plane / length)
            })
            .collect();
        Self { planes }
    }

    pub fn intersects(&self, sphere: &Sphere) -> bool {
        self.planes.iter().all(|plane| plane.truncate().dot(sphere.center) + plane.w >= -sphere.radius)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perspective_frustum() {
        let view = Mat4::look_to_rh(Vec3::ZERO, Vec3::X, Vec3::Z);
        let frustum = Frustum::from_matrix(&(Mat4::perspective_infinite_reverse_rh(1.2, 1.0, 1.0) * view));
        let at = |x, y, z| Sphere { center: Vec3::new(x, y, z), radius: 1.0 };
        assert!(frustum.intersects(&at(100.0, 0.0, 0.0)));
        assert!(frustum.intersects(&at(1e6, 0.0, 0.0)), "no far plane");
        assert!(!frustum.intersects(&at(-10.0, 0.0, 0.0)), "behind");
        assert!(!frustum.intersects(&at(10.0, 50.0, 0.0)), "outside the side");
        assert!(frustum.intersects(&at(10.0, 7.5, 0.0)), "touching the side");
    }

    #[test]
    fn sphere_around_points() {
        let sphere = Sphere::around([Vec3::new(-1.0, 0.0, 0.0), Vec3::new(3.0, 0.0, 0.0)].into_iter());
        assert_eq!(sphere, Sphere { center: Vec3::new(1.0, 0.0, 0.0), radius: 2.0 });
    }
}
