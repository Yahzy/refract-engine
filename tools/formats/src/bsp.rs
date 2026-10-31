//! Map loading: BSP geometry batched per material and per spatial chunk, plus the entity lump.
//!
//! Everything stays in map space (inch units, Z up); converting to the
//! renderer's space is the client's job.

use std::collections::HashMap;

use anyhow::{Context, Result};
use vbsp::{Bsp, Handle, TextureFlags, Vector};

use crate::ambient::{AmbientCube, LeafAmbient};
use crate::fs::FileSystem;
pub use crate::mesh::MeshBatch;
use crate::path::normalize;

/// A model placed in the map: a `prop_static` from the BSP's game lump, or a
/// prop entity (`prop_dynamic`, `prop_physics`, `prop_door_rotating`...).
#[derive(Debug, Clone)]
pub struct StaticProp {
    /// Normalized model path, e.g. `models/props/fence01.mdl`.
    pub model: String,
    pub origin: [f32; 3],
    /// `pitch yaw roll` in degrees.
    pub angles: [f32; 3],
    pub skin: usize,
    /// Inside the 3D skybox area rather than the playable world.
    pub in_skybox: bool,
    /// Distances (world units) over which the mapper made the prop fade out;
    /// `fade_max` is 0 when it never fades.
    pub fade_min: f32,
    pub fade_max: f32,
    /// `modelscale`.
    pub scale: f32,
    /// `rendercolor` and `renderamt`, 0..1 sRGB.
    pub color: [f32; 4],
}

/// The `sky_camera` entity: the 3D skybox is drawn as seen from `origin`,
/// with the player's position scaled down by `scale`.
#[derive(Debug, Clone, Copy)]
pub struct SkyCamera {
    pub origin: [f32; 3],
    pub scale: f32,
}

/// A key/value entity from the BSP entity lump.
#[derive(Debug, Clone)]
pub struct MapEntity {
    pub classname: String,
    pub keyvalues: Vec<(String, String)>,
}

impl MapEntity {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.keyvalues.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
    }

    /// Parses a `"x y z"` vector keyvalue such as `origin` or `angles`.
    pub fn vector(&self, key: &str) -> Option<[f32; 3]> {
        let mut it = self.get(key)?.split_whitespace().map(|s| s.parse::<f32>().ok());
        Some([it.next()??, it.next()??, it.next()??])
    }
}

pub struct Map {
    pub name: String,
    /// Playable world geometry.
    pub batches: Vec<MeshBatch>,
    /// 3D skybox geometry, drawn behind the world through [`Map::sky_camera`].
    pub sky_batches: Vec<MeshBatch>,
    pub sky_camera: Option<SkyCamera>,
    pub entities: Vec<MapEntity>,
    pub static_props: Vec<StaticProp>,
    /// Props placed as entities. Until entities run, they are drawn where the map puts them.
    pub entity_props: Vec<StaticProp>,
    /// Hidden sides of solid brushes, which block sunlight without being drawn.
    pub shadow_batch: MeshBatch,
    /// What fills each point of the world (water, solid...).
    pub contents: ContentsTree,
    /// Where the compiler placed `env_cubemap`s, whose pictures are in the pakfile.
    pub cubemaps: Vec<CubemapSample>,
    /// `info_overlay` decals laid on the faces they cover, per material and
    /// chunk; drawn slightly in front of the surface.
    pub overlay_batches: Vec<MeshBatch>,
    /// Baked ambient light samples, per leaf.
    pub ambient: Option<LeafAmbient>,
    /// Changes whenever the map is recompiled: a hash of the BSP header (lump
    /// directory and map revision).
    pub fingerprint: u64,
}

/// An `env_cubemap`: vbsp removes the entity and keeps its origin in the
/// CUBEMAPS lump; `buildcubemaps` stores its picture in the pakfile.
#[derive(Debug, Clone, Copy)]
pub struct CubemapSample {
    /// world units, truncated to integers like the texture name.
    pub origin: [i32; 3],
}

