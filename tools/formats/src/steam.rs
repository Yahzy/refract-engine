//! Locates the Steam installation and the games installed in its libraries.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Steam {
    pub root: PathBuf,
    pub libraries: Vec<PathBuf>,
}

impl Steam {
    /// Finds Steam from the registry (Windows) or the usual install paths.
    pub fn locate() -> Option<Steam> {
        let root = steam_root()?;
        let mut libraries = vec![root.clone()];
        let vdf = root.join("steamapps").join("libraryfolders.vdf");
        if let Ok(text) = std::fs::read_to_string(&vdf) {
            for lib in parse_library_paths(&text) {
                if !libraries.iter().any(|l| same_path(l, &lib)) {
                    libraries.push(lib);
                }
            }
        }
        Some(Steam { root, libraries })
    }

    /// Returns `steamapps/common/<folder>` from the first library that has it.
    pub fn find_game(&self, folder: &str) -> Option<PathBuf> {
        self.libraries
            .iter()
            .map(|lib| lib.join("steamapps").join("common").join(folder))
            .find(|p| p.is_dir())
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    let a = a.to_string_lossy().replace('\\', "/").to_lowercase();
    let b = b.to_string_lossy().replace('\\', "/").to_lowercase();
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

/// Extracts every `"path" "..."` value from `libraryfolders.vdf`.
fn parse_library_paths(text: &str) -> Vec<PathBuf> {
    text.lines()
        .filter_map(|line| {
            let mut quoted = line.split('"').skip(1).step_by(2);
            match (quoted.next(), quoted.next()) {
                (Some("path"), Some(value)) => Some(PathBuf::from(value.replace("\\\\", "\\"))),
                _ => None,
            }
        })
        .collect()
}

#[cfg(windows)]
fn steam_root() -> Option<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

    let from_user = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Software\\Valve\\Steam")
        .and_then(|k| k.get_value::<String, _>("SteamPath"));
    let from_machine = || {
        RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey("SOFTWARE\\WOW6432Node\\Valve\\Steam")
            .and_then(|k| k.get_value::<String, _>("InstallPath"))
    };
    let path = from_user.or_else(|_| from_machine()).ok()?;
    Some(PathBuf::from(path)).filter(|p| p.is_dir())
}

#[cfg(not(windows))]
fn steam_root() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    [".steam/steam", ".local/share/Steam", "Library/Application Support/Steam"]
        .iter()
        .map(|p| home.join(p))
        .find(|p| p.join("steamapps").is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_library_folders() {
        let vdf = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"D:\\Programs\\Steam"
		"label"		""
	}
	"1"
	{
		"path"		"E:\\SteamLibrary"
	}
}"#;
        assert_eq!(
            parse_library_paths(vdf),
            vec![PathBuf::from("D:\\Programs\\Steam"), PathBuf::from("E:\\SteamLibrary")]
        );
    }
}
