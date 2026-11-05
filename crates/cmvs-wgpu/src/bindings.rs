//! Reusable scene uniforms and texture bindings. Queue writes precede each scene's
//! submission, so later frames can reuse the buffer without a CPU/GPU idle wait.
use super::*;
use cmvs_core::scene_render::shader::PARAM_FLOATS;
use std::collections::HashSet;

const PARAM_BYTES: u64 = (PARAM_FLOATS * 4) as u64;

struct Group {
    binding: wgpu::BindGroup,
    used: bool,
}

pub(super) struct DrawBindings {
    buffer: wgpu::Buffer,
    stride: u64,
    data: Vec<u8>,
    groups: HashMap<[wgpu::TextureView; 3], Group>,
}

impl DrawBindings {
    pub fn new(device: &wgpu::Device) -> Self {
        let alignment = u64::from(device.limits().min_uniform_buffer_offset_alignment);
        let stride = PARAM_BYTES.div_ceil(alignment) * alignment;
        Self {
            buffer: Self::buffer(device, stride),
            stride,
            data: Vec::new(),
            groups: HashMap::new(),
        }
    }

    fn buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene parameters"),
            size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn prepare(&mut self, device: &wgpu::Device, count: usize) -> Result<()> {
        let size = (count as u64)
            .checked_mul(self.stride)
            .ok_or("scene parameters exceed buffer limits")?;
        // Dynamic offsets are u32 even on devices with larger buffer limits.
        let limit = device.limits().max_buffer_size.min(u64::from(u32::MAX)) & !3;
        if size > limit {
            return Err("scene parameters exceed buffer limits".into());
        }
        if size > self.buffer.size() {
            self.buffer = Self::buffer(device, size.next_power_of_two().min(limit));
            self.groups.clear(); // Bindings refer to the previous buffer.
        }
        self.data.resize(size as usize, 0);
        for group in self.groups.values_mut() {
            group.used = false;
        }
        Ok(())
    }

    pub fn write(&mut self, index: usize, params: &[f32]) -> u32 {
        let offset = index * self.stride as usize;
        for (bytes, value) in self.data[offset..]
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(params)
        {
            bytes.copy_from_slice(&value.to_le_bytes());
        }
        offset as u32
    }

    pub fn group(
        &mut self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        views: [wgpu::TextureView; 3],
    ) -> wgpu::BindGroup {
        let group = self.groups.entry(views).or_insert_with_key(|views| Group {
            binding: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("scene textures"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.buffer,
                            offset: 0,
                            size: wgpu::BufferSize::new(PARAM_BYTES),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&views[0]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&views[1]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&views[2]),
                    },
                ],
            }),
            used: true,
        });
        group.used = true;
        group.binding.clone()
    }

    pub fn upload(&self, queue: &wgpu::Queue) {
        if !self.data.is_empty() {
            queue.write_buffer(&self.buffer, 0, &self.data);
        }
    }

    pub fn retain<'a>(&mut self, views: impl Iterator<Item = &'a wgpu::TextureView>) {
        // wgpu hashes immutable resource identity, not the handle's interior state.
        #[allow(clippy::mutable_key_type)]
        let live: HashSet<_> = views.collect();
        // Keep only this scene's combinations of cached textures. Neither old
        // combinations nor evicted/oversized textures may extend cache residency.
        self.groups
            .retain(|views, group| group.used && views.iter().all(|view| live.contains(view)));
    }

    pub fn clear(&mut self) {
        self.groups.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cmvs_core::scene_render::Blend;

    #[test]
    #[ignore = "requires a native GPU adapter; run explicitly"]
    fn reused_bindings_preserve_offsets_and_pending_frames() {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle().with_env());
        let (mut gpu, _) = Renderer::new(&instance, None).unwrap();
        let render = |gpu: &mut Renderer, count: usize, red: u8| {
            let layers: Vec<_> = (0..count)
                .map(|i| Layer::Solid {
                    rect: [i as i64, 0, 1, 1],
                    color: [red + i as u8, 40, 80],
                    alpha: 255,
                    blend: Blend::Replace,
                })
                .collect();
            gpu.compose([8, 1], [0, 0, 0, 255], &layers).unwrap()
        };
        let first = render(&mut gpu, 1, 10);
        let first_buffer = gpu.bindings.buffer.clone();
        let grown = render(&mut gpu, 5, 20);
        assert_ne!(gpu.bindings.buffer, first_buffer);
        assert_eq!(gpu.bindings.groups.len(), 1);
        let buffer = gpu.bindings.buffer.clone();
        let group = gpu.bindings.groups.values().next().unwrap().binding.clone();
        let reused = render(&mut gpu, 5, 30);
        assert_eq!(gpu.bindings.buffer, buffer);
        assert_eq!(gpu.bindings.groups.values().next().unwrap().binding, group);
        let smaller = render(&mut gpu, 3, 50);

        // Eviction must not leave bindings to the old texture resident in the cache.
        gpu.cache.clear();
        gpu.cache_bytes = 0;
        let replaced = render(&mut gpu, 2, 60);
        assert_eq!(gpu.bindings.groups.len(), 1);
        assert_ne!(gpu.bindings.groups.values().next().unwrap().binding, group);
        let empty = render(&mut gpu, 0, 0);
        assert!(gpu.bindings.groups.is_empty());

        // No waits/readbacks between submissions: overwriting or reallocating the
        // uniform buffer must not change any previously submitted logical frame.
        for (frame, count, red) in [
            (first, 1, 10),
            (grown, 5, 20),
            (reused, 5, 30),
            (smaller, 3, 50),
            (replaced, 2, 60),
            (empty, 0, 0),
        ] {
            let image = frame.read().unwrap();
            for (i, pixel) in image.pixels().as_chunks::<4>().0.iter().enumerate() {
                let expected = if i < count {
                    [red + i as u8, 40, 80, 255]
                } else {
                    [0, 0, 0, 255]
                };
                assert_eq!(pixel, &expected);
            }
        }
        gpu.check().unwrap();
    }
}
