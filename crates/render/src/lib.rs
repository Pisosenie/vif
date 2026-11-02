mod atlas;
mod builtin;
pub mod chrome;
mod font;

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use swash::scale::image::Content;
use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::zeno::{Angle, Format, Transform};

use atlas::Atlas;
use font::Fonts;

pub use font::{FontConfig, Metrics, Style};

pub const RECT_FILL: u32 = 0;
pub const RECT_CURL: u32 = 1;
pub const RECT_DOTTED: u32 = 2;
pub const RECT_DASHED: u32 = 3;
pub const RECT_HOLLOW: u32 = 4;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Rect {
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub color: [f32; 4],
    pub kind: u32,
    pub radius: f32,
    pub thickness: f32,
    pub _pad: f32,
}

impl Rect {
    pub fn fill(x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) -> Self {
        Self {
            pos: [x, y],
            size: [w, h],
            color,
            kind: RECT_FILL,
            radius: 0.0,
            thickness: 0.0,
            _pad: 0.0,
        }
    }

    pub fn rounded(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    pub fn kind(mut self, kind: u32, thickness: f32) -> Self {
        self.kind = kind;
        self.thickness = thickness;
        self
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Sprite {
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub uv: [f32; 4],
    pub color: [f32; 4],
}

/// Everything one frame draws, back to front.
#[derive(Default, Clone)]
pub struct Scene {
    pub back: Vec<Rect>,
    pub mask: Vec<Sprite>,
    pub color: Vec<Sprite>,
    pub front: Vec<Rect>,
    /// Glyphs drawn over `front`, for panels that sit on top of the text.
    pub over: Vec<Sprite>,
    /// Slices of uploaded images, keyed by [`Renderer::image`]'s key; uv is 0–1.
    pub images: Vec<(u64, Sprite)>,
}

impl Scene {
    pub fn clear(&mut self) {
        self.back.clear();
        self.mask.clear();
        self.color.clear();
        self.front.clear();
        self.over.clear();
        self.images.clear();
    }

    pub fn extend(&mut self, other: &Scene) {
        self.back.extend_from_slice(&other.back);
        self.mask.extend_from_slice(&other.mask);
        self.color.extend_from_slice(&other.color);
        self.front.extend_from_slice(&other.front);
        self.over.extend_from_slice(&other.over);
        self.images.extend_from_slice(&other.images);
    }
}

/// One terminal cell handed to [`Renderer::draw_line`].
#[derive(Clone, Copy, Debug)]
pub struct LineCell<'a> {
    /// The cell's characters, "" for the right half of a wide character.
    pub text: &'a str,
    pub style: Style,
    pub color: [f32; 4],
    pub wide: bool,
}

#[derive(Clone, Copy)]
struct Cached {
    color: bool,
    origin: [u32; 2],
    size: [u32; 2],
    left: i32,
    top: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    viewport: [f32; 2],
    mask_size: [f32; 2],
    color_size: [f32; 2],
    _pad: [f32; 2],
}

struct Instances {
    buffer: wgpu::Buffer,
    len: u32,
}

impl Instances {
    fn new(device: &wgpu::Device) -> Self {
        Self {
            buffer: buffer(device, 64 * 1024),
            len: 0,
        }
    }

    fn write<T: Pod>(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, items: &[T]) {
        let bytes: &[u8] = bytemuck::cast_slice(items);
        if bytes.len() as u64 > self.buffer.size() {
            self.buffer = buffer(device, (bytes.len() as u64).next_power_of_two());
        }
        if !bytes.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytes);
        }
        self.len = items.len() as u32;
    }
}

fn buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("vif instances"),
        size,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

struct Picture {
    bind_group: wgpu::BindGroup,
    used: u64,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    fonts: Fonts,
    font_config: FontConfig,
    scale: ScaleContext,
    glyphs: HashMap<(u16, u16), Option<Cached>>,
    drawn: HashMap<char, Option<Cached>>,
    generation: u64,
    mask: Atlas,
    color: Atlas,
    globals: wgpu::Buffer,
    layout: wgpu::BindGroupLayout,
    samplers: [wgpu::Sampler; 2],
    bind_group: wgpu::BindGroup,
    rect_pipeline: wgpu::RenderPipeline,
    mask_pipeline: wgpu::RenderPipeline,
    color_pipeline: wgpu::RenderPipeline,
    back: Instances,
    sprites: Instances,
    colors: Instances,
    front: Instances,
    over: Instances,
    image_layout: wgpu::BindGroupLayout,
    image_pipeline: wgpu::RenderPipeline,
    pictures: HashMap<u64, Picture>,
    picture_runs: Vec<(u64, u32, u32)>,
    pictures_buffer: Instances,
    frame: u64,
}

