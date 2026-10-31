//! Copying a rendered image back to the CPU (screenshots, tests).

use crate::RenderError;

/// 8-bit RGBA pixels, sRGB encoded, rows top to bottom.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn read_rgba8(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Result<Image, RenderError> {
    let (width, height) = (texture.width(), texture.height());
    let row_bytes = width * 4;
    let padded_row_bytes = row_bytes.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(padded_row_bytes * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("readback") });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded_row_bytes), rows_per_image: Some(height) },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);

    let (sender, receiver) = std::sync::mpsc::channel();
    buffer.map_async(wgpu::MapMode::Read, .., move |result| {
        let _ = sender.send(result);
    });
    device.poll(wgpu::PollType::wait_indefinitely()).map_err(|err| RenderError::Readback(err.to_string()))?;
    receiver
        .recv()
        .map_err(|err| RenderError::Readback(err.to_string()))?
        .map_err(|err| RenderError::Readback(err.to_string()))?;

    let mapped = buffer.get_mapped_range(..);
    let mut rgba = Vec::with_capacity((row_bytes * height) as usize);
    for row in mapped.chunks_exact(padded_row_bytes as usize) {
        rgba.extend_from_slice(&row[..row_bytes as usize]);
    }
    Ok(Image { width, height, rgba })
}
