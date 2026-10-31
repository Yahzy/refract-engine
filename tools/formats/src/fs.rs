//! Virtual filesystem with ordered search paths.
//!
//! Each search path is a loose folder, a VPK archive or a map pakfile, tagged
//! with path ids (`GAME`, `MOD`, …). Lookups walk the search paths in order and
//! the first match wins, so earlier paths override later ones.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::gma::Gma;
use crate::path::{normalize, split, wildcard_match};

/// Files and directories returned by [`FileSystem::find`], sorted and deduplicated.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FindResult {
    pub files: Vec<String>,
    pub dirs: Vec<String>,
}

pub struct FileSystem {
    search_paths: Vec<SearchPath>,
}

struct SearchPath {
    ids: Vec<String>,
    source: Source,
}

enum Source {
    Dir(PathBuf),
    Vpk(VpkArchive),
    Pak(PakArchive),
    Gma(GmaArchive),
}

impl Source {
    fn index(&self) -> Option<&ArchiveIndex> {
        match self {
            Source::Dir(_) => None,
            Source::Vpk(vpk) => Some(&vpk.index),
            Source::Pak(pak) => Some(&pak.index),
            Source::Gma(gma) => Some(&gma.index),
        }
    }
}

/// File and directory listing of an archive, keyed by normalized path.
#[derive(Default)]
struct ArchiveIndex {
    /// Normalized path -> name as stored in the archive.
    files: HashMap<String, String>,
    /// Normalized directory -> (child files, child directories).
    dirs: HashMap<String, (BTreeSet<String>, BTreeSet<String>)>,
}

impl ArchiveIndex {
    fn build<'a>(names: impl Iterator<Item = &'a str>) -> ArchiveIndex {
        let mut index = ArchiveIndex::default();
        for key in names {
            let norm = normalize(key);
            let (dir, name) = split(&norm);
            index.dirs.entry(dir.to_owned()).or_default().0.insert(name.to_owned());
            // Register every ancestor directory so `find("models/*")` lists subfolders.
            let mut child = dir;
            while !child.is_empty() {
                let (parent, child_name) = split(child);
                let inserted = index.dirs.entry(parent.to_owned()).or_default().1.insert(child_name.to_owned());
                if !inserted {
                    break;
                }
                child = parent;
            }
            index.files.insert(norm, key.to_owned());
        }
        index
    }
}

struct VpkArchive {
    path: PathBuf,
    vpk: vpk::VPK,
    index: ArchiveIndex,
}

impl VpkArchive {
    fn open(path: &Path) -> Result<VpkArchive> {
        let vpk = vpk::from_path(path).with_context(|| format!("reading VPK {}", path.display()))?;
        let index = ArchiveIndex::build(vpk.tree.keys().map(String::as_str));
        Ok(VpkArchive { path: path.to_owned(), vpk, index })
    }

    fn read(&self, path: &str) -> Option<Vec<u8>> {
        let key = self.index.files.get(path)?;
        match self.vpk.tree[key].get() {
            Ok(data) => Some(data.into_owned()),
            Err(err) => {
                log::warn!("failed to read {path} from {}: {err}", self.path.display());
                None
            }
        }
    }
}

/// The zip "pakfile" lump embedded in a BSP (custom materials, cubemaps…).
struct PakArchive {
    name: String,
    pack: vbsp::Packfile,
    index: ArchiveIndex,
}

impl PakArchive {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        let key = self.index.files.get(path)?;
        match self.pack.get(key) {
            Ok(data) => data,
            Err(err) => {
                log::warn!("failed to read {path} from {} pakfile: {err}", self.name);
                None
            }
        }
    }
}

/// A Workshop addon (`.gma`).
struct GmaArchive {
    gma: Gma,
    index: ArchiveIndex,
    /// Stored name -> (offset, size).
    entries: HashMap<String, (u64, u64)>,
}

impl GmaArchive {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        let key = self.index.files.get(path)?;
        let (offset, size) = self.entries[key];
        self.gma.read(offset, size).inspect_err(|err| log::warn!("failed to read {path} from {}: {err}", self.gma.path.display())).ok()
    }
}

impl Default for FileSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl FileSystem {
    pub fn new() -> FileSystem {
        FileSystem { search_paths: Vec::new() }
    }

