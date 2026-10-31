//! Window, main loop and input. The application implements [`App`]; [`run`]
//! opens the window, builds the app and calls it back once per frame.

mod input;
mod run;

pub use input::Input;
pub use run::{App, BoxError, PlatformError, WindowConfig, capture_cursor, run};
pub use winit::keyboard::KeyCode;
pub use winit::event::MouseButton;
pub use winit::window::Window;
