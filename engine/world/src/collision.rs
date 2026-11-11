//! Solid geometry made of convex brushes, and box sweeps through it.
//!
//! A brush is the intersection of half-spaces. Sweeping a box through a
//! brush is the same as sweeping a point through the brush grown by the box
//! (its Minkowski sum); for a convex brush that grown shape is again bounded
//! by planes: the brush's own planes pushed out by the box, plus "bevel"
//! planes along the box's faces and its edges crossed with the brush's
//! edges. Brushes store those bevels, so a sweep is a clip of a segment
//! against a handful of planes.

use glam::{Quat, Vec3, Vec3Swizzles};

use crate::entity::EntityId;

/// How far sweeps stop before a surface: movement never ends exactly on a
/// plane, where rounding could put it inside on the next sweep.
pub const DIST_EPSILON: f32 = 1.0 / 32.0;

/// The points `p` with `normal · p <= dist` are behind the plane (inside).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plane {
    pub normal: Vec3,
    pub dist: f32,
}

impl Plane {
    /// The plane through `point` facing `normal` (normalized here).
    pub fn new(normal: Vec3, point: Vec3) -> Self {
        let normal = normal.normalize();
        Self { normal, dist: normal.dot(point) }
    }

    /// Signed distance from the plane, positive in front.
    pub fn distance(&self, point: Vec3) -> f32 {
        self.normal.dot(point) - self.dist
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    fn around(points: &[Vec3]) -> Self {
        let mut bounds = Self::new(Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for &p in points {
            bounds.min = bounds.min.min(p);
            bounds.max = bounds.max.max(p);
        }
        bounds
    }

    pub fn overlaps(&self, other: &Aabb) -> bool {
        self.min.cmple(other.max).all() && other.min.cmple(self.max).all()
    }
}

/// A convex solid.
#[derive(Clone, Debug)]
pub struct Brush {
    /// The faces' planes, then the bevels.
    planes: Vec<Plane>,
    /// Corners of each face, counter-clockwise seen from outside; one entry
    /// per face plane.
    polygons: Vec<Vec<Vec3>>,
    bounds: Aabb,
}

/// Points closer than this to a plane are on it.
const ON_PLANE: f32 = 0.01;

impl Brush {
    /// The solid behind every plane, or None if it is empty or unbounded.
    /// Planes that touch the solid in less than a face are dropped.
    pub fn new(planes: &[Plane]) -> Option<Self> {
        let vertices = corners(planes);
        if vertices.len() < 4 {
            return None;
        }
        let bounds = Aabb::around(&vertices);
        if (bounds.max - bounds.min).min_element() <= ON_PLANE || bounds.max.max_element() > 1e6 || bounds.min.min_element() < -1e6 {
            return None;
        }
        let mut faces = Vec::new();
        let mut polygons = Vec::new();
        for plane in planes {
            if faces.iter().any(|f: &Plane| f.normal.abs_diff_eq(plane.normal, 1e-4)) {
                continue;
            }
            let polygon = face_polygon(plane, &vertices);
            if polygon.len() >= 3 {
                faces.push(*plane);
                polygons.push(polygon);
            }
        }
        let mut brush = Self { planes: faces, polygons, bounds };
        brush.add_bevels(&vertices);
        Some(brush)
    }

    /// A box between two corners.
    pub fn cuboid(min: Vec3, max: Vec3) -> Self {
        let planes = [
            Plane::new(Vec3::X, max),
            Plane::new(-Vec3::X, min),
            Plane::new(Vec3::Y, max),
            Plane::new(-Vec3::Y, min),
            Plane::new(Vec3::Z, max),
            Plane::new(-Vec3::Z, min),
        ];
        Self::new(&planes).expect("a box with positive size")
    }

    /// A box of `min..max` whose top slopes: it is `max.z` high at `min.x`
    /// and `low` high at `max.x` (a ramp going down along +X when `low <
    /// max.z`). Rotate the result to face other ways.
    pub fn ramp(min: Vec3, max: Vec3, low: f32) -> Self {
        let slope = Vec3::new(max.z - low, 0.0, max.x - min.x);
        let planes = [
            Plane::new(Vec3::X, max),
            Plane::new(-Vec3::X, min),
            Plane::new(Vec3::Y, max),
            Plane::new(-Vec3::Y, min),
            Plane::new(slope, Vec3::new(min.x, min.y, max.z)),
            Plane::new(-Vec3::Z, min),
        ];
        Self::new(&planes).expect("a ramp with positive size")
    }

    /// The faces: their plane and their corners, counter-clockwise seen from outside.
    pub fn faces(&self) -> impl Iterator<Item = (&Plane, &[Vec3])> {
        self.planes.iter().zip(self.polygons.iter().map(Vec::as_slice))
    }

    pub fn bounds(&self) -> Aabb {
        self.bounds
    }

    /// The corners.
    pub fn vertices(&self) -> Vec<Vec3> {
        let mut vertices: Vec<Vec3> = Vec::new();
        for p in self.polygons.iter().flatten() {
            if !vertices.iter().any(|q| q.distance_squared(*p) < ON_PLANE * ON_PLANE) {
                vertices.push(*p);
            }
        }
        vertices
    }

    /// The brush turned by `rotation` then moved by `translation`. Its
    /// bevels are made anew: they follow the world axes.
    pub fn transformed(&self, rotation: Quat, translation: Vec3) -> Self {
        let faces = self.polygons.len();
        let planes = self.planes[..faces]
            .iter()
            .map(|p| {
                let normal = rotation * p.normal;
                Plane { normal, dist: p.dist + normal.dot(translation) }
            })
            .collect();
        let polygons = self.polygons.iter().map(|polygon| polygon.iter().map(|p| rotation * *p + translation).collect()).collect();
        let mut brush = Self { planes, polygons, bounds: self.bounds };
        let vertices = brush.vertices();
        brush.bounds = Aabb::around(&vertices);
        brush.add_bevels(&vertices);
        brush
    }

    /// Adds the planes the brush grown by a box needs and the faces lack:
    /// one per box face direction, and one per box edge crossed with a
    /// brush edge, wherever such a plane only touches the brush.
    fn add_bevels(&mut self, vertices: &[Vec3]) {
        let mut candidates = Vec::new();
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            candidates.push(axis);
            candidates.push(-axis);
        }
        let face_count = self.planes.len();
        for (i, a) in vertices.iter().enumerate() {
            for b in &vertices[i + 1..] {
                // An edge: two corners sharing two faces.
                let shared = self.planes[..face_count].iter().filter(|p| p.distance(*a).abs() < ON_PLANE && p.distance(*b).abs() < ON_PLANE).count();
                if shared < 2 {
                    continue;
                }
                let edge = *b - *a;
                for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                    let normal = edge.cross(axis);
                    if normal.length_squared() > 1e-6 {
                        let normal = normal.normalize();
                        // Only the side where the edge is outermost bounds the brush.
                        for normal in [normal, -normal] {
                            let dist = vertices.iter().map(|v| normal.dot(*v)).fold(f32::MIN, f32::max);
                            if (normal.dot(*a) - dist).abs() < ON_PLANE {
                                candidates.push(normal);
                            }
                        }
                    }
                }
            }
        }
        for normal in candidates {
            if self.planes.iter().any(|p| p.normal.abs_diff_eq(normal, 1e-3)) {
                continue;
            }
            let dist = vertices.iter().map(|v| normal.dot(*v)).fold(f32::MIN, f32::max);
            self.planes.push(Plane { normal, dist });
        }
    }
}

