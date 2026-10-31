//! The repository's layering rule: no engine crate depends on a tool crate.
//! Everything about imported formats stays in the tools.

use std::fs;
use std::path::Path;

fn manifests(dir: &Path) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let manifest = entry.unwrap().path().join("Cargo.toml");
        if let Ok(text) = fs::read_to_string(&manifest) {
            let name = text.lines().find_map(|l| l.strip_prefix("name = \"")).and_then(|l| l.strip_suffix('"')).unwrap().to_string();
            found.push((name, text));
        }
    }
    found
}

#[test]
fn engine_never_depends_on_tools() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tools: Vec<String> = manifests(&root.join("tools")).into_iter().map(|(name, _)| name).collect();
    assert!(!tools.is_empty());
    for (engine_crate, manifest) in manifests(&root.join("engine")) {
        for line in manifest.lines() {
            let dependency = line.split(['=', '.', ' ']).next().unwrap_or("");
            assert!(!tools.iter().any(|tool| tool == dependency), "{engine_crate} depends on the tool crate {dependency}");
            assert!(!line.contains("tools/"), "{engine_crate} points into tools/: {line}");
        }
    }
}
