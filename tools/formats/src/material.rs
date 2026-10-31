//! Materials (VMT) and textures (VTF).

use std::collections::HashMap;

use crate::fs::FileSystem;
use crate::keyvalues::{self, Block, Value};
use crate::path::normalize;
use crate::vtf::Vtf;

/// What the renderer needs to know about a material.
#[derive(Debug, Clone, Default)]
pub struct MaterialInfo {
    /// Lowercase shader name, e.g. `lightmappedgeneric`, `water`.
    pub shader: String,
    /// Texture path without `materials/` and `.vtf`, e.g. `concrete/concretefloor001a`.
    pub base_texture: Option<String>,
    /// `$translucent`: alpha blended.
    pub translucent: bool,
    /// `$additive`: added on top of what is behind (glows, beams).
    pub additive: bool,
    /// `$alphatest` with its `$alphatestreference` cutoff.
    pub alpha_test: Option<f32>,
    pub no_cull: bool,
    /// Tangent-space normal map (`$bumpmap`, or `$normalmap` on water).
    pub bump_map: Option<String>,
    /// The bump maps are self-shadowed (`$ssbump`): three directional
    /// occlusion values instead of a normal, see [`Texture::ssbump_to_normal`].
    pub ssbump: bool,
    /// Second texture of `WorldVertexTransition`, blended by displacement vertex alpha.
    pub base_texture2: Option<String>,
    /// Normal map of the second texture (`$bumpmap2`).
    pub bump_map2: Option<String>,
    /// Sharpens the blend between the two textures (`$blendmodulatetexture`).
    pub blend_modulate: Option<String>,
    /// Where the surface is glossy (`$envmapmask`).
    pub envmap_mask: Option<String>,
    /// Where the surface glows (`$selfillummask`), instead of the base alpha.
    pub selfillum_mask: Option<String>,
    pub detail: Option<Detail>,
    /// Has an `$envmap`: the surface is glossy.
    pub envmap: bool,
    /// `$selfillum`: the base texture's alpha masks a glow.
    pub self_illum: bool,
    /// Water `$fogcolor`, linear 0..1 RGB.
    pub fog_color: Option<[f32; 3]>,
    /// Every `$parameter` of the material, lowercase, as written.
    pub vars: HashMap<String, String>,
    /// Material proxies (`Proxies` block), in order.
    pub proxies: Vec<ProxyDef>,
}

/// One entry of a material's `Proxies` block, e.g. `TextureScroll`.
#[derive(Debug, Clone, Default)]
pub struct ProxyDef {
    /// Lowercase proxy name.
    pub name: String,
    /// Lowercase keys, values as written.
    pub params: HashMap<String, String>,
}

/// A `$detail` texture tiled over the base texture.
#[derive(Debug, Clone)]
pub struct Detail {
    pub texture: String,
    /// UV multiplier relative to the base texture.
    pub scale: f32,
    pub blend_factor: f32,
    /// The VMT's `$detailblendmode`; 0 is "mod2x".
    pub blend_mode: i32,
}

/// An RGBA8 texture with its full mip chain, largest first.
#[derive(Debug, Clone)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub mips: Vec<Vec<u8>>,
}

/// A block-compressed texture as the VTF stores it, for the GPU to sample
/// directly: a quarter to an eighth of the memory of RGBA8.
#[derive(Debug, Clone)]
pub struct CompressedTexture {
    pub width: u32,
    pub height: u32,
    pub format: BlockFormat,
    /// Every mip level, largest first.
    pub mips: Vec<Vec<u8>>,
}

/// GPU block compression formats, by their DirectX names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockFormat {
    /// DXT1, with or without its one-bit alpha.
    Bc1,
    /// DXT3.
    Bc2,
    /// DXT5.
    Bc3,
}

/// Default `$alphatestreference`.
const ALPHA_TEST_REFERENCE: f32 = 0.5;

/// Loads `materials/<name>.vmt`, following `patch` materials to their include.
pub fn load_material(fs: &FileSystem, name: &str) -> Option<MaterialInfo> {
    let (shader, body) = load_vmt(fs, &material_path(name, "vmt"), 0)?;
    let mut vars: HashMap<String, String> = HashMap::new();
    for (key, value) in body.strings() {
        vars.insert(key.to_ascii_lowercase(), value.to_owned());
    }
    let proxies = body
        .get_block("proxies")
        .map(|proxies| {
            proxies
                .blocks()
                .map(|(name, params)| ProxyDef {
                    name: name.to_ascii_lowercase(),
                    params: params.strings().map(|(k, v)| (k.to_ascii_lowercase(), v.to_owned())).collect(),
                })
                .collect()
        })
        .unwrap_or_default();
    Some(material_info(shader, vars, proxies))
}