    /// Appends a loose folder. `ids` is a `+`-separated list such as `"GAME+MOD"`.
    pub fn add_dir(&mut self, dir: impl Into<PathBuf>, ids: &str) {
        let dir = dir.into();
        log::debug!("mount dir {} [{ids}]", dir.display());
        self.search_paths.push(SearchPath { ids: parse_ids(ids), source: Source::Dir(dir) });
    }

    /// Appends a VPK archive; pass the `_dir.vpk` file.
    pub fn add_vpk(&mut self, file: impl AsRef<Path>, ids: &str) -> Result<()> {
        let archive = VpkArchive::open(file.as_ref())?;
        log::debug!("mount vpk {} ({} files) [{ids}]", file.as_ref().display(), archive.index.files.len());
        self.search_paths.push(SearchPath { ids: parse_ids(ids), source: Source::Vpk(archive) });
        Ok(())
    }

    /// Appends a Workshop addon archive (`.gma`).
    pub fn add_gma(&mut self, file: impl AsRef<Path>, ids: &str) -> Result<()> {
        let gma = Gma::open(file.as_ref())?;
        let index = ArchiveIndex::build(gma.files.iter().map(|(name, _, _)| name.as_str()));
        let entries = gma.files.iter().map(|(name, offset, size)| (name.clone(), (*offset, *size))).collect();
        log::debug!("mount gma {} \"{}\" ({} files) [{ids}]", file.as_ref().display(), gma.name, index.files.len());
        self.search_paths.push(SearchPath { ids: parse_ids(ids), source: Source::Gma(GmaArchive { gma, index, entries }) });
        Ok(())
    }

    /// Mounts a map's embedded pakfile with the highest priority, tagged `GAME+MAP`.
    /// Any previously mounted map pakfile is removed first.
    pub fn mount_map_pak(&mut self, map_name: &str, pack: vbsp::Packfile) {
        self.unmount("MAP");
        let names: Vec<String> = match pack.clone().into_zip().into_inner() {
            Ok(zip) => zip.file_names().map(str::to_owned).collect(),
            Err(_) => Vec::new(),
        };
        let index = ArchiveIndex::build(names.iter().map(String::as_str));
        log::debug!("mount {map_name} pakfile ({} files)", index.files.len());
        let source = Source::Pak(PakArchive { name: map_name.to_owned(), pack, index });
        self.search_paths.insert(0, SearchPath { ids: parse_ids("GAME+MAP"), source });
    }

    /// Removes every search path tagged with `id`.
    pub fn unmount(&mut self, id: &str) {
        self.search_paths.retain(|sp| !sp.ids.iter().any(|i| i.eq_ignore_ascii_case(id)));
    }

    /// Human-readable list of mounted search paths, in priority order.
    pub fn search_paths(&self) -> Vec<String> {
        self.search_paths
            .iter()
            .map(|sp| {
                let what = match &sp.source {
                    Source::Dir(p) => format!("dir {}", p.display()),
                    Source::Vpk(v) => format!("vpk {} ({} files)", v.path.display(), v.index.files.len()),
                    Source::Pak(p) => format!("pakfile of {} ({} files)", p.name, p.index.files.len()),
                    Source::Gma(g) => format!("gma \"{}\" ({} files)", g.gma.name, g.index.files.len()),
                };
                format!("[{}] {what}", sp.ids.join("+"))
            })
            .collect()
    }

