//! Baked indirect light: a grid of probes (see `refract_asset::map::Probe`),
//! read by the shaders from a storage buffer and blended per pixel between
//! the eight around it, skipping probes inside walls or behind surfaces.

use glam::Vec3;
use refract_asset::map::Probe;
use wgpu::util::DeviceExt;

/// A probe grid as maps store it.
pub struct ProbeGrid<'a> {
    /// Corner of the first cell; probes sit at cell centres.
    pub origin: Vec3,
    pub cell_size: f32,
    pub resolution: [u32; 3],
    /// x fastest, then y, then z.
    pub probes: &'a [Probe],
}

pub struct ProbeVolume {
    pub buffer: wgpu::Buffer,
    pub origin: Vec3,
    pub cell_size: f32,
    pub resolution: [u32; 3],
    /// False when there is no grid: the sky lights everything.
    pub present: bool,
}

impl ProbeVolume {
    pub fn none(device: &wgpu::Device) -> Self {
        let mut volume = Self::upload(device, &ProbeGrid { origin: Vec3::ZERO, cell_size: 1.0, resolution: [1, 1, 1], probes: &[Probe::default()] });
        volume.present = false;
        volume
    }

    pub fn upload(device: &wgpu::Device, grid: &ProbeGrid) -> Self {
        assert_eq!(grid.probes.len(), grid.resolution.iter().map(|&r| r as usize).product::<usize>(), "probe count");
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("probes"), contents: bytemuck::cast_slice(grid.probes), usage: wgpu::BufferUsages::STORAGE });
        Self { buffer, origin: grid.origin, cell_size: grid.cell_size, resolution: grid.resolution, present: true }
    }
}