/// Reads a VMT as (lowercase shader, parameters), resolving `patch` materials:
/// the included material's parameters, overridden by `insert` and `replace`.
fn load_vmt(fs: &FileSystem, path: &str, depth: u32) -> Option<(String, Block)> {
    let text = read_text(fs, path)?;
    let root = keyvalues::parse(&text);
    let (shader, body) = root.0.into_iter().find_map(|(k, v)| match v {
        Value::Block(b) => Some((k.to_ascii_lowercase(), b)),
        Value::Str(_) => None,
    })?;
    if shader != "patch" {
        return Some((shader, body));
    }
    let Some(include) = body.get_str("include").filter(|_| depth < 8) else {
        log::warn!("{path}: patch without a usable include");
        return None;
    };
    let Some((shader, mut base)) = load_vmt(fs, include, depth + 1) else {
        log::warn!("{path}: included material {include} not found");
        return None;
    };
    for section in ["insert", "replace"] {
        if let Some(changes) = body.get_block(section) {
            base.0.extend(changes.0.iter().cloned());
        }
    }
    Some((shader, base))
}

fn material_info(shader: String, vars: HashMap<String, String>, proxies: Vec<ProxyDef>) -> MaterialInfo {
    let texture = |name: &str| vars.get(name).filter(|t| !t.is_empty()).map(|t| strip_material_path(t, "vtf"));
    let number = |name: &str| vars.get(name).and_then(|v| parse_number(v));
    let flag = |name: &str| number(name).is_some_and(|v| v != 0.0);

    // Water's $bumpmap is a DuDv distortion map; its normals are in $normalmap.
    let bump_map = if shader == "water" { texture("$normalmap") } else { texture("$bumpmap").or_else(|| texture("$normalmap")) };
    MaterialInfo {
        base_texture: texture("$basetexture"),
        translucent: flag("$translucent") || (flag("$vertexalpha") && !flag("$alphatest")),
        additive: flag("$additive"),
        alpha_test: flag("$alphatest")
            .then(|| number("$alphatestreference").filter(|r| *r > 0.0).unwrap_or(ALPHA_TEST_REFERENCE)),
        no_cull: flag("$nocull"),
        bump_map,
        ssbump: flag("$ssbump"),
        base_texture2: texture("$basetexture2"),
        bump_map2: texture("$bumpmap2"),
        blend_modulate: texture("$blendmodulatetexture"),
        envmap_mask: texture("$envmapmask"),
        selfillum_mask: texture("$selfillummask"),
        detail: texture("$detail").map(|texture| Detail {
            texture,
            scale: number("$detailscale").unwrap_or(4.0),
            blend_factor: number("$detailblendfactor").unwrap_or(1.0),
            blend_mode: number("$detailblendmode").unwrap_or(0.0) as i32,
        }),
        envmap: vars.get("$envmap").is_some_and(|v| !v.is_empty()),
        self_illum: flag("$selfillum"),
        fog_color: vars.get("$fogcolor").and_then(|v| parse_color(v)),
        shader,
        vars,
        proxies,
    }
}

/// A number, tolerating the vector form `[0.5]` and trailing junk like `1.0f`.
pub fn parse_number(value: &str) -> Option<f32> {
    let v = value.trim().trim_matches(|c| c == '[' || c == ']' || c == '{' || c == '}').trim();
    let end = v.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))).unwrap_or(v.len());
    v[..end].parse().ok()
}

/// Parses `{255 128 0}` (0-255, sRGB) or `[1 0.5 0]` (0-1) into linear RGB.
pub fn parse_color(value: &str) -> Option<[f32; 3]> {
    let v = value.trim();
    let bytes = v.starts_with('{');
    let parts: Vec<f32> = v
        .trim_matches(|c| c == '{' || c == '}' || c == '[' || c == ']')
        .split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect();
    let [r, g, b] = match parts.as_slice() {
        [r, g, b, ..] => [*r, *g, *b],
        [x] => [*x, *x, *x],
        _ => return None,
    };
    let to_linear = |c: f32| if bytes { (c / 255.0).powf(2.2) } else { c };
    Some([to_linear(r), to_linear(g), to_linear(b)])
}