impl CubemapSample {
    /// The cubemap texture, e.g. `maps/test_map/c1344_3584_32`.
    pub fn texture(&self, map: &str) -> String {
        let [x, y, z] = self.origin;
        format!("maps/{map}/c{x}_{y}_{z}")
    }
}

/// Leaf contents flag: water.
pub const CONTENTS_WATER: u32 = 0x20;
/// Leaf contents flag: slime, water's toxic sibling.
pub const CONTENTS_SLIME: u32 = 0x10;

/// The world's BSP tree with the contents of every leaf, to ask what is at a point.
#[derive(Debug, Clone, Default)]
pub struct ContentsTree {
    /// Plane normal, distance and (front, back) children; a negative child is `!leaf`.
    nodes: Vec<([f32; 3], f32, [i32; 2])>,
    leaves: Vec<u32>,
}

impl ContentsTree {
    fn new(bsp: &Bsp, leaves: &[RawLeaf]) -> ContentsTree {
        let nodes = bsp
            .nodes
            .iter()
            .map(|n| {
                let plane = &bsp.planes[n.plane_index as usize];
                ([plane.normal.x, plane.normal.y, plane.normal.z], plane.dist, n.children)
            })
            .collect();
        ContentsTree { nodes, leaves: leaves.iter().map(|l| l.contents).collect() }
    }

    /// `CONTENTS_*` flags of the leaf containing `point` (world units); 0 is empty space.
    pub fn contents_at(&self, point: [f32; 3]) -> u32 {
        self.leaf_at(point).and_then(|leaf| self.leaves.get(leaf)).copied().unwrap_or(0)
    }

    /// Index of the leaf containing `point`.
    pub fn leaf_at(&self, point: [f32; 3]) -> Option<usize> {
        let mut node = 0i32;
        while node >= 0 {
            let (normal, dist, children) = self.nodes.get(node as usize)?;
            let dot = point[0] * normal[0] + point[1] * normal[1] + point[2] * normal[2];
            node = if dot < *dist { children[1] } else { children[0] };
        }
        Some(!node as usize)
    }
}

/// Leaf contents flag: solid.
pub const CONTENTS_SOLID: u32 = 0x1;

impl Map {
    /// The baked ambient light nearest to `point` (world units), unless
    /// the point is inside a wall or the map has no ambient lighting.
    pub fn ambient_at(&self, point: [f32; 3]) -> Option<AmbientCube> {
        let leaf = self.contents.leaf_at(point)?;
        if self.contents.contents_at(point) & CONTENTS_SOLID != 0 {
            return None;
        }
        self.ambient.as_ref()?.cube(leaf, point)
    }
}

impl Map {
    pub fn worldspawn(&self) -> Option<&MapEntity> {
        self.find_entities("worldspawn").next()
    }

    pub fn find_entities<'a>(&'a self, classname: &'a str) -> impl Iterator<Item = &'a MapEntity> + 'a {
        self.entities.iter().filter(move |e| e.classname.eq_ignore_ascii_case(classname))
    }
}

/// Brush entities whose geometry is never drawn.
const INVISIBLE_BRUSH_CLASSES: &[&str] = &["trigger_", "func_areaportal", "func_occluder", "func_clip", "func_buyzone"];

