//! The Refract client.
//!
//! refract [+map <name>] [+setpos x y z] [+setang pitch yaw roll] [-rendertest]
//!         [-novsync] [-noraytracing]
//!         [--screenshot <file.png>] [--size WIDTHxHEIGHT] [--time <seconds>]
//!         [--bench <frames>]
//!
//! Without a map, the playground, played on foot (see `player.rs`). Maps and
//! the render test scene are flown through (see `flycam.rs`).

mod args;
mod flycam;
mod game;
mod level;
mod map;
mod player;
mod playground;
mod props;
mod scene;
mod test_scene;
mod textures;

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context;
use refract_platform::{App, BoxError, Input, KeyCode, MouseButton, Window, WindowConfig};
use refract_render::{Camera, Renderer, RendererConfig};
use refract_world::PlayerCommand;

use args::Args;
use game::Game;
use scene::Scene;

/// The game folder: content found there is mounted first, and converted
/// content goes into its `cache/content`.
const GAME_DIR: &str = "game";

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,refract=info,refract_render=info")).init();
    let args = Args::parse(std::env::args().skip(1))?;
    if let Some(path) = &args.screenshot {
        return screenshot(&args, path);
    }
    if let Some(frames) = args.bench {
        return bench(&args, frames);
    }
    let config = WindowConfig { title: "Refract".into(), width: args.size.0, height: args.size.1 };
    refract_platform::run(config, |window| Client::new(window, &args))?;
    Ok(())
}

struct Client {
    renderer: Renderer,
    scene: Scene,
    control: Control,
    start: Instant,
    mouse_captured: bool,
    fps: FpsCounter,
}

/// Who moves the camera.
enum Control {
    Player(Box<Game>),
    Fly(Camera),
}

impl Client {
    fn new(window: Arc<Window>, args: &Args) -> Result<Self, BoxError> {
        let size = window.inner_size();
        let mut renderer = Renderer::with_surface(window, size.width, size.height, RendererConfig { vsync: args.vsync, ray_tracing: args.ray_tracing })?;
        let mut scene = load_scene(&mut renderer, args)?;
        let camera = initial_camera(&scene, args);
        let control = match scene.collision.take() {
            Some(collision) => {
                let origin = args.position.unwrap_or(playground::SPAWN);
                let props = std::mem::take(&mut scene.props);
                Control::Player(Box::new(Game::new(&mut renderer, collision, props, origin, camera.angles.yaw)))
            }
            None => Control::Fly(camera),
        };
        Ok(Self { renderer, scene, control, start: Instant::now(), mouse_captured: false, fps: FpsCounter::default() })
    }

    fn capture(&mut self, window: &Window, captured: bool) {
        if captured != self.mouse_captured {
            refract_platform::capture_cursor(window, captured);
            self.mouse_captured = captured;
        }
    }
}

impl App for Client {
    fn resized(&mut self, width: u32, height: u32) {
        self.renderer.resize(width, height);
    }

    fn frame(&mut self, window: &Window, input: &Input, dt: f32) -> bool {
        let camera = match &mut self.control {
            Control::Fly(camera) => {
                if input.key_pressed(KeyCode::Escape) {
                    return false;
                }
                flycam::update(camera, input, dt);
                let camera = *camera;
                self.capture(window, flycam::looking(input));
                camera
            }
            Control::Player(_) => {
                // Escape frees the mouse, then quits; a click takes it back.
                if input.key_pressed(KeyCode::Escape) {
                    if !self.mouse_captured {
                        return false;
                    }
                    self.capture(window, false);
                } else if !input.focused() {
                    self.capture(window, false);
                } else if input.button_down(MouseButton::Left) {
                    self.capture(window, true);
                }
                let Control::Player(game) = &mut self.control else { unreachable!() };
                game.update(input, self.mouse_captured, dt, &mut self.renderer);
                game.camera()
            }
        };

        let time = self.start.elapsed().as_secs_f32();
        self.scene.update(&mut self.renderer, time);
        if let Err(err) = self.renderer.render(&self.scene.frame(camera, time)) {
            log::error!("render: {err}");
            return false;
        }
        if let Some(fps) = self.fps.tick(dt) {
            let speed = match &self.control {
                Control::Player(game) => format!(" — {:.0} u/s", game.speed()),
                Control::Fly(_) => String::new(),
            };
            window.set_title(&format!("Refract — {fps:.0} FPS ({:.2} ms){speed}", 1000.0 / fps));
        }
        true
    }
}