impl Renderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        font_config: FontConfig,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vif shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vif globals"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture_entry(1),
                texture_entry(2),
                sampler_entry(3),
                sampler_entry(4),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vif pipeline"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let rect_attrs = wgpu::vertex_attr_array![
            0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Uint32, 4 => Float32, 5 => Float32
        ];
        let sprite_attrs = wgpu::vertex_attr_array![
            0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4
        ];
        let pipeline = |vs: &str, fs: &str, stride: usize, attrs: &[wgpu::VertexAttribute]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: stride as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: attrs,
                    }],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview: None,
                cache: None,
            })
        };
        let rect_pipeline = pipeline("vs_rect", "fs_rect", size_of::<Rect>(), &rect_attrs);
        let mask_pipeline = pipeline("vs_mask", "fs_mask", size_of::<Sprite>(), &sprite_attrs);
        let color_pipeline = pipeline("vs_color", "fs_color", size_of::<Sprite>(), &sprite_attrs);
        let image_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vif image"),
            entries: &[texture_entry(0), sampler_entry(1)],
        });
        let image_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vif image pipeline"),
            bind_group_layouts: &[&layout, &image_layout],
            push_constant_ranges: &[],
        });
        let image_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("fs_image"),
            layout: Some(&image_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_image"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: size_of::<Sprite>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &sprite_attrs,
                }],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_image"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vif globals"),
            size: size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = |filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            })
        };
        let nearest = sampler(wgpu::FilterMode::Nearest);
        let smooth = sampler(wgpu::FilterMode::Linear);
        let mask = Atlas::new(device, wgpu::TextureFormat::R8Unorm, 1024);
        let color = Atlas::new(device, wgpu::TextureFormat::Rgba8Unorm, 512);
        let bind_group = bind_group(device, &layout, &globals, &mask, &color, &nearest, &smooth);

        Self {
            device: device.clone(),
            queue: queue.clone(),
            fonts: Fonts::new(&font_config),
            font_config,
            scale: ScaleContext::new(),
            glyphs: HashMap::new(),
            drawn: HashMap::new(),
            generation: 0,
            mask,
            color,
            globals,
            layout,
            samplers: [nearest, smooth],
            bind_group,
            rect_pipeline,
            mask_pipeline,
            color_pipeline,
            back: Instances::new(device),
            sprites: Instances::new(device),
            colors: Instances::new(device),
            front: Instances::new(device),
            over: Instances::new(device),
            image_layout,
            image_pipeline,
            pictures: HashMap::new(),
            picture_runs: Vec::new(),
            pictures_buffer: Instances::new(device),
            frame: 0,
        }
    }

    pub fn metrics(&self) -> Metrics {
        self.fonts.metrics
    }

    pub fn font_config(&self) -> &FontConfig {
        &self.font_config
    }

    /// Bumps whenever cached glyph placements become invalid.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn set_font(&mut self, config: FontConfig) {
        if config == self.font_config {
            return;
        }
        self.fonts = Fonts::new(&config);
        self.font_config = config;
        self.reset_atlas();
    }

    fn reset_atlas(&mut self) {
        self.glyphs.clear();
        self.drawn.clear();
        self.mask.clear();
        self.color.clear();
        self.generation += 1;
    }

    /// Lays out one row of cells starting at (x, y), the row's top-left corner.
    pub fn draw_line(&mut self, cells: &[LineCell], x: f32, y: f32, scene: &mut Scene) {
        let m = self.fonts.metrics;
        let mut start = 0;
        let mut texts: Vec<&str> = Vec::with_capacity(cells.len());
        while start < cells.len() {
            let style = cells[start].style;
            let mut end = start;
            while end < cells.len() && cells[end].style == style {
                end += 1;
            }
            texts.clear();
            for (i, cell) in cells[start..end].iter().enumerate() {
                let mut chars = cell.text.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) if builtin::covers(c) => {
                        let cell_x = x + (start + i) as f32 * m.cell_width;
                        self.draw_builtin(c, cell_x, y, cell.color, scene);
                        texts.push(" ");
                    }
                    _ => texts.push(cell.text),
                }
            }
            if texts.iter().any(|t| !t.trim().is_empty()) {
                let placed = self.fonts.shape(style, &texts);
                for p in placed.iter() {
                    let index = start + p.cell;
                    let cell = &cells[index];
                    let Some(glyph) = self.glyph(p.face, p.glyph) else {
                        continue;
                    };
                    let cell_x = x + index as f32 * m.cell_width;
                    let span = if cell.wide { 2.0 } else { 1.0 } * m.cell_width;
                    let uv = [
                        glyph.origin[0] as f32,
                        glyph.origin[1] as f32,
                        glyph.size[0] as f32,
                        glyph.size[1] as f32,
                    ];
                    if glyph.color {
                        let (w, h) = (glyph.size[0] as f32, glyph.size[1] as f32);
                        let fit = (span / w).min(m.cell_height / h).min(1.0);
                        let (w, h) = (w * fit, h * fit);
                        scene.color.push(Sprite {
                            pos: [
                                (cell_x + (span - w) / 2.0).round(),
                                (y + (m.cell_height - h) / 2.0).round(),
                            ],
                            size: [w, h],
                            uv,
                            color: [1.0, 1.0, 1.0, cell.color[3]],
                        });
                    } else {
                        scene.mask.push(Sprite {
                            pos: [
                                (cell_x + p.x).round() + glyph.left as f32,
                                (y + m.baseline + p.y).round() - glyph.top as f32,
                            ],
                            size: [glyph.size[0] as f32, glyph.size[1] as f32],
                            uv,
                            color: cell.color,
                        });
                    }
                }
            }
            start = end;
        }
    }

    fn draw_builtin(&mut self, c: char, x: f32, y: f32, color: [f32; 4], scene: &mut Scene) {
        let m = self.fonts.metrics;
        let cached = match self.drawn.get(&c) {
            Some(cached) => *cached,
            None => {
                let (w, h) = (m.cell_width as u32, m.cell_height as u32);
                let cached = builtin::draw(c, w, h, m.stroke).and_then(|pixels| {
                    let origin = self.insert(false, w, h, &pixels)?;
                    Some(Cached {
                        color: false,
                        origin,
                        size: [w, h],
                        left: 0,
                        top: 0,
                    })
                });
                self.drawn.insert(c, cached);
                cached
            }
        };
        if let Some(g) = cached {
            scene.mask.push(Sprite {
                pos: [x.round(), y.round()],
                size: [g.size[0] as f32, g.size[1] as f32],
                uv: [
                    g.origin[0] as f32,
                    g.origin[1] as f32,
                    g.size[0] as f32,
                    g.size[1] as f32,
                ],
                color,
            });
        }
    }

    /// Uploads an RGBA image once under `key`; later calls with the same key do nothing.
    pub fn image(&mut self, key: u64, rgba: &[u8], width: u32, height: u32) {
        if self.pictures.contains_key(&key) || width == 0 || height == 0 {
            return;
        }
        let max = self.device.limits().max_texture_dimension_2d;
        let (data, w, h) = if width > max || height > max {
            let scale = f64::from(max) / f64::from(width.max(height));
            let (w, h) = (
                ((f64::from(width) * scale) as u32).max(1),
                ((f64::from(height) * scale) as u32).max(1),
            );
            let mut out = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h {
                let sy = (u64::from(y) * u64::from(height) / u64::from(h)) as usize;
                for x in 0..w {
                    let sx = (u64::from(x) * u64::from(width) / u64::from(w)) as usize;
                    let at = (sy * width as usize + sx) * 4;
                    out.extend_from_slice(&rgba[at..at + 4]);
                }
            }
            (std::borrow::Cow::Owned(out), w, h)
        } else {
            (std::borrow::Cow::Borrowed(rgba), width, height)
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vif image"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vif image"),
            layout: &self.image_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.samplers[1]),
                },
            ],
        });
        self.pictures.insert(
            key,
            Picture {
                bind_group,
                used: self.frame,
            },
        );
    }

    fn glyph(&mut self, face: u16, glyph: u16) -> Option<Cached> {
        if let Some(cached) = self.glyphs.get(&(face, glyph)) {
            return *cached;
        }
        let cached = self.rasterize(face, glyph);
        self.glyphs.insert((face, glyph), cached);
        cached
    }

    fn rasterize(&mut self, face_index: u16, glyph: u16) -> Option<Cached> {
        let face = self.fonts.face(face_index);
        let px = self.fonts.px;
        let mut builder = self
            .scale
            .builder(face.font())
            .size(px)
            .hint(!cfg!(target_os = "macos"));
        if let Some(weight) = face.weight {
            builder = builder.variations(&[("wght", weight)][..]);
        }
        let mut scaler = builder.build();
        let mut render = Render::new(&[
            Source::ColorOutline(0),
            Source::ColorBitmap(StrikeWith::BestFit),
            Source::Outline,
        ]);
        render.format(Format::Alpha);
        if face.fake_bold {
            render.embolden((px / 28.0).max(0.5));
        }
        if face.fake_italic {
            render.transform(Some(Transform::skew(
                Angle::from_degrees(12.0),
                Angle::from_degrees(0.0),
            )));
        }
        let image = render.render(&mut scaler, glyph)?;
        let (w, h) = (image.placement.width, image.placement.height);
        if w == 0 || h == 0 {
            return None;
        }
        let color = matches!(image.content, Content::Color);
        let origin = self.insert(color, w, h, &image.data)?;
        Some(Cached {
            color,
            origin,
            size: [w, h],
            left: image.placement.left,
            top: image.placement.top,
        })
    }

    /// Places an image in an atlas, growing the atlas up to 4096² before starting it over.
    fn insert(&mut self, color: bool, w: u32, h: u32, data: &[u8]) -> Option<[u32; 2]> {
        let atlas = if color { &mut self.color } else { &mut self.mask };
        if let Some(origin) = atlas.insert(&self.queue, w, h, data) {
            return Some(origin);
        }
        let size = atlas.size();
        if size < 4096 {
            let format = atlas.texture.format();
            *atlas = Atlas::new(&self.device, format, size * 2);
            self.bind_group = bind_group(
                &self.device,
                &self.layout,
                &self.globals,
                &self.mask,
                &self.color,
                &self.samplers[0],
                &self.samplers[1],
            );
        }
        self.reset_atlas();
        let atlas = if color { &mut self.color } else { &mut self.mask };
        atlas.insert(&self.queue, w, h, data)
    }

    /// Uploads the scene; call before [`Renderer::draw`].
    pub fn prepare(&mut self, scene: &Scene, width: u32, height: u32) {
        let globals = Globals {
            viewport: [width.max(1) as f32, height.max(1) as f32],
            mask_size: [self.mask.size() as f32; 2],
            color_size: [self.color.size() as f32; 2],
            _pad: [0.0; 2],
        };
        self.queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
        self.back.write(&self.device, &self.queue, &scene.back);
        self.sprites.write(&self.device, &self.queue, &scene.mask);
        self.colors.write(&self.device, &self.queue, &scene.color);
        self.front.write(&self.device, &self.queue, &scene.front);
        self.over.write(&self.device, &self.queue, &scene.over);

        self.frame += 1;
        let mut images = scene.images.clone();
        images.sort_by_key(|(key, _)| *key);
        self.picture_runs.clear();
        for (i, (key, _)) in images.iter().enumerate() {
            match self.picture_runs.last_mut() {
                Some((last, _, count)) if last == key => *count += 1,
                _ => self.picture_runs.push((*key, i as u32, 1)),
            }
            if let Some(picture) = self.pictures.get_mut(key) {
                picture.used = self.frame;
            }
        }
        let sprites: Vec<Sprite> = images.into_iter().map(|(_, s)| s).collect();
        self.pictures_buffer.write(&self.device, &self.queue, &sprites);
        let frame = self.frame;
        self.pictures.retain(|_, p| frame - p.used < 600);
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_bind_group(0, &self.bind_group, &[]);
        let layers = [
            (&self.rect_pipeline, &self.back),
            (&self.mask_pipeline, &self.sprites),
            (&self.color_pipeline, &self.colors),
            (&self.rect_pipeline, &self.front),
            (&self.mask_pipeline, &self.over),
        ];
        for (i, (pipeline, instances)) in layers.into_iter().enumerate() {
            if i == 1 && !self.picture_runs.is_empty() {
                // Images sit on the cell backgrounds, under the text.
                pass.set_pipeline(&self.image_pipeline);
                pass.set_vertex_buffer(0, self.pictures_buffer.buffer.slice(..));
                for (key, start, count) in &self.picture_runs {
                    if let Some(picture) = self.pictures.get(key) {
                        pass.set_bind_group(1, &picture.bind_group, &[]);
                        pass.draw(0..4, *start..start + count);
                    }
                }
            }
            if instances.len == 0 {
                continue;
            }
            pass.set_pipeline(pipeline);
            pass.set_vertex_buffer(0, instances.buffer.slice(..));
            pass.draw(0..4, 0..instances.len);
        }
    }

    /// Prepares and draws the scene into `target` in one pass.
    pub fn render(&mut self, target: &wgpu::TextureView, width: u32, height: u32, clear: wgpu::Color, scene: &Scene) {
        self.prepare(scene, width, height);
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("vif frame"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vif pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            self.draw(&mut pass);
        }
        self.queue.submit([encoder.finish()]);
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    globals: &wgpu::Buffer,
    mask: &Atlas,
    color: &Atlas,
    nearest: &wgpu::Sampler,
    smooth: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("vif globals"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&mask.view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&color.view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(nearest),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(smooth),
            },
        ],
    })
}
