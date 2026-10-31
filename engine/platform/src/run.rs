//! The main loop: creates the window, forwards input, and runs one frame per
//! redraw, as fast as the swapchain lets it.

use std::error::Error;
use std::sync::Arc;
use std::time::Instant;

use glam::Vec2;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{DeviceEvent, DeviceId, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{CursorGrabMode, Window, WindowId};

use crate::Input;

pub type BoxError = Box<dyn Error + Send + Sync>;

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("event loop: {0}")]
    EventLoop(#[from] winit::error::EventLoopError),
    #[error("window creation: {0}")]
    Window(#[from] winit::error::OsError),
    #[error(transparent)]
    App(BoxError),
}

pub struct WindowConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
}

pub trait App: Sized {
    /// The window's drawable area changed, in physical pixels.
    fn resized(&mut self, width: u32, height: u32);
    /// Runs one frame, `dt` seconds after the previous one. Returns false to quit.
    fn frame(&mut self, window: &Window, input: &Input, dt: f32) -> bool;
}

/// Opens the window, builds the app with `create`, and runs it until it quits
/// or its window is closed.
pub fn run<A: App>(config: WindowConfig, create: impl FnOnce(Arc<Window>) -> Result<A, BoxError>) -> Result<(), PlatformError> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut runner = Runner { app: None, create: Some(create), window: None, config, input: Input::default(), last_frame: Instant::now(), error: None };
    event_loop.run_app(&mut runner)?;
    runner.error.map_or(Ok(()), Err)
}

/// Hides the cursor and locks it in place (mouse look), or releases it.
pub fn capture_cursor(window: &Window, captured: bool) {
    let grab = if captured {
        window.set_cursor_grab(CursorGrabMode::Locked).or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined))
    } else {
        window.set_cursor_grab(CursorGrabMode::None)
    };
    if let Err(err) = grab {
        log::warn!("cursor grab: {err}");
    }
    window.set_cursor_visible(!captured);
}

struct Runner<A, F> {
    // Declared before the window so it is dropped first: it may hold the window's surface.
    app: Option<A>,
    create: Option<F>,
    window: Option<Arc<Window>>,
    config: WindowConfig,
    input: Input,
    last_frame: Instant,
    error: Option<PlatformError>,
}

impl<A: App, F: FnOnce(Arc<Window>) -> Result<A, BoxError>> Runner<A, F> {
    fn start(&mut self, event_loop: &ActiveEventLoop) -> Result<(), PlatformError> {
        let attributes = Window::default_attributes()
            .with_title(self.config.title.clone())
            .with_inner_size(LogicalSize::new(self.config.width, self.config.height));
        let window = Arc::new(event_loop.create_window(attributes)?);
        let create = self.create.take().expect("started once");
        self.app = Some(create(window.clone()).map_err(PlatformError::App)?);
        self.window = Some(window);
        self.last_frame = Instant::now();
        Ok(())
    }
}

impl<A: App, F: FnOnce(Arc<Window>) -> Result<A, BoxError>> ApplicationHandler for Runner<A, F> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.app.is_none()
            && let Err(err) = self.start(event_loop)
        {
            self.error = Some(err);
            event_loop.exit();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let (Some(app), Some(window)) = (&mut self.app, &self.window) else { return };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => app.resized(size.width, size.height),
            WindowEvent::Focused(false) => self.input.release_all(),
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(key) = event.physical_key {
                    self.input.on_key(key, event.state, event.repeat);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => self.input.on_button(button, state),
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = (now - self.last_frame).as_secs_f32();
                self.last_frame = now;
                if !app.frame(window, &self.input, dt) {
                    event_loop.exit();
                }
                self.input.end_frame();
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta: (x, y) } = event {
            self.input.on_mouse_motion(Vec2::new(x as f32, y as f32));
        }
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}
