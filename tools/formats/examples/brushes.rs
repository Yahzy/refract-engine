//! Lists long, thin solid brushes that have hidden (nodraw) sides: the usual
//! suspects when a shadow appears from nowhere.
//!
//! cargo run -p refract-formats --example brushes -- test_map

use std::path::Path;

use refract_formats::shadow::side_polygon;
use refract_formats::{Steam, mount_default};
use vbsp::{Bsp, BrushFlags, TextureFlags};

fn main() -> anyhow::Result<()> {
    let name = std::env::args().nth(1).expect("map name");
    let fs = mount_default(Path::new("game"), Steam::locate().as_ref());
    let data = fs.read(&format!("maps/{name}.bsp"), None).expect("map not found");
    let bsp = Bsp::read(&data)?;
    for (index, brush) in bsp.brushes.iter().enumerate() {
        if !brush.flags.contains(BrushFlags::SOLID) {
            continue;
        }
        let start = brush.brush_side as usize;
        let sides = &bsp.brush_sides[start..start + brush.num_brush_sides as usize];
        let planes: Vec<([f64; 3], f64)> = sides
            .iter()
            .map(|s| {
                let p = &bsp.planes[s.plane as usize];
                ([p.normal.x as f64, p.normal.y as f64, p.normal.z as f64], p.dist as f64)
            })
            .collect();
        let names: Vec<String> = sides
            .iter()
            .filter_map(|s| usize::try_from(s.texture_info).ok().and_then(|i| bsp.texture_info(i)))
            .filter(|i| !i.flags.contains(TextureFlags::SKY))
            .map(|i| i.name().to_ascii_lowercase())
            .collect();
        if !names.iter().any(|n| n.contains("nodraw")) {
            continue;
        }
        let (mut min, mut max) = ([f64::MAX; 3], [f64::MIN; 3]);
        for i in 0..planes.len() {
            for p in side_polygon(&planes, i) {
                for k in 0..3 {
                    min[k] = min[k].min(p[k]);
                    max[k] = max[k].max(p[k]);
                }
            }
        }
        let size: Vec<f64> = (0..3).map(|k| max[k] - min[k]).collect();
        let longest = size.iter().cloned().fold(0.0, f64::max);
        let thinnest = size.iter().cloned().fold(f64::MAX, f64::min);
        if longest > 1500.0 && thinnest < 48.0 && min[2] > -1000.0 {
            let mut unique = names.clone();
            unique.sort();
            unique.dedup();
            println!("brush {index} min {min:.0?} max {max:.0?} {:?} {unique:?}", brush.flags);
        }
    }
    Ok(())
}
