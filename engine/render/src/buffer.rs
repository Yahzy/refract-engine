//! GPU buffers that grow on demand. Growing replaces the buffer, so whoever
//! binds it watches [`GrowableBuffer::generation`] to rebuild bind groups.

pub struct GrowableBuffer {
    buffer: wgpu::Buffer,
    label: &'static str,
    usage: wgpu::BufferUsages,
    len: u64,
    generation: u32,
}

impl GrowableBuffer {
    pub fn new(device: &wgpu::Device, label: &'static str, usage: wgpu::BufferUsages, capacity: u64) -> Self {
        let usage = usage | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC;
        let buffer = create(device, label, usage, capacity.max(wgpu::COPY_BUFFER_ALIGNMENT));
        Self { buffer, label, usage, len: 0, generation: 0 }
    }

    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    /// Changes every time the buffer is replaced by a bigger one.
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// Adds `data` after the current contents; returns its byte offset.
    pub fn append(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[u8]) -> u64 {
        let offset = self.len;
        self.reserve(device, queue, offset + data.len() as u64, true);
        queue.write_buffer(&self.buffer, offset, data);
        self.len += data.len() as u64;
        offset
    }

    /// Replaces the whole contents with `data`.
    pub fn replace(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[u8]) {
        self.reserve(device, queue, data.len() as u64, false);
        if !data.is_empty() {
            queue.write_buffer(&self.buffer, 0, data);
        }
        self.len = data.len() as u64;
    }

    fn reserve(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, needed: u64, keep: bool) {
        if needed <= self.buffer.size() {
            return;
        }
        let capacity = needed.next_power_of_two().max(self.buffer.size() * 2);
        let bigger = create(device, self.label, self.usage, capacity);
        if keep && self.len > 0 {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("grow buffer") });
            encoder.copy_buffer_to_buffer(&self.buffer, 0, &bigger, 0, self.len);
            queue.submit([encoder.finish()]);
        }
        self.buffer = bigger;
        self.generation += 1;
    }
}

fn create(device: &wgpu::Device, label: &str, usage: wgpu::BufferUsages, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false })
}