/// Loads `maps/<name>.bsp` and mounts its pakfile into `fs`.
pub fn load_map(fs: &mut FileSystem, name: &str) -> Result<Map> {
    let path = format!("maps/{name}.bsp");
    let mut data = fs.read(&path, None).with_context(|| format!("{path} not found"))?;
    sanitize_entity_lump(&mut data);
    let bsp = Bsp::read(&data).with_context(|| format!("parsing {path}"))?;
    fs.mount_map_pak(name, bsp.pack.clone());

    let entities: Vec<MapEntity> = bsp
        .entities
        .iter()
        .map(|raw| {
            let keyvalues: Vec<(String, String)> =
                raw.properties().map(|(k, v)| (k.to_owned(), v.to_owned())).collect();
            let classname = keyvalues
                .iter()
                .find(|(k, _)| k == "classname")
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            MapEntity { classname, keyvalues }
        })
        .collect();

    let sky_camera = entities.iter().find(|e| e.classname == "sky_camera").map(|e| SkyCamera {
        origin: e.vector("origin").unwrap_or([0.0; 3]),
        scale: e.get("scale").and_then(|s| s.parse::<f32>().ok()).filter(|s| *s > 0.0).unwrap_or(16.0),
    });
    let sky = SkyboxArea::new(&bsp, &data, sky_camera);

    let mut builder = GeometryBuilder::default();
    let mut sky_builder = GeometryBuilder::default();
    let models: Vec<_> = bsp.models().collect();
    if let Some(world) = models.first() {
        for face in world.faces() {
            let target = if sky.contains_face(&face) { &mut sky_builder } else { &mut builder };
            target.add_face(&face, [0.0; 3]);
        }
    }
    // Brush entities reference their model as "*N" and are placed at their origin.
    for entity in &entities {
        if INVISIBLE_BRUSH_CLASSES.iter().any(|c| entity.classname.starts_with(c)) {
            continue;
        }
        let Some(index) = entity.get("model").and_then(|m| m.strip_prefix('*')).and_then(|n| n.parse::<usize>().ok())
        else {
            continue;
        };
        let Some(model) = models.get(index) else { continue };
        let origin = entity.vector("origin").unwrap_or([0.0; 3]);
        for face in model.faces() {
            builder.add_face(&face, origin);
        }
    }

    let static_props: Vec<StaticProp> = bsp
        .static_props()
        .map(|prop| StaticProp {
            model: normalize(prop.model()),
            origin: [prop.origin.x, prop.origin.y, prop.origin.z],
            angles: [prop.angles.pitch, prop.angles.yaw, prop.angles.roll],
            skin: prop.skin.max(0) as usize,
            in_skybox: sky.contains(prop.origin + Vector { x: 0.0, y: 0.0, z: 2.0 }),
            fade_min: prop.fade_min_distance.max(0.0),
            fade_max: prop.fade_max_distance.max(0.0),
            scale: 1.0,
            color: [1.0; 4],
        })
        .collect();

    let entity_props: Vec<StaticProp> = entities.iter().filter_map(|e| entity_prop(e, &sky)).collect();

    let batches = builder.finish();
    let sky_batches = sky_builder.finish();
    log::info!(
        "loaded {name}: {} batches, {} triangles ({} in 3D skybox), {} entities, {} static props, {} prop entities",
        batches.len(),
        batches.iter().map(|b| b.indices.len() / 3).sum::<usize>(),
        sky_batches.iter().map(|b| b.indices.len() / 3).sum::<usize>(),
        entities.len(),
        static_props.len(),
        entity_props.len()
    );
    // A probe just outside each side tells which area it faces.
    let world = world_brushes(&bsp, &sky.leaves);
    let shadow_batch = crate::shadow::hidden_brush_sides(&bsp, |brush| world[brush], |center, normal| {
        let probe = Vector { x: center[0] + normal[0] * 2.0, y: center[1] + normal[1] * 2.0, z: center[2] + normal[2] * 2.0 };
        !sky.contains(probe)
    });
    log::debug!("{} shadow-only triangles", shadow_batch.indices.len() / 3);
    let contents = ContentsTree::new(&bsp, &sky.leaves);
    let cubemaps = raw_cubemaps(&data);
    let leaf_bounds: Vec<_> = sky.leaves.iter().map(|l| (l.mins, l.maxs)).collect();
    let ambient = LeafAmbient::read(&data, &leaf_bounds);
    let overlay_batches = build_overlays(&bsp, &data);
    Ok(Map {
        name: name.to_owned(),
        batches,
        sky_batches,
        sky_camera,
        entities,
        static_props,
        entity_props,
        shadow_batch,
        contents,
        cubemaps,
        overlay_batches,
        ambient,
        fingerprint: fnv1a(&data[..data.len().min(HEADER_SIZE)]),
    })
}