/// Mapping size of a material without textures.
const ERROR_TEXTURE_SIZE: u32 = 32;

/// The size a map face's texel coordinates are divided by: that of the
/// material's "representative" texture, the first of `$basetexture`,
/// `$envmapmask`, `$bumpmap`, `$dudvmap` and `$normalmap` it has.
pub fn mapping_size(fs: &FileSystem, info: &MaterialInfo) -> [u32; 2] {
    let representative = ["$basetexture", "$envmapmask", "$bumpmap", "$dudvmap", "$normalmap"]
        .iter()
        .find_map(|key| info.vars.get(*key).filter(|v| !v.trim().is_empty()));
    representative
        .and_then(|name| {
            let data = fs.read(&material_path(name, "vtf"), None)?;
            let vtf = Vtf::parse(&data).ok()?;
            Some([vtf.width, vtf.height])
        })
        .filter(|[w, h]| *w > 0 && *h > 0)
        .unwrap_or([ERROR_TEXTURE_SIZE; 2])
}

/// Loads and decodes `materials/<name>.vtf`.
pub fn load_texture(fs: &FileSystem, name: &str) -> Option<Texture> {
    load_texture_frames(fs, name, 1).into_iter().next()
}

/// Loads up to `max_frames` animation frames of `materials/<name>.vtf`
/// (`$frame`, `AnimatedTexture`), each with its mip chain.
pub fn load_texture_frames(fs: &FileSystem, name: &str, max_frames: u32) -> Vec<Texture> {
    let path = material_path(name, "vtf");
    let Some(data) = fs.read(&path, None) else { return Vec::new() };
    let vtf = match Vtf::parse(&data) {
        Ok(vtf) => vtf,
        Err(err) => {
            log::warn!("{path}: {err:#}");
            return Vec::new();
        }
    };
    (0..vtf.frames.min(max_frames.max(1)))
        .map_while(|frame| match texture_from_vtf(&vtf, frame) {
            Ok(texture) => Some(texture),
            Err(err) => {
                log::warn!("{path}: {err:#}");
                None
            }
        })
        .collect()
}

/// Like [`load_texture_frames`], but keeps DXT textures compressed. `None`
/// when the texture is in another format, lacks a full mip chain, or is not a
/// multiple of 4 texels wide and high (as the GPU requires).
pub fn load_compressed_frames(fs: &FileSystem, name: &str, max_frames: u32) -> Option<Vec<CompressedTexture>> {
    use crate::vtf::Format;
    let data = fs.read(&material_path(name, "vtf"), None)?;
    let vtf = Vtf::parse(&data).ok()?;
    let format = match vtf.format {
        Format::Dxt1 | Format::Dxt1OneBitAlpha => BlockFormat::Bc1,
        Format::Dxt3 => BlockFormat::Bc2,
        Format::Dxt5 => BlockFormat::Bc3,
        _ => return None,
    };
    let (width, height) = vtf.dimensions(0);
    let full_chain = 32 - width.max(height).leading_zeros();
    if vtf.mip_count < full_chain || width % 4 != 0 || height % 4 != 0 {
        return None;
    }
    (0..vtf.frames.min(max_frames.max(1)))
        .map(|frame| {
            let mips = (0..full_chain).map(|mip| vtf.raw(mip, frame, 0).map(<[u8]>::to_vec)).collect::<anyhow::Result<_>>();
            Some(CompressedTexture { width, height, format, mips: mips.ok()? })
        })
        .collect()
}

/// The VTF's own mip chain, as authored (vtex filtering keeps foliage and
/// fences readable at a distance); built here only when the file has none.
fn texture_from_vtf(vtf: &Vtf, frame: u32) -> anyhow::Result<Texture> {
    let (width, height) = vtf.dimensions(0);
    let full_chain = 32 - width.max(height).leading_zeros();
    if vtf.mip_count < full_chain {
        return Ok(Texture { width, height, mips: build_mips(width, height, vtf.decode(0, frame, 0)?) });
    }
    let mips = (0..full_chain).map(|mip| vtf.decode(mip, frame, 0)).collect::<anyhow::Result<_>>()?;
    Ok(Texture { width, height, mips })
}

