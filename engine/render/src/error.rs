#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("no Vulkan adapter: {0}")]
    Adapter(#[from] wgpu::RequestAdapterError),
    #[error("device creation: {0}")]
    Device(#[from] wgpu::RequestDeviceError),
    #[error("surface creation: {0}")]
    Surface(#[from] wgpu::CreateSurfaceError),
    #[error("the GPU lacks required features: {0:?}")]
    MissingFeatures(wgpu::Features),
    #[error("the surface was lost and could not be recreated")]
    SurfaceLost,
    #[error("reading the image back failed: {0}")]
    Readback(String),
}