/// Every point where three planes meet that lies behind all of them.
fn corners(planes: &[Plane]) -> Vec<Vec3> {
    let mut points: Vec<Vec3> = Vec::new();
    for (i, a) in planes.iter().enumerate() {
        for (j, b) in planes.iter().enumerate().skip(i + 1) {
            for c in &planes[j + 1..] {
                let det = a.normal.dot(b.normal.cross(c.normal));
                if det.abs() < 1e-6 {
                    continue;
                }
                let point = (b.normal.cross(c.normal) * a.dist + c.normal.cross(a.normal) * b.dist + a.normal.cross(b.normal) * c.dist) / det;
                let inside = planes.iter().all(|p| p.distance(point) <= ON_PLANE);
                if inside && !points.iter().any(|q| q.distance_squared(point) < ON_PLANE * ON_PLANE) {
                    points.push(point);
                }
            }
        }
    }
    points
}

/// The corners on `plane`, sorted counter-clockwise around its normal.
fn face_polygon(plane: &Plane, vertices: &[Vec3]) -> Vec<Vec3> {
    let mut polygon: Vec<Vec3> = vertices.iter().copied().filter(|v| plane.distance(*v).abs() < ON_PLANE).collect();
    if polygon.len() < 3 {
        return polygon;
    }
    let center = polygon.iter().copied().sum::<Vec3>() / polygon.len() as f32;
    let u = plane.normal.any_orthonormal_vector();
    let v = plane.normal.cross(u);
    polygon.sort_by(|a, b| {
        let angle = |p: &Vec3| (*p - center).dot(v).atan2((*p - center).dot(u));
        angle(a).total_cmp(&angle(b))
    });
    polygon
}

