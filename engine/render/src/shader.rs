//! Loading the SPIR-V that `build.rs` compiled from `shaders/*.slang`.

/// The SPIR-V of the Slang program `shaders/<name>.slang`.
macro_rules! spirv {
    ($name:literal) => {
        include_bytes!(concat!(env!("OUT_DIR"), "/", $name, ".spv"))
    };
}
pub(crate) use spirv;

pub fn load(device: &wgpu::Device, label: &str, spirv: &[u8]) -> wgpu::ShaderModule {
    let words: Vec<u32> = bytemuck::pod_collect_to_vec(spirv);
    // SAFETY: the SPIR-V comes from the Slang compiler at build time, and each
    // pipeline declares bind group layouts matching the shader's bindings.
    unsafe {
        device.create_shader_module_passthrough(wgpu::ShaderModuleDescriptorPassthrough {
            label: Some(label),
            spirv: Some(words.into()),
            ..Default::default()
        })
    }
}