/// The bump basis: the three tangent-space directions a self-shadowed
/// bump map (and a bumped lightmap) stores light for.
const BUMP_BASIS: [[f32; 3]; 3] = [
    [0.816_496_6, 0.0, 0.577_350_3],
    [-0.408_248_3, std::f32::consts::FRAC_1_SQRT_2, 0.577_350_3],
    [-0.408_248_3, -std::f32::consts::FRAC_1_SQRT_2, 0.577_350_3],
];

impl Texture {
    /// Turns a self-shadowed bump map into a regular tangent-space normal map
    /// (the green channel pointing down, as in VTF normal maps): each channel says how
    /// much light comes from one direction of the bump basis, so their
    /// weighted sum is the direction the surface faces.
    pub fn ssbump_to_normal(&mut self) {
        for mip in &mut self.mips {
            for p in mip.chunks_exact_mut(4) {
                let weights = [p[0], p[1], p[2]].map(|c| c as f32 / 255.0);
                let mut n = [0.0f32; 3];
                for (basis, w) in BUMP_BASIS.iter().zip(weights) {
                    for k in 0..3 {
                        n[k] += basis[k] * w;
                    }
                }
                let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                let n = if length > 1e-4 { n.map(|c| c / length) } else { [0.0, 0.0, 1.0] };
                // The tangent space has T pointing down the texture, the same
                // convention as VTF normal maps.
                p[0] = ((n[0] * 0.5 + 0.5) * 255.0).round() as u8;
                p[1] = ((n[1] * 0.5 + 0.5) * 255.0).round() as u8;
                p[2] = ((n[2] * 0.5 + 0.5) * 255.0).round() as u8;
            }
        }
    }

    /// Scales the alpha of each smaller mip so that the share of pixels passing
    /// the alpha test stays the same as in the full-size image. Without this,
    /// averaging makes alpha-tested leaves and fences thin out with distance.
    pub fn preserve_alpha_coverage(&mut self, reference: f32) {
        let threshold = (reference.clamp(0.0, 1.0) * 255.0) as u8;
        let coverage = |pixels: &[u8], scale: f32| {
            let passing = pixels.chunks_exact(4).filter(|p| (p[3] as f32 * scale).min(255.0) as u8 >= threshold);
            passing.count() as f32 / (pixels.len() / 4).max(1) as f32
        };
        let target = coverage(&self.mips[0], 1.0);
        if target <= 0.0 || target >= 1.0 {
            return;
        }
        for mip in self.mips.iter_mut().skip(1) {
            // Coverage grows with the scale: binary search the one that matches.
            let (mut low, mut high) = (0.0f32, 8.0f32);
            for _ in 0..12 {
                let mid = (low + high) * 0.5;
                if coverage(mip, mid) < target { low = mid } else { high = mid }
            }
            for p in mip.chunks_exact_mut(4) {
                p[3] = (p[3] as f32 * high).min(255.0) as u8;
            }
        }
    }
}

/// Loads a cubemap texture (`$envmap`, `env_cubemap` pictures) as six faces
/// with their mips, in the renderer's cube order and orientation (+X, -X, +Y,
/// -Y, +Z, -Z of a Y-up world sampled with z negated). VTF cubemaps are
/// stored for map (Z-up) directions instead.
pub fn load_cubemap(fs: &FileSystem, name: &str) -> Option<[Texture; 6]> {
    let path = material_path(name, "vtf");
    let data = fs.read(&path, None)?;
    let vtf = Vtf::parse(&data).map_err(|err| log::warn!("{path}: {err:#}")).ok()?;
    if vtf.faces < 6 || vtf.width != vtf.height {
        log::warn!("{path}: not a cubemap");
        return None;
    }
    let size = vtf.width;
    let mip_count = (32 - size.leading_zeros()).min(vtf.mip_count.max(1));
    let source: Vec<Vec<Vec<u8>>> = (0..mip_count)
        .map(|mip| (0..6).map(|face| vtf.decode(mip, 0, face)).collect::<anyhow::Result<Vec<_>>>())
        .collect::<anyhow::Result<_>>()
        .map_err(|err| log::warn!("{path}: {err:#}"))
        .ok()?;
    let faces = std::array::from_fn(|face| {
        let mips = source
            .iter()
            .enumerate()
            .map(|(mip, faces)| {
                let n = (size >> mip).max(1);
                let mut out = Vec::with_capacity((n * n * 4) as usize);
                for y in 0..n {
                    for x in 0..n {
                        let [cx, cy, cz] = cube_direction(face, x, y, n);
                        // Renderer cube direction -> map direction.
                        let (src_face, sx, sy) = cube_texel([cz, -cx, cy], n);
                        let i = ((sy * n + sx) * 4) as usize;
                        out.extend_from_slice(&faces[src_face][i..i + 4]);
                    }
                }
                out
            })
            .collect();
        Texture { width: size, height: size, mips }
    });
    Some(faces)
}