    fn paths_for<'a>(&'a self, id: Option<&'a str>) -> impl Iterator<Item = &'a SearchPath> + 'a {
        self.search_paths
            .iter()
            .filter(move |sp| id.is_none_or(|id| sp.ids.iter().any(|i| i.eq_ignore_ascii_case(id))))
    }

    /// Reads a whole file. `id` restricts the lookup to one path id, `None` searches everything.
    pub fn read(&self, path: &str, id: Option<&str>) -> Option<Vec<u8>> {
        let path = normalize(path);
        self.paths_for(id).find_map(|sp| match &sp.source {
            Source::Dir(dir) => std::fs::read(dir.join(&path)).ok(),
            Source::Vpk(vpk) => vpk.read(&path),
            Source::Pak(pak) => pak.read(&path),
            Source::Gma(gma) => gma.read(&path),
        })
    }

    pub fn exists(&self, path: &str, id: Option<&str>) -> bool {
        let path = normalize(path);
        self.paths_for(id).any(|sp| match (&sp.source, sp.source.index()) {
            (Source::Dir(dir), _) => dir.join(&path).exists(),
            (_, Some(index)) => index.files.contains_key(&path) || index.dirs.contains_key(&path),
            (_, None) => false,
        })
    }

    /// Returns the on-disk location of a loose file, if the first match is not inside an archive.
    pub fn disk_path(&self, path: &str, id: Option<&str>) -> Option<PathBuf> {
        let path = normalize(path);
        for sp in self.paths_for(id) {
            match (&sp.source, sp.source.index()) {
                (Source::Dir(dir), _) => {
                    let full = dir.join(&path);
                    if full.is_file() {
                        return Some(full);
                    }
                }
                (_, Some(index)) if index.files.contains_key(&path) => return None,
                _ => {}
            }
        }
        None
    }

    /// Lists files and directories matching a pattern whose wildcards are in
    /// the last component only, e.g. `"maps/*.bsp"` (same rules as `file.Find`).
    pub fn find(&self, pattern: &str, id: Option<&str>) -> FindResult {
        let pattern = normalize(pattern);
        let (dir, name_pattern) = split(&pattern);
        let mut files = BTreeSet::new();
        let mut dirs = BTreeSet::new();
        for sp in self.paths_for(id) {
            match (&sp.source, sp.source.index()) {
                (Source::Dir(root), _) => {
                    let Ok(entries) = std::fs::read_dir(root.join(dir)) else { continue };
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                        if !wildcard_match(name_pattern, &name) {
                            continue;
                        }
                        match entry.file_type() {
                            Ok(t) if t.is_dir() => dirs.insert(name),
                            Ok(_) => files.insert(name),
                            Err(_) => false,
                        };
                    }
                }
                (_, Some(index)) => {
                    let Some((afiles, adirs)) = index.dirs.get(dir) else { continue };
                    files.extend(afiles.iter().filter(|n| wildcard_match(name_pattern, n)).cloned());
                    dirs.extend(adirs.iter().filter(|n| wildcard_match(name_pattern, n)).cloned());
                }
                (_, None) => {}
            }
        }
        FindResult { files: files.into_iter().collect(), dirs: dirs.into_iter().collect() }
    }
}

fn parse_ids(ids: &str) -> Vec<String> {
    ids.split('+').filter(|s| !s.is_empty()).map(|s| s.to_ascii_uppercase()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_override_order_and_find() {
        let tmp = std::env::temp_dir().join(format!("refract-fs-test-{}", std::process::id()));
        let a = tmp.join("a");
        let b = tmp.join("b");
        std::fs::create_dir_all(a.join("lua/startup")).unwrap();
        std::fs::create_dir_all(b.join("lua/startup")).unwrap();
        std::fs::write(a.join("lua/startup/test.lua"), "print('a')").unwrap();
        std::fs::write(b.join("lua/startup.lua"), "x").unwrap();
        std::fs::write(b.join("lua/startup/test.lua"), "print('b')").unwrap();
        std::fs::write(b.join("lua/startup/other.lua"), "").unwrap();

        let mut fs = FileSystem::new();
        fs.add_dir(&a, "GAME+MOD");
        fs.add_dir(&b, "GAME");

        assert_eq!(fs.read("LUA\\Startup\\test.lua", None).unwrap(), b"print('a')");
        assert_eq!(fs.read("lua/startup/test.lua", Some("game")).unwrap(), b"print('a')");
        assert!(fs.read("lua/startup/other.lua", Some("MOD")).is_none());

        let found = fs.find("lua/startup/*.lua", None);
        assert_eq!(found.files, vec!["other.lua", "test.lua"]);
        let found = fs.find("lua/*", None);
        assert_eq!(found.dirs, vec!["startup"]);
        assert_eq!(found.files, vec!["startup.lua"]);

        fs.unmount("MOD");
        assert_eq!(fs.read("lua/startup/test.lua", None).unwrap(), b"print('b')");

        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
