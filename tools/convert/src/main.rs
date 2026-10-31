//! refract-convert: converts content into the engine's native formats.
//!
//! refract-convert map <name> [--game <dir>] [--out <dir>] [--force]
//!
//! `--game` is the game folder whose content is mounted with the installed
//! Steam games (default `game`); `--out` the content root the engine
//! reads (default `<game>/cache/content`).

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, bail};
use refract_convert::{Converter, MapOutcome};
use refract_formats::{Steam, mount_default};

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,refract_convert=info")).init();
    let mut args = std::env::args().skip(1);
    let (mut game, mut out, mut force, mut maps) = (PathBuf::from("game"), None, false, Vec::new());
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "map" => maps.push(args.next().context("map: missing name")?),
            "--game" => game = args.next().context("--game: missing folder")?.into(),
            "--out" => out = Some(PathBuf::from(args.next().context("--out: missing folder")?)),
            "--force" => force = true,
            _ => bail!("unknown argument {arg}\nusage: refract-convert map <name> [--game <dir>] [--out <dir>] [--force]"),
        }
    }
    if maps.is_empty() {
        bail!("nothing to convert\nusage: refract-convert map <name> [--game <dir>] [--out <dir>] [--force]");
    }

    let out = out.unwrap_or_else(|| game.join("cache").join("content"));
    let fs = mount_default(&game, Steam::locate().as_ref());
    let mut converter = Converter::new(fs, out, force);
    for name in maps {
        let start = Instant::now();
        match converter.map(&name)? {
            MapOutcome::Converted => log::info!("{name} converted in {:.1} s", start.elapsed().as_secs_f32()),
            MapOutcome::UpToDate => log::info!("{name} is up to date"),
        }
    }
    converter.finish()?;
    Ok(())
}
