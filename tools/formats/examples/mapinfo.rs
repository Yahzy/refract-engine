//! Load a map and check that its materials and textures resolve.
//!
//! cargo run -p refract-formats --example mapinfo -- test_map

use std::path::Path;
use std::time::Instant;

use refract_formats::material::{load_material, load_skybox, load_texture};
use refract_formats::model::load_model;
use refract_formats::{Steam, bsp, mount_default};

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let name = std::env::args().nth(1).expect("map name");

    let mut fs = mount_default(Path::new("game"), Steam::locate().as_ref());
    let start = Instant::now();
    let map = bsp::load_map(&mut fs, &name)?;
    println!("geometry in {:?}", start.elapsed());

    let start = Instant::now();
    let (mut missing_vmt, mut missing_vtf) = (Vec::new(), Vec::new());
    for batch in &map.batches {
        match load_material(&fs, &batch.material) {
            None => missing_vmt.push(batch.material.clone()),
            Some(mat) => match mat.base_texture.as_deref().map(|t| (t, load_texture(&fs, t))) {
                Some((_, Some(_))) => {}
                Some((t, None)) => missing_vtf.push(t.to_owned()),
                None => missing_vtf.push(format!("{} (no $basetexture)", batch.material)),
            },
        }
    }
    println!("materials in {:?}", start.elapsed());
    println!(
        "{} batches, {} missing vmt {:?}, {} missing vtf {:?}",
        map.batches.len(),
        missing_vmt.len(),
        missing_vmt,
        missing_vtf.len(),
        missing_vtf
    );
    if let Some(spawn) = map.find_entities("info_player_start").next() {
        println!("info_player_start at {:?}", spawn.vector("origin"));
    }

    let start = Instant::now();
    let mut models: Vec<&str> = map.static_props.iter().map(|p| p.model.as_str()).collect();
    models.sort();
    models.dedup();
    let mut failed = Vec::new();
    let mut triangles = 0;
    for model in &models {
        match load_model(&fs, model, 0) {
            Ok(m) => triangles += m.meshes.iter().map(|b| b.indices.len() / 3).sum::<usize>(),
            Err(err) => failed.push(format!("{err:#}")),
        }
    }
    println!(
        "{} static props, {} unique models ({triangles} triangles) in {:?}, {} failed {:?}",
        map.static_props.len(),
        models.len(),
        start.elapsed(),
        failed.len(),
        failed
    );

    if let Ok(dump) = std::env::var("DUMP_VMT") {
        let text = fs.read(&format!("materials/{dump}.vmt"), None).map(|d| String::from_utf8_lossy(&d).into_owned());
        println!("{dump}.vmt:
{}", text.unwrap_or_else(|| "not found".into()));
        println!("{:#?}", load_material(&fs, &dump));
    }
    if let Ok(name) = std::env::var("AVG_TEXTURE") {
        if let Some(t) = load_texture(&fs, &name) {
            let n = (t.width * t.height) as u64;
            let mut sum = [0u64; 4];
            for px in t.mips[0].chunks(4) {
                for c in 0..4 {
                    sum[c] += px[c] as u64;
                }
            }
            println!("{name}: {}x{} avg rgba {:?}", t.width, t.height, sum.map(|s| s / n));
        }
    }
    if let Ok(point) = std::env::var("CONTENTS") {
        let p: Vec<f32> = point.split_whitespace().filter_map(|v| v.parse().ok()).collect();
        println!("contents at {p:?}: {:#x}", map.contents.contents_at([p[0], p[1], p[2]]));
        println!("ambient at {p:?}: {:?}", map.ambient_at([p[0], p[1], p[2]]));
    }
    if let Ok(filter) = std::env::var("BOUNDS") {
        let overlays = std::env::var("OVERLAYS").is_ok();
        let batches = if overlays { &map.overlay_batches } else { &map.batches };
        for b in batches.iter().filter(|b| b.material.contains(filter.as_str())) {
            let min = b.positions.iter().fold([f32::MAX; 3], |m, p| std::array::from_fn(|i| m[i].min(p[i])));
            let max = b.positions.iter().fold([f32::MIN; 3], |m, p| std::array::from_fn(|i| m[i].max(p[i])));
            println!("{} {min:?} {max:?}", b.material);
        }
    }
    if std::env::var("LIST_MATERIALS").is_ok() {
        for b in &map.batches {
            println!("material {}", b.material);
        }
    }
    println!("sky_camera {:?}", map.sky_camera);
    let filter = std::env::var("PROPS").unwrap_or_default();
    let skybox = std::env::var("SKYBOX_PROPS").is_ok();
    for prop in map.static_props.iter().filter(|p| p.in_skybox == skybox && p.model.contains(filter.as_str())).take(40) {
        println!("prop {} at {:?}", prop.model, prop.origin);
    }
    let skyname = map.worldspawn().and_then(|w| w.get("skyname")).unwrap_or("sky_day01_01");
    println!("skybox {skyname}: {}", if load_skybox(&fs, skyname).is_some() { "ok" } else { "MISSING" });
    for class in ["light_environment", "light", "light_spot", "env_fog_controller"] {
        println!("{class}: {}", map.find_entities(class).count());
    }
    if let Ok(class) = std::env::var("ENTITY") {
        for entity in map.find_entities(&class) {
            println!("{class}: {:?}", entity.keyvalues);
        }
    }
    if let Some(env) = map.find_entities("light_environment").next() {
        println!("{:?}", env.keyvalues);
    }
    Ok(())
}
