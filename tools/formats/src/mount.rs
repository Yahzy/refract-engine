//! The default search path layout: our own game folder first, then the
//! content of installed Steam games listed in `<game>/mount.toml`.
//!
//! ```toml
//! [[game]]
//! steam = "Some Game"                         # folder in steamapps/common
//! content = ["base/pak01_dir.vpk", "base"]    # VPK archives and loose folders, in search order
//! addons = "base/addons"                      # addon folders and .gma archives
//! workshop = 123456                           # app id of the Workshop items to mount (.gma)
//! ```

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::fs::FileSystem;
use crate::steam::Steam;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MountConfig {
    #[serde(default, rename = "game")]
    pub games: Vec<MountedGame>,
}

/// An installed game whose content is read.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MountedGame {
    /// Its folder in `steamapps/common`.
    pub steam: String,
    /// VPK archives (`.vpk`) and loose folders, relative to the game folder.
    #[serde(default)]
    pub content: Vec<String>,
    /// Folder of addons: subfolders and `.gma` archives.
    pub addons: Option<String>,
    /// Steam app id whose subscribed Workshop items are mounted.
    pub workshop: Option<u32>,
}

impl MountConfig {
    /// `<game_dir>/mount.toml`; empty when missing or invalid.
    pub fn load(game_dir: &Path) -> Self {
        let path = game_dir.join("mount.toml");
        let Ok(text) = std::fs::read_to_string(&path) else {
            log::warn!("no {}: only local content is mounted", path.display());
            return Self::default();
        };
        toml::from_str(&text).unwrap_or_else(|err| {
            log::warn!("{}: {err}", path.display());
            Self::default()
        })
    }
}

/// Builds the standard filesystem:
///
/// 1. `game_dir`, our own folder (gamemodes, addons, lua): `GAME+MOD+LUA`
/// 2. `game_dir/download`, content downloaded from servers: `GAME+DOWNLOAD`
/// 3. every mounted game's addons and Workshop items: `GAME+ADDON`
/// 4. every mounted game's content: `GAME+MOUNT`
///
/// Only our own folder is tagged `LUA`: scripts found elsewhere are never run.
pub fn mount_default(game_dir: &Path, steam: Option<&Steam>) -> FileSystem {
    let mut fs = FileSystem::new();
    fs.add_dir(game_dir, "GAME+MOD+LUA");
    fs.add_dir(game_dir.join("download"), "GAME+DOWNLOAD");

    let config = MountConfig::load(game_dir);
    if config.games.is_empty() {
        return fs;
    }
    let Some(steam) = steam else {
        log::warn!("Steam not found: only local content is mounted");
        return fs;
    };
    let games: Vec<(&MountedGame, PathBuf)> = config
        .games
        .iter()
        .filter_map(|game| {
            let root = steam.find_game(&game.steam);
            if root.is_none() {
                log::warn!("{} is not installed: its content is missing", game.steam);
            }
            Some((game, root?))
        })
        .collect();

    for (game, root) in &games {
        mount_addons(&mut fs, steam, game, root);
    }
    for (game, root) in &games {
        for entry in &game.content {
            let path = root.join(entry);
            if entry.ends_with(".vpk") {
                mount_vpk(&mut fs, &path, "GAME+MOUNT");
            } else {
                fs.add_dir(&path, "GAME+MOUNT");
            }
        }
        log::info!("mounted {}", game.steam);
    }
    fs
}

/// Addon folders and `.gma` files from the game's addon folder, then every
/// subscribed Workshop item. Workshop items stored as LZMA `.bin` are skipped.
fn mount_addons(fs: &mut FileSystem, steam: &Steam, game: &MountedGame, root: &Path) {
    if game.addons.is_none() && game.workshop.is_none() {
        return;
    }
    let mut gmas = Vec::new();
    if let Some(addons) = &game.addons {
        for entry in sorted_entries(&root.join(addons)) {
            if entry.is_dir() {
                fs.add_dir(&entry, "GAME+ADDON");
            } else if entry.extension().is_some_and(|e| e == "gma") {
                gmas.push(entry);
            }
        }
    }
    let mut skipped = 0;
    if let Some(app) = game.workshop {
        for library in &steam.libraries {
            let workshop = library.join("steamapps").join("workshop").join("content").join(app.to_string());
            for item in sorted_entries(&workshop) {
                for file in sorted_entries(&item) {
                    match file.extension().and_then(|e| e.to_str()) {
                        Some("gma") => gmas.push(file),
                        Some("bin") => skipped += 1,
                        _ => {}
                    }
                }
            }
        }
    }
    let total = gmas.len();
    for gma in gmas {
        if let Err(err) = fs.add_gma(&gma, "GAME+ADDON") {
            log::warn!("{err:#}");
        }
    }
    log::info!("{}: mounted {total} addons ({skipped} compressed Workshop items skipped)", game.steam);
}

fn sorted_entries(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<_> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
    entries.sort();
    entries
}

fn mount_vpk(fs: &mut FileSystem, path: &Path, ids: &str) {
    if !path.is_file() {
        log::debug!("skipping missing {}", path.display());
        return;
    }
    if let Err(err) = fs.add_vpk(path, ids) {
        log::warn!("{err:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_config() {
        let config: MountConfig = toml::from_str(
            r#"
[[game]]
steam = "A"
content = ["a/pak_dir.vpk", "a"]
addons = "a/addons"
workshop = 42

[[game]]
steam = "B"
"#,
        )
        .unwrap();
        assert_eq!(config.games.len(), 2);
        assert_eq!(config.games[0].content, ["a/pak_dir.vpk", "a"]);
        assert_eq!(config.games[0].workshop, Some(42));
        assert!(config.games[1].addons.is_none());
    }
}
