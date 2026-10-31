//! Counts the shaders and `$parameters` a map's world and prop materials use,
//! to see which material features matter. With a material name filter, prints
//! the full parameters of the matching materials instead.
//!
//! cargo run -p refract-formats --example matvars -- big_map [filter]

use std::collections::BTreeMap;
use std::path::Path;

use refract_formats::material::load_material;
use refract_formats::model::load_model;
use refract_formats::{Steam, bsp, mount_default};

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("error")).init();
    let name = std::env::args().nth(1).expect("map name");
    let filter = std::env::args().nth(2);
    let mut fs = mount_default(Path::new("game"), Steam::locate().as_ref());
    let map = bsp::load_map(&mut fs, &name)?;

    let mut materials: Vec<String> = map.batches.iter().chain(&map.sky_batches).map(|b| b.material.clone()).collect();
    let mut models: Vec<(String, usize)> =
        map.static_props.iter().chain(&map.entity_props).map(|p| (p.model.clone(), p.skin)).collect();
    models.sort();
    models.dedup();
    for (model, skin) in &models {
        if let Ok(m) = load_model(&fs, model, *skin) {
            materials.extend(m.meshes.iter().map(|b| b.material.clone()));
        }
    }
    materials.sort();
    materials.dedup();

    let mut usage: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for material in &materials {
        let Some(info) = load_material(&fs, material) else {
            println!("MISSING {material}");
            continue;
        };
        if let Some(filter) = &filter {
            if material.contains(filter.as_str()) || info.shader.contains(filter.as_str()) {
                let mut vars: Vec<_> = info.vars.iter().collect();
                vars.sort();
                println!("{material} ({}): {vars:?}", info.shader);
                for proxy in &info.proxies {
                    println!("    proxy {} {:?}", proxy.name, proxy.params);
                }
            }
            continue;
        }
        let shader = usage.entry(info.shader.clone()).or_default();
        *shader.entry("(materials)".into()).or_default() += 1;
        for key in info.vars.keys() {
            *shader.entry(key.clone()).or_default() += 1;
        }
    }
    for (shader, vars) in usage {
        println!("{shader}: {vars:?}");
    }
    Ok(())
}
