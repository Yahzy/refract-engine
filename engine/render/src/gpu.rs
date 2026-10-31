//! The Vulkan device and the features the renderer relies on.

use crate::RenderError;

pub struct Gpu {
    /// Ray queries are enabled (the GPU traces rays in hardware).
    pub ray_query: bool,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

/// Shaders are SPIR-V compiled from Slang and given to Vulkan as they are;
/// textures are block compressed and bound all at once (bindless), indexed
/// per material.
const REQUIRED_FEATURES: wgpu::Features = wgpu::Features::PASSTHROUGH_SHADERS
    .union(wgpu::Features::TEXTURE_COMPRESSION_BC)
    .union(wgpu::Features::TEXTURE_BINDING_ARRAY)
    .union(wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING)
    .union(wgpu::Features::PARTIALLY_BOUND_BINDING_ARRAY);

/// Ray queries against acceleration structures, for baking light.
pub const RAY_QUERY: wgpu::Features = wgpu::Features::EXPERIMENTAL_RAY_QUERY;

impl Gpu {
    /// A device for rendering, with ray queries when the GPU has them and
    /// `ray_tracing` allows.
    pub fn new(instance: wgpu::Instance, surface: Option<&wgpu::Surface<'_>>, ray_tracing: bool) -> Result<Self, RenderError> {
        let optional = if ray_tracing { RAY_QUERY } else { wgpu::Features::empty() };
        Self::create(instance, surface, wgpu::Features::empty(), optional)
    }

    /// A device with `extra` features on top of those the renderer needs.
    pub fn with_features(instance: wgpu::Instance, surface: Option<&wgpu::Surface<'_>>, extra: wgpu::Features) -> Result<Self, RenderError> {
        Self::create(instance, surface, extra, wgpu::Features::empty())
    }

    /// `extra` features are required, `optional` ones taken when available.
    fn create(instance: wgpu::Instance, surface: Option<&wgpu::Surface<'_>>, extra: wgpu::Features, optional: wgpu::Features) -> Result<Self, RenderError> {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: surface,
        }))?;
        let info = adapter.get_info();
        log::info!("GPU: {} ({:?}, driver {})", info.name, info.backend, info.driver_info);

        let features = REQUIRED_FEATURES | extra | (optional & adapter.features());
        let missing = features - adapter.features();
        if !missing.is_empty() {
            return Err(RenderError::MissingFeatures(missing));
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("refract"),
            required_features: features,
            required_limits: adapter.limits(),
            // SAFETY: the only experimental feature asked for is ray queries,
            // used as documented (acceleration structures built before use).
            experimental_features: if features.contains(RAY_QUERY) { unsafe { wgpu::ExperimentalFeatures::enabled() } } else { wgpu::ExperimentalFeatures::disabled() },
            ..Default::default()
        }))?;
        Ok(Self { ray_query: features.contains(RAY_QUERY), adapter, device, queue })
    }

    pub fn instance() -> wgpu::Instance {
        wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            flags: wgpu::InstanceFlags::from_build_config(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        })
    }
}
