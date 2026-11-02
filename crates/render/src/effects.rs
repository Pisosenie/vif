//! Shaders over the whole window, in Shadertoy's GLSL the way Ghostty takes them: the frame is
//! drawn into a texture, each shader reads the one before it, and a last copy puts it in the window.
//!
//! The frames between them are kept bottom row first, the way GL and Shadertoy keep theirs, so a
//! shader's `fragCoord` and texture coordinates both start at the bottom left; the copy turns
//! the frame the right way up.

use std::borrow::Cow;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PRELUDE: &str = "#version 450
layout(set = 0, binding = 0, std140) uniform VifGlobals {
    vec3 iResolution;
    float iTime;
    float iTimeDelta;
    float iFrameRate;
    int iFrame;
    float iTimeCursorChange;
    vec4 iMouse;
    vec4 iDate;
    vec4 iCurrentCursor;
    vec4 iPreviousCursor;
    vec4 iCurrentCursorColor;
    vec4 iPreviousCursorColor;
    vec3 iChannelResolution[4];
    float iSampleRate;
    vec4 iChannelTime;
    int iFocus;
    float iTimeFocus;
    vec3 iBackgroundColor;
    vec3 iForegroundColor;
};
layout(set = 0, binding = 1) uniform texture2D vif_screen;
layout(set = 0, binding = 2) uniform sampler vif_sampler;
#define iChannel0 sampler2D(vif_screen, vif_sampler)
layout(location = 0) out vec4 vif_color;
";

const MAIN: &str = "
void main() {
    vec4 color = vec4(0.0);
    mainImage(color, gl_FragCoord.xy);
    vif_color = color;
}
";

const VERTEX: &str = "
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}
";

/// Copies the last shader's frame to the window, the bottom row of the texture at the bottom.
const BLIT: &str = "
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@group(0) @binding(1) var frame: texture_2d<f32>;
@group(0) @binding(2) var frame_sampler: sampler;

@vertex
fn vs_blit(@builtin(vertex_index) i: u32) -> Out {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: Out;
    out.position = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    out.uv = p;
    return out;
}

@fragment
fn fs_blit(in: Out) -> @location(0) vec4<f32> {
    return textureSample(frame, frame_sampler, in.uv);
}
";

/// How long a shader that plays on cursor moves keeps drawing after one.
const AFTER_CURSOR: Duration = Duration::from_millis(1000);

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    resolution: [f32; 3],
    time: f32,
    time_delta: f32,
    frame_rate: f32,
    frame: i32,
    cursor_change: f32,
    mouse: [f32; 4],
    date: [f32; 4],
    cursor: [f32; 4],
    previous: [f32; 4],
    cursor_color: [f32; 4],
    previous_color: [f32; 4],
    channel_resolution: [[f32; 4]; 4],
    sample_rate: f32,
    _pad: [f32; 3],
    channel_time: [f32; 4],
    focus: i32,
    time_focus: f32,
    _pad2: [f32; 2],
    background: [f32; 4],
    foreground: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Animation {
    Still,
    AfterCursor,
    Always,
}

struct Target {
    view: wgpu::TextureView,
    size: (u32, u32),
}

pub struct Effects {
    passes: Vec<wgpu::RenderPipeline>,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    sampler: wgpu::Sampler,
    format: wgpu::TextureFormat,
    targets: Vec<Target>,
    animation: Animation,
    started: Instant,
    last: Instant,
    frame: i32,
    state: Uniforms,
    changed: Instant,
    /// The cursor now and before its last move, and the mouse, measured from the top.
    now: [f32; 4],
    was: [f32; 4],
    mouse: [f32; 2],
}