/// A box swept by its center-bottom-relative extents: the box at `origin`
/// covers `origin + min ..= origin + max`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hull {
    pub min: Vec3,
    pub max: Vec3,
}

/// Where a sweep stopped.
#[derive(Clone, Copy, Debug)]
pub struct Trace {
    /// Part of the way covered, 0 to 1.
    pub fraction: f32,
    pub end: Vec3,
    /// The surface hit, if the sweep stopped before its end.
    pub normal: Option<Vec3>,
    /// The box started inside a solid.
    pub start_solid: bool,
    /// The box stayed inside a solid the whole way: it cannot move.
    pub all_solid: bool,
    /// The entity hit, if what stopped the sweep is one (and not the level).
    pub entity: Option<EntityId>,
}

/// Something boxes sweep through.
pub trait Solid {
    /// Sweeps `hull` from `start` to `end`, stopping at the first solid.
    fn trace(&self, start: Vec3, end: Vec3, hull: Hull) -> Trace;

    /// Whether `hull` at `position` overlaps a solid.
    fn is_solid(&self, position: Vec3, hull: Hull) -> bool {
        self.trace(position, position, hull).start_solid
    }
}

/// A sweep through several sets of brushes, keeping the nearest hit.
pub struct Sweep {
    start: Vec3,
    end: Vec3,
    hull: Hull,
    swept: Aabb,
    trace: Trace,
}

impl Sweep {
    pub fn new(start: Vec3, end: Vec3, hull: Hull) -> Self {
        let trace = Trace { fraction: 1.0, end, normal: None, start_solid: false, all_solid: false, entity: None };
        let swept = Aabb::new(start.min(end) + hull.min - DIST_EPSILON, start.max(end) + hull.max + DIST_EPSILON);
        Self { start, end, hull, swept, trace }
    }

    /// Clips against a brush, part of `entity` (or of the level).
    pub fn clip(&mut self, brush: &Brush, entity: Option<EntityId>) {
        if self.trace.all_solid || !brush.bounds.overlaps(&self.swept) {
            return;
        }
        let fraction = self.trace.fraction;
        clip(brush, self.start, self.end, self.hull, &mut self.trace);
        if self.trace.fraction < fraction {
            self.trace.entity = entity;
        }
    }

    pub fn finish(mut self) -> Trace {
        if self.trace.fraction < 1.0 {
            self.trace.end = self.start + (self.end - self.start) * self.trace.fraction;
        }
        self.trace
    }
}