/// Direction through the centre of texel (x, y) of a `size`-wide cube face
/// (Direct3D/Vulkan convention: faces +X, -X, +Y, -Y, +Z, -Z).
fn cube_direction(face: usize, x: u32, y: u32, size: u32) -> [f32; 3] {
    let s = 2.0 * (x as f32 + 0.5) / size as f32 - 1.0;
    let t = 2.0 * (y as f32 + 0.5) / size as f32 - 1.0;
    match face {
        0 => [1.0, -t, -s],
        1 => [-1.0, -t, s],
        2 => [s, 1.0, t],
        3 => [s, -1.0, -t],
        4 => [s, -t, 1.0],
        _ => [-s, -t, -1.0],
    }
}

/// The face and texel a direction hits, the inverse of [`cube_direction`].
fn cube_texel([x, y, z]: [f32; 3], size: u32) -> (usize, u32, u32) {
    let (ax, ay, az) = (x.abs(), y.abs(), z.abs());
    let (face, s, t, major) = if ax >= ay && ax >= az {
        if x > 0.0 { (0, -z, -y, ax) } else { (1, z, -y, ax) }
    } else if ay >= az {
        if y > 0.0 { (2, x, z, ay) } else { (3, x, -z, ay) }
    } else if z > 0.0 {
        (4, x, -y, az)
    } else {
        (5, -x, -y, az)
    };
    let texel = |c: f32| (((c / major + 1.0) * 0.5 * size as f32) as u32).min(size - 1);
    (face, texel(s), texel(t))
}

/// Suffixes of the six skybox faces, in cubemap layer order (+X, -X, +Y, -Y, +Z, -Z)
/// for a Y-up, -Z-forward renderer: `ft` looks down +X, `rt` down -Y.
pub const SKY_FACES: [&str; 6] = ["rt", "lf", "up", "dn", "bk", "ft"];

/// Loads the six faces of `materials/skybox/<name>XX.vmt`, resized to a common size.
pub fn load_skybox(fs: &FileSystem, name: &str) -> Option<[Texture; 6]> {
    let faces: Vec<Texture> = SKY_FACES
        .iter()
        .map(|face| {
            let material = load_material(fs, &format!("skybox/{name}{face}"))?;
            load_texture(fs, material.base_texture.as_deref()?)
        })
        .collect::<Option<_>>()?;
    let size = faces.iter().map(|f| f.width.max(f.height)).max()?;
    let faces: Vec<Texture> = faces.into_iter().map(|f| f.resized(size, size)).collect();
    faces.try_into().ok()
}

impl Texture {
    /// Nearest-neighbour resize of the top mip; the result has a single mip level.
    pub fn resized(self, width: u32, height: u32) -> Texture {
        if self.width == width && self.height == height {
            return Texture { mips: vec![self.mips.into_iter().next().unwrap()], ..self };
        }
        let src = &self.mips[0];
        let mut data = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            let sy = (y * self.height / height).min(self.height - 1);
            for x in 0..width {
                let sx = (x * self.width / width).min(self.width - 1);
                let i = ((sy * self.width + sx) * 4) as usize;
                data.extend_from_slice(&src[i..i + 4]);
            }
        }
        Texture { width, height, mips: vec![data] }
    }
}

fn material_path(name: &str, ext: &str) -> String {
    format!("materials/{}.{ext}", strip_material_path(name, ext))
}

/// `Materials\Foo/Bar.VTF` -> `foo/bar`
fn strip_material_path(name: &str, ext: &str) -> String {
    let name = normalize(name);
    let name = name.strip_prefix("materials/").unwrap_or(&name);
    let name = name.strip_suffix(&format!(".{ext}")).unwrap_or(name);
    name.to_owned()
}

