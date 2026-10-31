//! Prints a model's meshes and how their materials resolve.
//!
//! cargo run -p refract-formats --example modelinfo -- models/props/tree01.mdl [map]

use std::path::Path;

use refract_formats::material::{load_material, load_texture};
use refract_formats::model::load_model;
use refract_formats::{Steam, bsp, mount_default};

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let model = std::env::args().nth(1).unwrap_or_else(|| "models/props/barrel01.mdl".into());
    let mut fs = mount_default(Path::new("game"), Steam::locate().as_ref());
    if let Some(map) = std::env::args().nth(2) {
        bsp::load_map(&mut fs, &map)?;
    }
    let data = load_model(&fs, &model, 0)?;
    println!("{} bounds {:?}", data.path, data.bounds);
    for mesh in &data.meshes {
        let info = load_material(&fs, &mesh.material);
        let texture = info.as_ref().and_then(|i| i.base_texture.clone());
        let decoded = texture.as_deref().and_then(|t| load_texture(&fs, t));
        let alpha = decoded.as_ref().map(|t| {
            let pixels = t.mips[0].chunks(4);
            let n = pixels.len().max(1);
            let opaque = t.mips[0].chunks(4).filter(|p| p[3] >= 128).count();
            format!("{}x{} {}% alpha>=0.5", t.width, t.height, opaque * 100 / n)
        });
        println!(
            "  {} tris  {}  shader={:?} alphatest={:?} translucent={:?} nocull={:?} texture={:?} {:?}",
            mesh.indices.len() / 3,
            mesh.material,
            info.as_ref().map(|i| i.shader.clone()),
            info.as_ref().map(|i| i.alpha_test),
            info.as_ref().map(|i| i.translucent),
            info.as_ref().map(|i| i.no_cull),
            texture,
            alpha
        );
    }
    Ok(())
}
