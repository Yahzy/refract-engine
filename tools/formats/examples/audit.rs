//! Lists what a map's materials and entities use that the renderer may not handle:
//! transparency flags as written in the VMT versus as parsed, material proxies,
//! and entity classes carrying a model.
//!
//! cargo run -p refract-formats --example audit -- big_map

use std::collections::BTreeMap;
use std::path::Path;

use refract_formats::material::load_material;
use refract_formats::model::load_model;
use refract_formats::{Steam, bsp, mount_default};

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let name = std::env::args().nth(1).expect("map name");
    let mut fs = mount_default(Path::new("game"), Steam::locate().as_ref());
    let map = bsp::load_map(&mut fs, &name)?;

    let mut classes: BTreeMap<String, usize> = BTreeMap::new();
    let mut models: Vec<(String, usize)> = map.static_props.iter().map(|p| (p.model.clone(), p.skin)).collect();
    for e in &map.entities {
        if let Some(model) = e.get("model").filter(|m| !m.starts_with('*')) {
            *classes.entry(e.classname.clone()).or_default() += 1;
            models.push((model.to_ascii_lowercase().replace('\\', "/"), e.get("skin").and_then(|s| s.parse().ok()).unwrap_or(0)));
        }
    }
    println!("entity classes with a model: {classes:?}");

    models.sort();
    models.dedup();
    let mut materials: Vec<String> = map.batches.iter().map(|b| b.material.clone()).collect();
    for (model, skin) in &models {
        match load_model(&fs, model, *skin) {
            Ok(m) => materials.extend(m.meshes.iter().map(|b| b.material.clone())),
            Err(err) => println!("MODEL FAIL {err:#}"),
        }
    }
    materials.sort();
    materials.dedup();

    let mut proxies: BTreeMap<String, usize> = BTreeMap::new();
    let mut shaders: BTreeMap<String, usize> = BTreeMap::new();
    for material in &materials {
        let Some(text) = fs.read(&format!("materials/{material}.vmt"), None) else { continue };
        let text = String::from_utf8_lossy(&text).to_ascii_lowercase();
        let info = load_material(&fs, material).unwrap_or_default();
        *shaders.entry(info.shader.clone()).or_default() += 1;
        for line in text.lines() {
            let word = line.trim().trim_matches('"');
            if ["texturescroll", "animatedtexture", "sine", "linearramp", "texturetransform", "playercolor",
                "add", "multiply", "equals", "clamp", "uniformnoise", "matrixrotate", "toggletexture",
                "waterlod", "entityrandom", "currenttime", "frac", "int", "wrapminmax", "subtract", "divide",
                "camo", "selectfirstifnonzero", "lessorequal", "gaussiannoise", "exponential", "abs", "empty"]
                .contains(&word)
            {
                *proxies.entry(word.to_owned()).or_default() += 1;
            }
        }
        let raw = |k: &str| text.contains(&format!("\"{k}\"")) || text.contains(&format!("{k} "));
        let flags = format!(
            "{}{}{}{}",
            if raw("$alphatest") { "A" } else { "-" },
            if raw("$translucent") { "T" } else { "-" },
            if raw("$nocull") { "N" } else { "-" },
            if raw("$additive") { "+" } else { "-" },
        );
        let parsed = format!(
            "{}{}{}",
            if info.alpha_test.is_some() { "A" } else { "-" },
            if info.translucent { "T" } else { "-" },
            if info.no_cull { "N" } else { "-" },
        );
        if flags != "----" {
            println!("{flags} parsed {parsed} {} {material}", info.shader);
        }
    }
    println!("shaders: {shaders:?}");
    println!("proxies: {proxies:?}");
    Ok(())
}
