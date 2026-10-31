//! Prints a texture's size, frames and per-channel averages, to check how a
//! shader should read it (e.g. what a normal map keeps in its alpha).
//!
//! cargo run -p refract-formats --example texstats -- dev/water_normal [map]

use std::path::Path;

use refract_formats::material::load_texture_frames;
use refract_formats::{Steam, bsp, mount_default};

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let name = std::env::args().nth(1).unwrap_or_else(|| "dev/water_normal".into());
    let mut fs = mount_default(Path::new("game"), Steam::locate().as_ref());
    if let Some(map) = std::env::args().nth(2) {
        bsp::load_map(&mut fs, &map)?;
    }
    let frames = load_texture_frames(&fs, &name, 256);
    let Some(first) = frames.first() else { anyhow::bail!("{name} not found") };
    println!("{name}: {}x{}, {} frames, {} mips", first.width, first.height, frames.len(), first.mips.len());
    let pixels = &first.mips[0];
    let count = (pixels.len() / 4) as f64;
    let mut sum = [0.0f64; 4];
    let (mut min, mut max) = ([255u8; 4], [0u8; 4]);
    for p in pixels.chunks_exact(4) {
        for c in 0..4 {
            sum[c] += p[c] as f64;
            min[c] = min[c].min(p[c]);
            max[c] = max[c].max(p[c]);
        }
    }
    let mean: Vec<String> = sum.iter().map(|s| format!("{:.1}", s / count)).collect();
    println!("mean rgba {mean:?}, min {min:?}, max {max:?}");
    Ok(())
}