impl Effects {
    /// Compiles each (name, GLSL source); a shader that doesn't compile is an error naming its line.
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        sources: &[(String, String)],
    ) -> Result<Self, String> {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vif effect"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                crate::texture_entry(1),
                crate::sampler_entry(2),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vif effect"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let vertex = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vif effect vertex"),
            source: wgpu::ShaderSource::Wgsl(VERTEX.into()),
        });
        let mut passes = Vec::new();
        let mut animation = Animation::Still;
        for (name, source) in sources {
            let module = compile(name, source)?;
            animation = animation.max(if source.contains("iTimeCursorChange") {
                Animation::AfterCursor
            } else if source.contains("iTime") || source.contains("iFrame") {
                Animation::Always
            } else {
                Animation::Still
            });
            let fragment = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(name),
                source: wgpu::ShaderSource::Naga(Cow::Owned(module)),
            });
            passes.push(device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(name),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &vertex,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &fragment,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview: None,
                cache: None,
            }));
        }
        {
            let blit = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("vif blit"),
                source: wgpu::ShaderSource::Wgsl(BLIT.into()),
            });
            passes.push(device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("vif blit"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &blit,
                    entry_point: Some("vs_blit"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &blit,
                    entry_point: Some("fs_blit"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview: None,
                cache: None,
            }));
        }
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vif effect uniforms"),
            size: size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("vif effect"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let now = Instant::now();
        Ok(Self {
            passes,
            layout,
            uniforms,
            sampler,
            format,
            targets: Vec::new(),
            animation,
            started: now,
            last: now,
            frame: 0,
            state: Uniforms::default(),
            changed: now - AFTER_CURSOR,
            now: [0.0; 4],
            was: [0.0; 4],
            mouse: [0.0; 2],
        })
    }

    /// The cursor at its top-left corner and size in pixels from the top, and its color.
    pub fn cursor(&mut self, rect: Option<[f32; 4]>, color: [f32; 4]) {
        let Some(rect) = rect else {
            return;
        };
        if rect != self.now {
            self.was = if self.now[2] > 0.0 { self.now } else { rect };
            self.now = rect;
            self.state.previous_color = self.state.cursor_color;
            self.state.cursor_change = self.started.elapsed().as_secs_f32();
            self.changed = Instant::now();
        }
        self.state.cursor_color = color;
    }

    pub fn mouse(&mut self, x: f32, y: f32) {
        self.mouse = [x, y];
    }

    /// Whether the window has focus, and its colors.
    pub fn window(&mut self, focused: bool, background: [f32; 4], foreground: [f32; 4]) {
        if i32::from(focused) != self.state.focus {
            self.state.focus = i32::from(focused);
            self.state.time_focus = self.started.elapsed().as_secs_f32();
        }
        self.state.background = background;
        self.state.foreground = foreground;
    }

    /// Whether the shaders want another frame right after this one.
    pub fn playing(&self) -> bool {
        match self.animation {
            Animation::Always => true,
            Animation::AfterCursor => self.changed.elapsed() < AFTER_CURSOR,
            Animation::Still => false,
        }
    }

    /// Where the frame goes before the shaders read it, upside down as GL keeps textures.
    pub(crate) fn input(&mut self, device: &wgpu::Device, width: u32, height: u32) -> &wgpu::TextureView {
        let wanted = if self.passes.len() > 1 { 2 } else { 1 };
        if self.targets.len() != wanted || self.targets[0].size != (width, height) {
            self.targets = (0..wanted)
                .map(|_| target(device, self.format, width, height))
                .collect();
        }
        &self.targets[0].view
    }

    pub(crate) fn run(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, output: &wgpu::TextureView) {
        let Some((width, height)) = self.targets.first().map(|t| t.size) else {
            return;
        };
        let now = Instant::now();
        let delta = now - self.last;
        self.last = now;
        let day = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |d| (d.as_secs() % 86_400) as f32);
        let size = [width as f32, height as f32, 1.0];
        let up = |[x, y, w, h]: [f32; 4]| [x, size[1] - y, w, h];
        self.state.resolution = size;
        self.state.cursor = up(self.now);
        self.state.previous = up(self.was);
        self.state.mouse = [self.mouse[0], size[1] - self.mouse[1], 0.0, 0.0];
        self.state.time = (now - self.started).as_secs_f32();
        self.state.time_delta = delta.as_secs_f32();
        self.state.frame_rate = 1.0 / delta.as_secs_f32().max(1e-3);
        self.state.frame = self.frame;
        self.state.date = [0.0, 0.0, 0.0, day];
        self.state.channel_resolution[0] = [size[0], size[1], 1.0, 0.0];
        self.frame = self.frame.wrapping_add(1);
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&self.state));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("vif effects"),
        });
        let last = self.passes.len() - 1;
        for (i, pipeline) in self.passes.iter().enumerate() {
            let input = &self.targets[i % 2].view;
            let into = if i == last {
                output
            } else {
                &self.targets[(i + 1) % 2].view
            };
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("vif effect"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(input),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vif effect"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: into,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([encoder.finish()]);
    }
}

