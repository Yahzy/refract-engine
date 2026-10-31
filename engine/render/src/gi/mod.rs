//! Global illumination: light bounced between surfaces, stored in a grid of
//! probes. The same code bakes the grid offline (the converter) and, later,
//! will update it while the game runs.

pub mod bake;
pub mod rays;
