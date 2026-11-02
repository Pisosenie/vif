use etagere::{AtlasAllocator, size2};

pub(crate) struct Atlas {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    allocator: AtlasAllocator,
    size: u32,
    bytes: u32,
}

impl Atlas {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, size: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vif atlas"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bytes = format.block_copy_size(None).unwrap_or(1);
        Self {
            texture,
            view,
            allocator: AtlasAllocator::new(size2(size as i32, size as i32)),
            size,
            bytes,
        }
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    /// Uploads an image and returns its top-left corner, or `None` when the atlas is full.
    pub fn insert(&mut self, queue: &wgpu::Queue, width: u32, height: u32, data: &[u8]) -> Option<[u32; 2]> {
        let alloc = self.allocator.allocate(size2(width as i32 + 1, height as i32 + 1))?;
        let origin = [alloc.rectangle.min.x as u32, alloc.rectangle.min.y as u32];
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: origin[0],
                    y: origin[1],
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * self.bytes),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        Some(origin)
    }

    pub fn clear(&mut self) {
        self.allocator.clear();
    }
}
