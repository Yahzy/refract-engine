//! Converts content into the engine's native formats (see `refract-asset`).
//!
//! Conversion is on demand: converting a map converts the materials, textures
//! and models it uses, each once; later maps reuse them. Output paths mirror
//! the content's own: `materials/concrete/floor01.vmt` becomes
//! `materials/concrete/floor01.rmat`, its texture
//! `textures/concrete/floor01.ktx2`.

mod geometry;
mod lights;
mod map;
mod material;
mod model;
mod probes;
mod sky;
mod texture;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use refract_formats::FileSystem;

pub use map::MapOutcome;

/// Bumped whenever the converter's output changes, so that existing content
/// is converted again.
pub const CONVERTER_VERSION: u32 = 14;

pub struct Converter {
    fs: FileSystem,
    out: PathBuf,
    /// Convert even what already exists.
    force: bool,
    /// Output paths converted (or found up to date) during this run.
    done: Mutex<HashSet<String>>,
}

impl Converter {
    /// `out` is the content root the engine reads.
    pub fn new(fs: FileSystem, out: impl Into<PathBuf>, force: bool) -> Self {
        let out = out.into();
        // Content written by another converter version is redone.
        let stamp = out.join("converter.version");
        let force = force || std::fs::read_to_string(&stamp).ok().and_then(|v| v.trim().parse().ok()) != Some(CONVERTER_VERSION);
        Self { fs, out, force, done: Mutex::new(HashSet::new()) }
    }

    pub fn out(&self) -> &Path {
        &self.out
    }

    /// Records that this run's content is in the current format.
    pub fn finish(&self) -> std::io::Result<()> {
        refract_asset::write_atomic(&self.out.join("converter.version"), CONVERTER_VERSION.to_string().as_bytes())
    }

    /// Claims `path` (relative to the output root) for conversion: false if
    /// it was already handled in this run, or exists and need not be redone.
    fn claim(&self, path: &str) -> bool {
        if !self.done.lock().unwrap().insert(path.to_owned()) {
            return false;
        }
        self.force || !self.out.join(path).exists()
    }

    fn write(&self, path: &str, bytes: &[u8]) -> anyhow::Result<()> {
        refract_asset::write_atomic(&self.out.join(path), bytes).map_err(|err| anyhow::anyhow!("writing {path}: {err}"))
    }
}

/// Native path of a material (`concrete/floor01` → `materials/concrete/floor01.rmat`).
pub fn material_path(name: &str) -> String {
    format!("materials/{}.rmat", name.to_ascii_lowercase())
}

/// Native path of a texture (`concrete/floor01` → `textures/concrete/floor01.ktx2`).
pub fn texture_path(name: &str) -> String {
    format!("textures/{}.ktx2", name.to_ascii_lowercase())
}

/// Native path of a model with a skin (`models/a.mdl` → `models/a.rmdl`, `models/a.skin2.rmdl`).
pub fn model_path(model: &str, skin: usize) -> String {
    let base = model.to_ascii_lowercase();
    let base = base.strip_suffix(".mdl").unwrap_or(&base);
    if skin == 0 { format!("{base}.rmdl") } else { format!("{base}.skin{skin}.rmdl") }
}

/// Runs `f` over `items` on all cores, preserving order.
fn parallel_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = items.len().div_ceil(threads).max(1);
    let f = &f;
    std::thread::scope(|scope| {
        let workers: Vec<_> = items.chunks(chunk).map(|chunk| scope.spawn(move || chunk.iter().map(f).collect::<Vec<_>>())).collect();
        workers.into_iter().flat_map(|w| w.join().unwrap()).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_paths() {
        assert_eq!(material_path("Concrete/Floor01"), "materials/concrete/floor01.rmat");
        assert_eq!(texture_path("concrete/floor01"), "textures/concrete/floor01.ktx2");
        assert_eq!(model_path("models/props/barrel01.mdl", 0), "models/props/barrel01.rmdl");
        assert_eq!(model_path("models/props/barrel01.mdl", 2), "models/props/barrel01.skin2.rmdl");
    }
}
