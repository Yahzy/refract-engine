//! The engine's native asset formats. The converter writes them; the engine
//! reads them, memory-mapped, without decoding.
//!
//! - Maps (`.rmap`) and models (`.rmdl`) are binary containers: a header and a
//!   table of aligned sections that cast directly to arrays (see [`container`]).
//! - Materials (`.rmat`) are TOML: small, readable, writable by hand.
//! - Textures (`.ktx2`) are standard KTX2 files holding GPU-ready (block
//!   compressed) data.
//!
//! Paths between assets are relative to the content root, with `/`
//! separators, e.g. `textures/concrete/floor01.ktx2`.

pub mod container;
pub mod encoding;
pub mod entities;
pub mod geometry;
pub mod map;
pub mod material;
pub mod model;
pub mod texture;

use std::path::Path;

pub use container::{Container, ContainerWriter};
pub use geometry::{Geometry, Submesh, VertexAttributes};
pub use map::MapFile;
pub use material::Material;
pub use model::ModelFile;
pub use texture::{TextureData, TextureFormat};

#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    #[error("{0}: {1}")]
    Io(String, std::io::Error),
    #[error("not a {expected} file")]
    WrongKind { expected: &'static str },
    #[error("format version {found}, expected {expected}: convert the content again")]
    Version { found: u32, expected: u32 },
    #[error("corrupt file: {0}")]
    Corrupt(&'static str),
    #[error("missing section {0}")]
    MissingSection(&'static str),
    #[error("material: {0}")]
    Material(#[from] toml::de::Error),
    #[error("texture: {0}")]
    Texture(String),
}

/// A file mapped into memory, read in place.
pub struct MappedFile(memmap2::Mmap);

impl MappedFile {
    pub fn open(path: &Path) -> Result<Self, AssetError> {
        let io = |err| AssetError::Io(path.display().to_string(), err);
        let file = std::fs::File::open(path).map_err(io)?;
        // SAFETY: content files are written once by the converter and replaced
        // atomically (written aside, then renamed), never modified in place.
        let map = unsafe { memmap2::Mmap::map(&file) }.map_err(io)?;
        Ok(Self(map))
    }

    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Writes `bytes` to `path` atomically: readers see the old file or the new
/// one, never a partial write.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let partial = path.with_extension("partial");
    std::fs::write(&partial, bytes)?;
    std::fs::rename(&partial, path)
}