/// All the solid geometry of a level.
#[derive(Clone, Debug, Default)]
pub struct Collision {
    brushes: Vec<Brush>,
}

impl Collision {
    pub fn new(brushes: Vec<Brush>) -> Self {
        Self { brushes }
    }

    pub fn brushes(&self) -> &[Brush] {
        &self.brushes
    }

    /// Clips a sweep against every brush.
    pub fn clip(&self, sweep: &mut Sweep) {
        for brush in &self.brushes {
            sweep.clip(brush, None);
        }
    }
}

impl Solid for Collision {
    fn trace(&self, start: Vec3, end: Vec3, hull: Hull) -> Trace {
        let mut sweep = Sweep::new(start, end, hull);
        self.clip(&mut sweep);
        sweep.finish()
    }
}

/// Clips the sweep against one brush grown by the hull.
fn clip(brush: &Brush, start: Vec3, end: Vec3, hull: Hull, trace: &mut Trace) {
    let mut enter = -1.0f32;
    let mut leave = 1.0f32;
    let mut normal = None;
    let mut starts_out = false;
    let mut ends_out = false;
    for plane in &brush.planes {
        // Push the plane out by the box corner that reaches furthest into it.
        let corner = Vec3::select(plane.normal.cmplt(Vec3::ZERO), hull.max, hull.min);
        let dist = plane.dist - plane.normal.dot(corner);
        let d1 = plane.normal.dot(start) - dist;
        let d2 = plane.normal.dot(end) - dist;
        starts_out |= d1 > 0.0;
        ends_out |= d2 > 0.0;
        // In front the whole way, or moving away: no contact with this brush.
        if d1 > 0.0 && (d2 >= DIST_EPSILON || d2 >= d1) {
            return;
        }
        if d1 <= 0.0 && d2 <= 0.0 {
            continue;
        }
        if d1 > d2 {
            // Entering through this plane; stop a little before it.
            let f = (d1 - DIST_EPSILON) / (d1 - d2);
            if f > enter {
                enter = f;
                normal = Some(plane.normal);
            }
        } else {
            let f = (d1 + DIST_EPSILON) / (d1 - d2);
            leave = leave.min(f);
        }
    }
    if !starts_out {
        trace.start_solid = true;
        if !ends_out {
            trace.all_solid = true;
            trace.fraction = 0.0;
            trace.end = start;
        }
        return;
    }
    if enter < leave && enter > -1.0 && enter < trace.fraction {
        trace.fraction = enter.max(0.0);
        trace.normal = normal;
    }
}