fn target(device: &wgpu::Device, format: wgpu::TextureFormat, width: u32, height: u32) -> Target {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("vif effect frame"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    Target {
        view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
        size: (width, height),
    }
}

/// Whether a shader compiles, without a GPU: for `vif check`.
pub fn check(name: &str, source: &str) -> Result<(), String> {
    compile(name, source).map(|_| ())
}

/// Wraps a Shadertoy `mainImage` in what wgpu needs and checks it, so a mistake is a message
/// with the shader's own line number instead of a crash.
fn compile(name: &str, source: &str) -> Result<naga::Module, String> {
    let body: String = source
        .lines()
        .map(|l| if l.trim_start().starts_with("#version") { "" } else { l })
        .collect::<Vec<_>>()
        .join("\n");
    let full = format!("{PRELUDE}{body}\n{MAIN}");
    let offset = PRELUDE.lines().count();
    let options = naga::front::glsl::Options::from(naga::ShaderStage::Fragment);
    let at = |span: naga::Span| {
        let line = span.location(&full).line_number as usize;
        line.saturating_sub(offset).max(1)
    };
    let module = naga::front::glsl::Frontend::default()
        .parse(&options, &full)
        .map_err(|e| {
            let first = e.errors.first();
            let line = first.map_or(1, |err| at(err.meta));
            let what = first.map_or_else(|| e.to_string(), |err| err.kind.to_string());
            format!("{name}, line {line}: {what}")
        })?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .map_err(|e| format!("{name}: {}", e.as_inner()))?;
    Ok(module)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Draws red over the top half through a shader that only copies, on every backend there is,
    /// and reads the frame back.
    #[test]
    fn shaders_see_the_frame_the_right_way_up() {
        use crate::{FontConfig, Rect, Renderer, Scene};
        let copy = "void mainImage(out vec4 c, in vec2 p) { c = texture(iChannel0, p / iResolution.xy); }";
        for backends in [wgpu::Backends::PRIMARY, wgpu::Backends::GL] {
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                backends,
                ..Default::default()
            });
            let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
                continue;
            };
            let Ok((device, queue)) = pollster::block_on(adapter.request_device(&Default::default())) else {
                continue;
            };
            let format = wgpu::TextureFormat::Rgba8Unorm;
            let font = FontConfig {
                family: String::new(),
                size: 13.0,
                line_height: 1.2,
                ligatures: true,
                scale: 1.0,
                faces: crate::Faces::default(),
            };
            let mut renderer = Renderer::new(&device, &queue, format, font);
            // One shader and a chain of two must both come out the right way up.
            for count in [1, 2] {
                let chain: Vec<(String, String)> = (0..count).map(|i| (format!("copy{i}"), copy.to_owned())).collect();
                renderer.set_effects(&chain).unwrap();
                let mut scene = Scene::default();
                scene.back.push(Rect::fill(0.0, 0.0, 8.0, 4.0, [1.0, 0.0, 0.0, 1.0]));
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: 8,
                        height: 8,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                let view = texture.create_view(&Default::default());
                renderer.render(&view, 8, 8, wgpu::Color::BLACK, &scene);
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: 256 * 8,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                let mut encoder = device.create_command_encoder(&Default::default());
                encoder.copy_texture_to_buffer(
                    texture.as_image_copy(),
                    wgpu::TexelCopyBufferInfo {
                        buffer: &buffer,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(256),
                            rows_per_image: Some(8),
                        },
                    },
                    texture.size(),
                );
                queue.submit([encoder.finish()]);
                buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                let data = buffer.slice(..).get_mapped_range();
                let backend = adapter.get_info().backend;
                assert_eq!(data[0], 255, "{backend:?}, {count} shaders: the top should be red");
                assert_eq!(
                    data[256 * 7],
                    0,
                    "{backend:?}, {count} shaders: the bottom should be black"
                );
                eprintln!("{backend:?} draws {count} shaders the right way up");
            }
        }
    }

    #[test]
    fn shadertoy_shaders_compile() {
        let tint = "void mainImage(out vec4 fragColor, in vec2 fragCoord) {
            vec2 uv = fragCoord / iResolution.xy;
            vec4 c = texture(iChannel0, uv);
            fragColor = vec4(c.rgb * (0.9 + 0.1 * sin(iTime)), c.a);
        }";
        assert!(compile("tint", tint).is_ok());
        for (name, source) in crate::BUILT_IN_SHADERS {
            if let Err(err) = compile(name, source) {
                panic!("{err}");
            }
        }
        let broken = "void mainImage(out vec4 fragColor, in vec2 fragCoord) {\n    fragColor = nope;\n}";
        let err = compile("broken", broken).unwrap_err();
        assert!(err.starts_with("broken, line 2"), "{err}");
    }
}
