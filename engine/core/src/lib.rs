//! Engine foundation: math types and the conventions of the world.
//!
//! The world is right-handed with X forward, Y left and Z up, measured in
//! world units (1 unit = 1 inch). Every engine crate works in this space.

pub mod angles;
pub mod axes;

pub use angles::Angles;
pub use glam;