/// The horizontal part of a vector.
pub fn horizontal(v: Vec3) -> Vec3 {
    v.xy().extend(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HULL: Hull = Hull { min: Vec3::new(-16.0, -16.0, 0.0), max: Vec3::new(16.0, 16.0, 72.0) };

    fn floor() -> Collision {
        Collision::new(vec![Brush::cuboid(Vec3::new(-512.0, -512.0, -64.0), Vec3::new(512.0, 512.0, 0.0))])
    }

    #[test]
    fn cuboid_has_six_faces_wound_outwards() {
        let brush = Brush::cuboid(Vec3::ZERO, Vec3::ONE * 10.0);
        assert_eq!(brush.faces().count(), 6);
        for (plane, polygon) in brush.faces() {
            assert_eq!(polygon.len(), 4);
            let normal = (polygon[1] - polygon[0]).cross(polygon[2] - polygon[0]);
            assert!(normal.dot(plane.normal) > 0.0);
        }
    }

    #[test]
    fn falling_box_stops_above_the_floor() {
        let trace = floor().trace(Vec3::new(0.0, 0.0, 100.0), Vec3::new(0.0, 0.0, -100.0), HULL);
        assert!(trace.fraction < 1.0);
        assert!((trace.end.z - DIST_EPSILON).abs() < 0.01, "{}", trace.end.z);
        assert_eq!(trace.normal, Some(Vec3::Z));
    }

    #[test]
    fn box_stops_at_a_wall_by_its_side() {
        let wall = Collision::new(vec![Brush::cuboid(Vec3::new(100.0, -64.0, 0.0), Vec3::new(132.0, 64.0, 128.0))]);
        let trace = wall.trace(Vec3::new(0.0, 0.0, 1.0), Vec3::new(200.0, 0.0, 1.0), HULL);
        assert!((trace.end.x - (100.0 - 16.0 - DIST_EPSILON)).abs() < 0.01, "{}", trace.end.x);
    }

    #[test]
    fn start_inside_is_solid() {
        assert!(floor().is_solid(Vec3::new(0.0, 0.0, -1.0), HULL));
        assert!(!floor().is_solid(Vec3::new(0.0, 0.0, 1.0), HULL));
    }

    #[test]
    fn ramp_slope_is_a_face_and_bevels_close_its_edges() {
        let ramp = Brush::ramp(Vec3::ZERO, Vec3::new(100.0, 50.0, 100.0), 0.0);
        assert_eq!(ramp.faces().count(), 5);
        let slope = ramp.faces().find(|(p, _)| p.normal.x > 0.1 && p.normal.z > 0.1).unwrap().0;
        assert!(slope.normal.abs_diff_eq(Vec3::new(1.0, 0.0, 1.0).normalize(), 1e-5));
        // The top edge at x = 0 needs a +Z bevel for boxes coming from above.
        assert!(ramp.planes.iter().any(|p| p.normal == Vec3::Z));
    }

    #[test]
    fn rotated_brush_gets_edge_bevels() {
        // A box turned 45° about Z: sweeping a box along its diagonal edge
        // must stop where the grown shapes touch, not at a phantom corner.
        let r = glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_4);
        let planes: Vec<Plane> = [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y].iter().map(|n| Plane::new(r * *n, r * *n * 50.0)).chain([Plane::new(Vec3::Z, Vec3::Z * 50.0), Plane::new(-Vec3::Z, -Vec3::Z * 50.0)]).collect();
        let brush = Brush::new(&planes).unwrap();
        assert!(brush.planes.iter().any(|p| p.normal.abs_diff_eq(Vec3::X, 1e-4)));
        let world = Collision::new(vec![brush]);
        let hull = Hull { min: Vec3::splat(-8.0), max: Vec3::splat(8.0) };
        // The diamond's corner is at x = 50·√2; the box's face reaches it first.
        let trace = world.trace(Vec3::new(200.0, 0.0, 0.0), Vec3::ZERO, hull);
        let expected = 50.0 * std::f32::consts::SQRT_2 + 8.0;
        assert!((trace.end.x - expected).abs() < 0.1, "{} vs {expected}", trace.end.x);
    }

    #[test]
    fn transformed_brushes_match_brushes_built_in_place() {
        let r = glam::Quat::from_rotation_z(0.6) * glam::Quat::from_rotation_x(0.3);
        let t = Vec3::new(100.0, -20.0, 40.0);
        let moved = Brush::cuboid(Vec3::splat(-10.0), Vec3::splat(10.0)).transformed(r, t);
        let planes: Vec<Plane> = [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z].iter().map(|n| Plane::new(r * *n, t + r * *n * 10.0)).collect();
        let built = Brush::new(&planes).unwrap();
        assert!(moved.bounds.min.abs_diff_eq(built.bounds.min, 1e-3) && moved.bounds.max.abs_diff_eq(built.bounds.max, 1e-3));
        assert_eq!(moved.planes.len(), built.planes.len());
        for plane in &built.planes {
            assert!(moved.planes.iter().any(|p| p.normal.abs_diff_eq(plane.normal, 1e-3) && (p.dist - plane.dist).abs() < 1e-2), "{plane:?}");
        }
    }
}