fn load_scene(renderer: &mut Renderer, args: &Args) -> anyhow::Result<Scene> {
    match &args.map {
        Some(name) => map::load(renderer, Path::new(GAME_DIR), name),
        None if args.render_test => Ok(test_scene::build(renderer)),
        None => Ok(playground::build(renderer)),
    }
}

fn initial_camera(scene: &Scene, args: &Args) -> Camera {
    let mut camera = scene.spawn;
    camera.position = args.position.unwrap_or(camera.position);
    camera.angles = args.angles.unwrap_or(camera.angles);
    camera
}

/// Renders one frame offscreen and writes it as a PNG.
fn screenshot(args: &Args, path: &str) -> anyhow::Result<()> {
    let (width, height) = args.size;
    let mut renderer = Renderer::offscreen(width, height, RendererConfig { vsync: false, ray_tracing: args.ray_tracing })?;
    let mut scene = load_scene(&mut renderer, args)?;
    let mut camera = initial_camera(&scene, args);
    // A level played on foot: its props settle for `--time` seconds, the
    // player standing still.
    if let Some(collision) = scene.collision.take() {
        let origin = args.position.unwrap_or(playground::SPAWN);
        let mut game = Game::new(&mut renderer, collision, std::mem::take(&mut scene.props), origin, camera.angles.yaw);
        let steps = (args.time * 60.0).ceil() as u32;
        for _ in 0..steps {
            game.advance(PlayerCommand { angles: camera.angles, ..Default::default() }, false, args.time / steps as f32, &mut renderer);
        }
        camera = Camera { angles: camera.angles, ..game.camera() };
    }
    scene.update(&mut renderer, args.time);
    renderer.render(&scene.frame(camera, args.time))?;
    let image = renderer.read_image()?;

    let file = std::fs::File::create(path).with_context(|| format!("creating {path}"))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&image.rgba)?;
    log::info!("screenshot saved to {path}");
    Ok(())
}

/// Renders `frames` frames offscreen with the lights moving, and reports the
/// CPU time spent preparing and submitting each frame, and the full frame time
/// including the GPU's work.
fn bench(args: &Args, frames: u32) -> anyhow::Result<()> {
    let (width, height) = args.size;
    let mut renderer = Renderer::offscreen(width, height, RendererConfig { vsync: false, ray_tracing: args.ray_tracing })?;
    let mut scene = load_scene(&mut renderer, args)?;
    let camera = initial_camera(&scene, args);
    // Warm-up: pipelines, first uploads.
    renderer.render(&scene.frame(camera, 0.0))?;
    renderer.wait_idle();

    let (mut cpu, mut total) = (Vec::new(), Vec::new());
    for i in 0..frames {
        let time = i as f32 / 60.0;
        let start = Instant::now();
        scene.update(&mut renderer, time);
        renderer.render(&scene.frame(camera, time))?;
        cpu.push(start.elapsed().as_secs_f64() * 1000.0);
        renderer.wait_idle();
        total.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let report = |name: &str, times: &mut Vec<f64>| {
        times.sort_by(f64::total_cmp);
        let mean = times.iter().sum::<f64>() / times.len() as f64;
        println!("{name}: mean {mean:.3} ms, median {:.3} ms, p99 {:.3} ms", times[times.len() / 2], times[times.len() * 99 / 100]);
    };
    println!("{frames} frames at {width}x{height}");
    report("CPU (prepare + submit)", &mut cpu);
    report("frame (CPU + GPU)", &mut total);
    Ok(())
}

/// Average frame rate over about a second.
#[derive(Default)]
struct FpsCounter {
    elapsed: f32,
    frames: u32,
}

impl FpsCounter {
    fn tick(&mut self, dt: f32) -> Option<f32> {
        self.elapsed += dt;
        self.frames += 1;
        if self.elapsed < 1.0 {
            return None;
        }
        let fps = self.frames as f32 / self.elapsed;
        *self = Self::default();
        Some(fps)
    }
}
