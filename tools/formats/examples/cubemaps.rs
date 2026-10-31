//! Lists a map's cubemap samples and how bright their pictures are (LDR and
//! HDR versions), to check that the map's cubemaps were built.
//!
//! cargo run -p refract-formats --example cubemaps -- big_map

use std::path::Path;

use refract_formats::material::load_cubemap;
use refract_formats::{Steam, bsp, mount_default};

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let name = std::env::args().nth(1).expect("map name");
    let mut fs = mount_default(Path::new("game"), Steam::locate().as_ref());
    let map = bsp::load_map(&mut fs, &name)?;
    println!("{} cubemap samples", map.cubemaps.len());
    for sample in map.cubemaps.iter().take(12) {
        let texture = sample.texture(&name);
        for variant in [texture.clone(), format!("{texture}.hdr")] {
            match load_cubemap(&fs, &variant) {
                Some(faces) => {
                    let pixels = &faces[0].mips[0];
                    let mean: Vec<u32> = (0..3)
                        .map(|c| pixels.chunks_exact(4).map(|p| p[c] as u32).sum::<u32>() / (pixels.len() as u32 / 4))
                        .collect();
                    println!("{variant}: {}px, {} mips, +X mean {mean:?}", faces[0].width, faces[0].mips.len());
                }
                None => println!("{variant}: missing"),
            }
        }
    }
    Ok(())
}