fn read_text(fs: &FileSystem, path: &str) -> Option<String> {
    let path = if path.ends_with(".vmt") { normalize(path) } else { material_path(path, "vmt") };
    let path = if path.starts_with("materials/") { path } else { format!("materials/{path}") };
    let data = fs.read(&path, None)?;
    let text = String::from_utf8_lossy(&data);
    Some(text.trim_start_matches('\u{feff}').to_owned())
}

/// Box-filtered mip chain down to 1x1.
fn build_mips(mut width: u32, mut height: u32, base: Vec<u8>) -> Vec<Vec<u8>> {
    let mut mips = vec![base];
    while width > 1 || height > 1 {
        let (nw, nh) = ((width / 2).max(1), (height / 2).max(1));
        let src = mips.last().unwrap();
        let mut dst = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                let mut sum = [0u32; 4];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let sx = (x * 2 + dx).min(width - 1);
                    let sy = (y * 2 + dy).min(height - 1);
                    let i = ((sy * width + sx) * 4) as usize;
                    for c in 0..4 {
                        sum[c] += src[i + c] as u32;
                    }
                }
                let o = ((y * nw + x) * 4) as usize;
                for c in 0..4 {
                    dst[o + c] = (sum[c] / 4) as u8;
                }
            }
        }
        mips.push(dst);
        width = nw;
        height = nh;
    }
    mips
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_paths() {
        assert_eq!(strip_material_path("Materials\\Concrete/Floor01.VTF", "vtf"), "concrete/floor01");
        assert_eq!(material_path("tools/toolsnodraw", "vmt"), "materials/tools/toolsnodraw.vmt");
    }

    #[test]
    fn material_flags() {
        let vmt = r#""VertexLitGeneric"
        {
            "$basetexture" "Models/a"
            $alphatest 1
            "$nocull" "1"
            "$detail" "detail\noise"
            "$detailscale" "7.5"
            "$bumpmap" "a_normal"
            //"$translucent" 1
            "$envmap" "env_cubemap"
        }"#;
        let root = keyvalues::parse(vmt);
        let (shader, Value::Block(body)) = &root.0[0] else { panic!() };
        let vars = body.strings().map(|(k, v)| (k.to_ascii_lowercase(), v.to_owned())).collect();
        let info = material_info(shader.to_ascii_lowercase(), vars, Vec::new());
        assert_eq!(info.base_texture.as_deref(), Some("models/a"));
        assert_eq!(info.alpha_test, Some(0.5));
        assert!(info.no_cull && !info.translucent && info.envmap);
        assert_eq!(info.bump_map.as_deref(), Some("a_normal"));
        let detail = info.detail.unwrap();
        assert_eq!((detail.texture.as_str(), detail.scale), ("detail/noise", 7.5));
        assert_eq!(parse_color("[1 0.5 0]"), Some([1.0, 0.5, 0.0]));
        assert_eq!(parse_color("{255 255 255}"), Some([1.0, 1.0, 1.0]));
        assert_eq!(parse_number(".25"), Some(0.25));
        assert_eq!(parse_number("[2]"), Some(2.0));
    }

    #[test]
    fn alpha_coverage() {
        // A 2x2 texture with one opaque pixel: the 1x1 mip averages to 64.
        let mut texture = Texture { width: 2, height: 2, mips: build_mips(2, 2, [[0, 0, 0, 255], [0; 4], [0; 4], [0; 4]].concat()) };
        assert_eq!(texture.mips[1][3], 63);
        texture.preserve_alpha_coverage(0.5);
        // The leaf must not vanish from the smaller mip.
        assert!(texture.mips[1][3] >= 127);
    }

    #[test]
    fn ssbump_flat() {
        // Equal light from the three directions: the surface faces straight out.
        let mut texture = Texture { width: 1, height: 1, mips: vec![vec![128, 128, 128, 255]] };
        texture.ssbump_to_normal();
        assert_eq!(&texture.mips[0][..3], &[128, 128, 255]);
    }

    #[test]
    fn cube_round_trip() {
        for face in 0..6 {
            for (x, y) in [(0, 0), (3, 1), (7, 7)] {
                assert_eq!(cube_texel(cube_direction(face, x, y, 8), 8), (face, x, y));
            }
        }
    }

    #[test]
    fn mip_chain() {
        let mips = build_mips(4, 2, vec![255; 4 * 2 * 4]);
        assert_eq!(mips.len(), 3);
        assert_eq!(mips[1].len(), 2 * 4);
        assert_eq!(mips[2], vec![255; 4]);
    }
}