/// [`Map::fingerprint`] without loading the map.
pub fn map_fingerprint(fs: &FileSystem, name: &str) -> Option<u64> {
    let data = fs.read(&format!("maps/{name}.bsp"), None)?;
    Some(fnv1a(&data[..data.len().min(HEADER_SIZE)]))
}

/// Identifier, version, 64 lump entries and the map revision.
const HEADER_SIZE: usize = 8 + 64 * 16 + 4;

/// FNV-1a: a hash that stays the same across builds, for cache keys.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, b| (hash ^ *b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// Entity classes drawn as a model.
const PROP_CLASSES: &[&str] = &[
    "prop_dynamic",
    "prop_dynamic_override",
    "prop_dynamic_ornament",
    "prop_physics",
    "prop_physics_override",
    "prop_physics_multiplayer",
    "prop_physics_respawnable",
    "physics_prop",
    "prop_door_rotating",
    "prop_ragdoll",
];
/// `rendermode` "Don't render".
const RENDER_NONE: i32 = 10;

fn entity_prop(entity: &MapEntity, sky: &SkyboxArea) -> Option<StaticProp> {
    if !PROP_CLASSES.iter().any(|c| entity.classname.eq_ignore_ascii_case(c)) {
        return None;
    }
    let model = normalize(entity.get("model")?);
    let number = |key: &str| entity.get(key).and_then(|v| v.trim().parse::<f32>().ok());
    if !model.ends_with(".mdl")
        || number("startdisabled").unwrap_or(0.0) != 0.0
        || number("rendermode").map(|m| m as i32) == Some(RENDER_NONE)
    {
        return None;
    }
    let origin = entity.vector("origin").unwrap_or([0.0; 3]);
    let rgb = entity.vector("rendercolor").unwrap_or([255.0; 3]);
    let alpha = number("renderamt").unwrap_or(255.0);
    Some(StaticProp {
        model,
        origin,
        angles: entity.vector("angles").unwrap_or([0.0; 3]),
        skin: number("skin").unwrap_or(0.0).max(0.0) as usize,
        in_skybox: sky.contains(Vector { x: origin[0], y: origin[1], z: origin[2] + 2.0 }),
        fade_min: number("fademindist").unwrap_or(0.0).max(0.0),
        fade_max: number("fademaxdist").unwrap_or(0.0).max(0.0),
        scale: number("modelscale").filter(|s| *s > 0.0).unwrap_or(1.0),
        color: [rgb[0] / 255.0, rgb[1] / 255.0, rgb[2] / 255.0, (alpha / 255.0).clamp(0.0, 1.0)],
    })
}

/// The 3D skybox is a sealed room: vbsp floods areas through the map's
/// portals, so the skybox always ends up in an area of its own. Everything in
/// the `sky_camera`'s area belongs to the 3D skybox.
///
/// `vbsp` (the crate) sorts its leaves by cluster, which breaks its `leaf_at`,
/// so the tree walk and the per-leaf table are done here from the raw lump.
struct SkyboxArea<'a> {
    bsp: &'a Bsp,
    /// (cluster, area) of each leaf, in file order.
    leaves: Vec<RawLeaf>,
    area: Option<i16>,
}

