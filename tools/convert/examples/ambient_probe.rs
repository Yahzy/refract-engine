//! Prints the ambient light at a point, as the BSP stores it and as the
//! converted map's probe grid holds it.
//!
//! cargo run --release -p refract-convert --example ambient_probe -- big_map -6800 -9300 280

use anyhow::Context;
use refract_asset::{MapFile, MappedFile};
use refract_formats::{Steam, bsp, mount_default};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let name = args.first().context("usage: ambient_probe <map> <x> <y> <z>")?;
    let point: [f32; 3] = [1, 2, 3].map(|i| args.get(i).and_then(|v| v.parse().ok()).unwrap_or(0.0));
    let faces = ["+X", "-X", "+Y", "-Y", "+Z", "-Z"];

    let mut fs = mount_default("game".as_ref(), Steam::locate().as_ref());
    let map = bsp::load_map(&mut fs, name)?;
    println!("BSP leaf {:?}", map.contents.leaf_at(point));
    match map.ambient_at(point) {
        Some(cube) => faces.iter().zip(cube).for_each(|(f, c)| println!("  bsp  {f}: {c:?}")),
        None => println!("  bsp: no sample"),
    }

    let file = MappedFile::open(format!("game/cache/content/maps/{name}.rmap").as_ref())?;
    let converted = MapFile::read(file.bytes())?;
    let (grid, probes) = converted.probes.context("no probes")?;
    let cell = [0, 1, 2].map(|k| ((point[k] - grid.origin[k]) / grid.cell_size).floor() as i64);
    let [rx, ry, rz] = grid.resolution.map(|r| r as i64);
    println!("grid origin {:?} cell {} resolution {:?} -> cell {cell:?}", grid.origin, grid.cell_size, grid.resolution);
    if cell.iter().zip([rx, ry, rz]).any(|(&c, r)| c < 0 || c >= r) {
        println!("  outside the grid");
        return Ok(());
    }
    let probe = probes[(cell[0] + rx * (cell[1] + ry * cell[2])) as usize];
    println!("  flags {}", probe.flags);
    for (face, name) in faces.iter().enumerate() {
        let v = probe.light[face];
        let scale = 2f32.powi((v >> 27) as i32 - 24);
        println!("  probe {name}: {:?}", [v & 511, (v >> 9) & 511, (v >> 18) & 511].map(|m| m as f32 * scale));
    }
    Ok(())
}
