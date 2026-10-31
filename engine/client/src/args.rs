//! Command line: `+command args...` sets up the session, `-flag` and
//! `--option value` change how the client runs.

use anyhow::{Context, bail};
use glam::Vec3;
use refract_core::Angles;

#[derive(Debug, Default)]
pub struct Args {
    /// Map to load; the test scene without one.
    pub map: Option<String>,
    pub position: Option<Vec3>,
    pub angles: Option<Angles>,
    pub vsync: bool,
    /// Hardware ray tracing, when the GPU has it.
    pub ray_tracing: bool,
    /// Renders one frame offscreen to this PNG, then exits.
    pub screenshot: Option<String>,
    pub size: (u32, u32),
    /// Scene time of the screenshot, in seconds.
    pub time: f32,
    /// Renders this many frames offscreen, reports the frame times, then exits.
    pub bench: Option<u32>,
}

impl Args {
    pub fn parse(args: impl IntoIterator<Item = String>) -> anyhow::Result<Self> {
        let mut parsed = Args { vsync: true, ray_tracing: true, size: (1280, 720), ..Default::default() };
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let mut next = |what: &str| args.next().with_context(|| format!("{arg}: missing {what}"));
            match arg.as_str() {
                "+map" => parsed.map = Some(next("name")?),
                "+setpos" => parsed.position = Some(Vec3::new(number(next("x")?)?, number(next("y")?)?, number(next("z")?)?)),
                "+setang" => parsed.angles = Some(Angles::new(number(next("pitch")?)?, number(next("yaw")?)?, number(next("roll")?)?)),
                "-novsync" => parsed.vsync = false,
                "-noraytracing" => parsed.ray_tracing = false,
                "--screenshot" => parsed.screenshot = Some(next("file")?),
                "--time" => parsed.time = number(next("seconds")?)?,
                "--bench" => parsed.bench = Some(next("frames")?.parse().context("--bench: frame count")?),
                "--size" => {
                    let size = next("WIDTHxHEIGHT")?;
                    let (w, h) = size.split_once('x').with_context(|| format!("--size {size}: expected WIDTHxHEIGHT"))?;
                    parsed.size = (w.parse()?, h.parse()?);
                }
                _ => bail!("unknown argument {arg}"),
            }
        }
        Ok(parsed)
    }
}

fn number(text: String) -> anyhow::Result<f32> {
    text.parse().with_context(|| format!("not a number: {text}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> anyhow::Result<Args> {
        Args::parse(line.split_whitespace().map(String::from))
    }

    #[test]
    fn commands_and_options() {
        let args = parse("+map test_map +setpos 1 2 3 +setang 10 90 0 -novsync --size 640x480").unwrap();
        assert_eq!(args.map.as_deref(), Some("test_map"));
        assert_eq!(args.position, Some(Vec3::new(1.0, 2.0, 3.0)));
        assert_eq!(args.angles, Some(Angles::new(10.0, 90.0, 0.0)));
        assert!(!args.vsync);
        assert_eq!(args.size, (640, 480));
    }

    #[test]
    fn rejects_unknown_and_incomplete() {
        assert!(parse("+bogus").is_err());
        assert!(parse("+setpos 1 2").is_err());
    }
}
