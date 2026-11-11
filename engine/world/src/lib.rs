//! The game simulation: solid geometry, entities, physics objects, player
//! movement, fixed ticks.
//!
//! Nothing here knows about windows or GPUs: a dedicated server runs the same
//! code without them. The simulation advances in fixed ticks, whatever the
//! frame rate; what is drawn is interpolated between the last two ticks.

pub mod collision;
pub mod entity;
pub mod movement;
pub mod world;

pub use collision::{Aabb, Brush, Collision, Hull, Plane, Solid, Trace};
pub use entity::{Entities, EntityId};
pub use movement::{MoveSettings, Player, PlayerCommand, Touch};
pub use refract_physics::Material;
pub use world::{Prop, PropDesc, PropShape, World};

/// Simulation ticks per second.
pub const TICK_RATE: f32 = 66.0 + 2.0 / 3.0;
/// Seconds per tick.
pub const TICK: f32 = 1.0 / TICK_RATE;

/// Turns frame times into a whole number of ticks.
#[derive(Clone, Copy, Debug, Default)]
pub struct TickClock {
    accumulated: f32,
}

/// Longest frame caught up on: after a stall, the simulation slows down
/// rather than running hundreds of ticks at once.
const MAX_CATCH_UP: f32 = 0.25;

impl TickClock {
    /// Adds a frame's time and returns how many ticks to run.
    pub fn advance(&mut self, dt: f32) -> u32 {
        self.accumulated += dt.min(MAX_CATCH_UP);
        let ticks = (self.accumulated / TICK) as u32;
        self.accumulated -= ticks as f32 * TICK;
        ticks
    }

    /// How far the present is between the last tick and the next, 0 to 1:
    /// what is drawn blends the last two ticks' states by this.
    pub fn alpha(&self) -> f32 {
        (self.accumulated / TICK).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_runs_whole_ticks_and_keeps_the_rest() {
        let mut clock = TickClock::default();
        assert_eq!(clock.advance(TICK * 2.5), 2);
        assert!((clock.alpha() - 0.5).abs() < 1e-3);
        assert_eq!(clock.advance(TICK * 0.6), 1);
    }
}