impl<'a> SkyboxArea<'a> {
    fn new(bsp: &'a Bsp, data: &[u8], camera: Option<SkyCamera>) -> SkyboxArea<'a> {
        let mut sky = SkyboxArea { bsp, leaves: raw_leaves(data), area: None };
        sky.area = camera.and_then(|c| sky.leaf_near(c.origin)).map(|leaf| sky.leaves[leaf].area).filter(|a| *a > 0);
        log::debug!("sky_camera {:?}: area {:?}", camera, sky.area);
        sky
    }

    /// Index of the leaf containing `point`, `None` for solid space.
    fn leaf_at(&self, point: Vector) -> Option<usize> {
        let mut node = 0i32;
        while node >= 0 {
            let Some(n) = self.bsp.nodes.get(node as usize) else { return None };
            let plane = &self.bsp.planes[n.plane_index as usize];
            let dot = point.x * plane.normal.x + point.y * plane.normal.y + point.z * plane.normal.z;
            node = if dot < plane.dist { n.children[1] } else { n.children[0] };
        }
        let leaf = !node as usize;
        self.leaves.get(leaf).filter(|l| l.cluster >= 0).map(|_| leaf)
    }

    /// The camera entity often sits inside a detail brush, which is solid in
    /// the BSP tree: search outwards for the nearest empty leaf.
    fn leaf_near(&self, [x, y, z]: [f32; 3]) -> Option<usize> {
        const OFFSETS: [f32; 9] = [0.0, 8.0, -8.0, 32.0, -32.0, 64.0, -64.0, 128.0, -128.0];
        OFFSETS
            .iter()
            .flat_map(|dz| OFFSETS.iter().flat_map(move |dy| OFFSETS.iter().map(move |dx| (*dx, *dy, *dz))))
            .find_map(|(dx, dy, dz)| self.leaf_at(Vector { x: x + dx, y: y + dy, z: z + dz }))
    }

    fn contains(&self, point: Vector) -> bool {
        self.area.is_some_and(|area| self.leaf_at(point).is_some_and(|leaf| self.leaves[leaf].area == area))
    }

    /// Probes just in front of the face, on its empty side.
    fn contains_face(&self, face: &Handle<'_, vbsp::Face>) -> bool {
        if self.area.is_none() {
            return false;
        }
        let (mut sum, mut count) = (Vector { x: 0.0, y: 0.0, z: 0.0 }, 0.0);
        for vertex in face.vertices() {
            sum = sum + vertex.position;
            count += 1.0;
        }
        if count == 0.0 {
            return false;
        }
        let normal = face.normal();
        let center = Vector { x: sum.x / count, y: sum.y / count, z: sum.z / count };
        self.contains(center + Vector { x: normal.x * 2.0, y: normal.y * 2.0, z: normal.z * 2.0 })
    }
}

/// Edge length of the cubes world geometry is split into, in world units.
/// One mesh per material per cube lets the renderer cull what is off-screen
/// or hidden, instead of drawing a material's faces across the whole map.
const CHUNK_SIZE: f32 = 1024.0;

#[derive(Default)]
struct GeometryBuilder {
    batches: HashMap<(String, [i32; 3]), MeshBatch>,
}

fn chunk_of(points: impl Iterator<Item = Vector>, offset: [f32; 3]) -> [i32; 3] {
    let (mut sum, mut count) = ([0.0f32; 3], 0.0f32);
    for p in points {
        sum = [sum[0] + p.x, sum[1] + p.y, sum[2] + p.z];
        count += 1.0;
    }
    let count = count.max(1.0);
    std::array::from_fn(|i| ((sum[i] / count + offset[i]) / CHUNK_SIZE).floor() as i32)
}

impl GeometryBuilder {
    fn add_face(&mut self, face: &Handle<'_, vbsp::Face>, offset: [f32; 3]) {
        let texture = face.texture();
        if texture.flags.intersects(
            TextureFlags::SKY | TextureFlags::SKY2D | TextureFlags::NODRAW | TextureFlags::TRIGGER
                | TextureFlags::HINT | TextureFlags::SKIP,
        ) {
            return;
        }
        let material = texture.name().to_ascii_lowercase().replace('\\', "/");
        if material.starts_with("tools/") {
            return;
        }

        let plane_normal = face.normal();
        let displacement = face.displacement();
        let positions: Vec<Vector> = match &displacement {
            Some(displacement) => displacement.displaced_vertices().collect(),
            None => face.vertex_positions().collect(),
        };
        let chunk = chunk_of(positions.iter().copied(), offset);
        let batch = self
            .batches
            .entry((material.clone(), chunk))
            .or_insert_with(|| MeshBatch { material, texel_uvs: true, ..Default::default() });
        let uv = |p: Vector| texel_uv(&texture, p);
        if let Some(displacement) = displacement {
            add_displacement(batch, &displacement, plane_normal, offset, uv);
            return;
        }

        let normal = [plane_normal.x, plane_normal.y, plane_normal.z];
        for tri in positions.chunks_exact(3) {
            let mut tri = [tri[0], tri[1], tri[2]];
            // Front faces are counter-clockwise around a normal pointing out of the solid.
            if dot(cross(sub(tri[1], tri[0]), sub(tri[2], tri[0])), plane_normal) < 0.0 {
                tri.swap(1, 2);
            }
            for v in tri {
                batch.indices.push(batch.positions.len() as u32);
                batch.positions.push([v.x + offset[0], v.y + offset[1], v.z + offset[2]]);
                batch.normals.push(normal);
                batch.uvs.push(uv(v));
                batch.blend.push(0.0);
            }
        }
    }

    fn finish(self) -> Vec<MeshBatch> {
        let mut batches: Vec<_> = self.batches.into_iter().filter(|(_, b)| !b.indices.is_empty()).collect();
        batches.sort_by(|a, b| a.0.cmp(&b.0));
        batches.into_iter().map(|(_, b)| b).collect()
    }
}

/// Texture coordinates in texels, to be divided by the material's
/// mapping size when loading the map, not by the size the compiler saw (which
/// is 0 when the compiler could not find the texture).
fn texel_uv(texture: &vbsp::TextureInfo, p: Vector) -> [f32; 2] {
    let [u, v] = [texture.texture_transforms_u, texture.texture_transforms_v]
        .map(|t| t[0] * p.x + t[1] * p.y + t[2] * p.z + t[3]);
    [u, v]
}

/// A leaf of the LEAFS lump, as stored in the file.
#[derive(Debug, Clone, Copy)]
struct RawLeaf {
    contents: u32,
    cluster: i16,
    area: i16,
    mins: [f32; 3],
    maxs: [f32; 3],
    first_brush: u16,
    brush_count: u16,
}

/// Reads every leaf from the LEAFS lump (10), in file order.
fn raw_leaves(data: &[u8]) -> Vec<RawLeaf> {
    let field = |at: usize| data.get(at..at + 4).map(|b| i32::from_le_bytes(b.try_into().unwrap()));
    let header = 8 + 10 * 16;
    let (Some(offset), Some(length), Some(version)) = (field(header), field(header + 4), field(header + 8)) else {
        return Vec::new();
    };
    // Version 0 leaves carry an ambient light cube (56 bytes), version 1 do not (32 bytes).
    let size = if version == 0 { 56 } else { 32 };
    let Some(lump) = data.get(offset.max(0) as usize..(offset.max(0) + length.max(0)) as usize) else {
        return Vec::new();
    };
    // contents: i32, cluster: i16, area:9 + flags:7, mins/maxs: 6 x i16,
    // first/num leaf faces: 2 x u16, first/num leaf brushes: 2 x u16.
    let u16_at = |leaf: &[u8], at: usize| u16::from_le_bytes([leaf[at], leaf[at + 1]]);
    lump.chunks_exact(size)
        .map(|leaf| RawLeaf {
            contents: u32::from_le_bytes([leaf[0], leaf[1], leaf[2], leaf[3]]),
            cluster: u16_at(leaf, 4) as i16,
            area: u16_at(leaf, 6) as i16 & 0x1ff,
            mins: std::array::from_fn(|k| u16_at(leaf, 8 + k * 2) as i16 as f32),
            maxs: std::array::from_fn(|k| u16_at(leaf, 14 + k * 2) as i16 as f32),
            first_brush: u16_at(leaf, 24),
            brush_count: u16_at(leaf, 26),
        })
        .collect()
}

/// Lays every overlay on its faces, batched per material and chunk.
fn build_overlays(bsp: &Bsp, data: &[u8]) -> Vec<MeshBatch> {
    let overlays = crate::overlay::read_overlays(data);
    let mut batches: HashMap<(String, [i32; 3]), MeshBatch> = HashMap::new();
    for overlay in &overlays {
        let Some(info) = usize::try_from(overlay.texinfo).ok().and_then(|i| bsp.texture_info(i)) else { continue };
        let material = info.name().to_ascii_lowercase().replace('\\', "/");
        let surface: Vec<_> = overlay
            .faces
            .iter()
            .filter_map(|f| usize::try_from(*f).ok())
            .flat_map(|f| crate::overlay::face_triangles(bsp, f))
            .collect();
        let [x, y, z] = overlay.origin;
        let chunk = [x, y, z].map(|c| (c / CHUNK_SIZE).floor() as i32);
        let batch = batches
            .entry((material.clone(), chunk))
            .or_insert_with(|| MeshBatch { material, ..Default::default() });
        crate::overlay::build_overlay(overlay, &surface, batch);
    }
    let mut batches: Vec<_> = batches.into_iter().filter(|(_, b)| !b.indices.is_empty()).collect();
    batches.sort_by(|a, b| a.0.cmp(&b.0));
    log::debug!("{} overlays in {} batches", overlays.len(), batches.len());
    batches.into_iter().map(|(_, b)| b).collect()
}

/// Reads the CUBEMAPS lump (42): origin (3 x i32) and size (i32) per sample.
fn raw_cubemaps(data: &[u8]) -> Vec<CubemapSample> {
    let field = |at: usize| data.get(at..at + 4).map(|b| i32::from_le_bytes(b.try_into().unwrap()));
    let header = 8 + 42 * 16;
    let (Some(offset), Some(length)) = (field(header), field(header + 4)) else { return Vec::new() };
    let Some(lump) = data.get(offset.max(0) as usize..(offset.max(0) + length.max(0)) as usize) else {
        return Vec::new();
    };
    lump.chunks_exact(16)
        .map(|c| {
            let i = |at: usize| i32::from_le_bytes(c[at..at + 4].try_into().unwrap());
            CubemapSample { origin: [i(0), i(4), i(8)] }
        })
        .collect()
}

/// Brushes of the world model, found through its BSP tree:
/// brush entities (triggers, doors, invisible walls) do not block light.
fn world_brushes(bsp: &Bsp, leaves: &[RawLeaf]) -> Vec<bool> {
    let mut world = vec![false; bsp.brushes.len()];
    let mut stack = vec![bsp.models.first().map_or(0, |m| m.head_node)];
    while let Some(node) = stack.pop() {
        if node >= 0 {
            if let Some(n) = bsp.nodes.get(node as usize) {
                stack.extend(n.children);
            }
            continue;
        }
        let Some(leaf) = leaves.get((-1 - node) as usize) else { continue };
        let range = leaf.first_brush as usize..leaf.first_brush as usize + leaf.brush_count as usize;
        for leaf_brush in bsp.leaf_brushes.get(range).into_iter().flatten() {
            if let Some(slot) = world.get_mut(leaf_brush.brush as usize) {
                *slot = true;
            }
        }
    }
    world
}

/// Replaces invalid UTF-8 in the entity lump with `?`, in place.
///
/// Maps compiled with Latin-1 text in their entities (common on RP maps)
/// would otherwise be rejected by the parser.
fn sanitize_entity_lump(data: &mut [u8]) {
    // Header: ident, version, then 64 lumps of (offset, length, version, fourCC); lump 0 is entities.
    let read_i32 = |d: &[u8], at: usize| d.get(at..at + 4).map(|b| i32::from_le_bytes(b.try_into().unwrap()));
    let (Some(offset), Some(length)) = (read_i32(data, 8), read_i32(data, 12)) else { return };
    let Some(lump) = data.get_mut(offset.max(0) as usize..(offset.max(0) + length.max(0)) as usize) else { return };
    if lump.starts_with(b"LZMA") {
        return;
    }
    let mut start = 0;
    while let Err(err) = std::str::from_utf8(&lump[start..]) {
        let bad = start + err.valid_up_to();
        let len = err.error_len().unwrap_or(lump.len() - bad);
        lump[bad..bad + len].fill(b'?');
        start = bad + len;
    }
}

/// Adds a displacement as a shared-vertex grid with smooth normals. UVs come
/// from the undisplaced surface, and the per-vertex alpha drives
/// `WorldVertexTransition` blending.
fn add_displacement(
    batch: &mut MeshBatch,
    displacement: &Handle<'_, vbsp::DisplacementInfo>,
    plane_normal: Vector,
    offset: [f32; 3],
    uv: impl Fn(Vector) -> [f32; 2],
) {
    let size = 2usize.pow(displacement.power as u32) + 1;
    let positions: Vec<Vector> = displacement.displaced_vertices().collect();
    let vertices: Vec<_> = displacement.displacement_vertices().collect();
    if positions.len() != size * size || vertices.len() != size * size {
        return;
    }
    let index = |x: usize, y: usize| y * size + x;

    let mut triangles = Vec::with_capacity((size - 1) * (size - 1) * 2);
    for x in 0..size - 1 {
        for y in 0..size - 1 {
            triangles.push([index(x, y), index(x + 1, y), index(x, y + 1)]);
            triangles.push([index(x + 1, y), index(x + 1, y + 1), index(x, y + 1)]);
        }
    }
    // The grid's winding depends on which corner it starts from: orient it once
    // so the surface faces the same way as the base face.
    let facing: f32 = triangles
        .iter()
        .map(|[a, b, c]| dot(cross(sub(positions[*b], positions[*a]), sub(positions[*c], positions[*a])), plane_normal))
        .sum();
    if facing < 0.0 {
        for tri in &mut triangles {
            tri.swap(1, 2);
        }
    }

    // Area-weighted vertex normals.
    let mut normals = vec![[0.0f32; 3]; positions.len()];
    for [a, b, c] in &triangles {
        let n = cross(sub(positions[*b], positions[*a]), sub(positions[*c], positions[*a]));
        for i in [a, b, c] {
            for k in 0..3 {
                normals[*i][k] += n[k];
            }
        }
    }

    let base = batch.positions.len() as u32;
    for (i, p) in positions.iter().enumerate() {
        let undisplaced = *p - vertices[i].displacement();
        batch.positions.push([p.x + offset[0], p.y + offset[1], p.z + offset[2]]);
        let n = normalize3(normals[i]);
        batch.normals.push(if n.iter().any(|c| c.is_nan()) { [plane_normal.x, plane_normal.y, plane_normal.z] } else { n });
        batch.uvs.push(uv(undisplaced));
        batch.blend.push((vertices[i].alpha / 255.0).clamp(0.0, 1.0));
    }
    batch.indices.extend(triangles.iter().flatten().map(|i| base + *i as u32));
}

fn sub(a: Vector, b: Vector) -> [f32; 3] {
    [a.x - b.x, a.y - b.y, a.z - b.z]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn dot(a: [f32; 3], b: Vector) -> f32 {
    a[0] * b.x + a[1] * b.y + a[2] * b.z
}

fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / len, v[1] / len, v[2] / len]
}
