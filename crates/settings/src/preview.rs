use iced::widget::shader::{self, Pipeline, Primitive, Viewport};
use iced::{Rectangle, mouse};
use vif_config::{BadgeShow, Badges, Config, CursorShape, Palette, Rgb, TabPosition, TabShow, TabStyle, Tabs};
use vif_render::chrome::{self, Bar, BarStyle};
use vif_render::{FontConfig, LineCell, RECT_CURL, Rect, Renderer, Scene, Style};

const BAR: f32 = 34.0;
/// What the preview's three tabs stand for: title, folder.
const TABS: [(&str, &str); 3] = [
    ("zsh", "~/src/vif"),
    ("nvim", "~/work/reporting-agent"),
    ("ssh devbox", "~"),
];

/// The settings window's preview: the terminal's own renderer drawing a sample session.
#[derive(Debug, Clone)]
pub struct Preview {
    font: FontConfig,
    palette: Palette,
    padding: [u16; 2],
    cursor: CursorShape,
    tabs: Tabs,
    badges: Badges,
    radius: f32,
}

impl Preview {
    /// Logical height that fits the sample at the configured size.
    pub fn height(config: &Config) -> f32 {
        let dpi = if cfg!(target_os = "macos") { 1.0 } else { 4.0 / 3.0 };
        let line = config.font.size * dpi * 1.2 * config.font.line_height;
        let bar = if config.tabs.show == TabShow::Never { 0.0 } else { BAR };
        (line * 8.4 + 2.0 * f32::from(config.window.padding[1]) + bar).clamp(160.0, 560.0)
    }

    pub fn new(config: &Config, palette: Palette) -> Self {
        Self {
            font: FontConfig {
                family: config.font.family.clone(),
                size: config.font.size,
                line_height: config.font.line_height,
                ligatures: config.font.ligatures,
                scale: 1.0,
            },
            palette,
            padding: config.window.padding,
            cursor: config.cursor.shape,
            tabs: config.tabs.clone(),
            badges: config.badges.clone(),
            radius: config.window.radius,
        }
    }
}

impl<Message> shader::Program<Message> for Preview {
    type State = ();
    type Primitive = Preview;

    fn draw(&self, _state: &(), _cursor: mouse::Cursor, _bounds: Rectangle) -> Preview {
        self.clone()
    }
}

pub struct Painter {
    renderer: Option<Renderer>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
}

impl Pipeline for Painter {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        Self {
            renderer: None,
            device: device.clone(),
            queue: queue.clone(),
            format,
        }
    }
}

impl Primitive for Preview {
    type Pipeline = Painter;

    fn prepare(
        &self,
        painter: &mut Painter,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        bounds: &Rectangle,
        viewport: &Viewport,
    ) {
        let scale = viewport.scale_factor();
        let font = FontConfig {
            scale,
            ..self.font.clone()
        };
        let renderer = painter
            .renderer
            .get_or_insert_with(|| Renderer::new(&painter.device, &painter.queue, painter.format, font.clone()));
        renderer.set_font(font);
        let linear = painter.format.is_srgb();
        let color = |c: Rgb| {
            tone(
                [
                    f32::from(c.r) / 255.0,
                    f32::from(c.g) / 255.0,
                    f32::from(c.b) / 255.0,
                    1.0,
                ],
                linear,
            )
        };
        let (w, h) = (bounds.width * scale, bounds.height * scale);
        let m = renderer.metrics();
        let p = &self.palette;
        let mut scene = Scene::default();
        scene
            .back
            .push(Rect::fill(0.0, 0.0, w, h, color(p.background)).rounded(12.0 * scale));
        let bar = if self.tabs.show == TabShow::Never {
            0.0
        } else {
            BAR * scale
        };
        let on_top = self.tabs.position == TabPosition::Top;
        if bar > 0.0 {
            let titles: Vec<String> = TABS
                .iter()
                .enumerate()
                .map(|(i, (title, cwd))| {
                    let dir = cwd.rsplit('/').next().unwrap_or(cwd);
                    let text = self
                        .tabs
                        .title
                        .replace("{index}", &(i + 1).to_string())
                        .replace("{title}", title)
                        .replace("{cwd}", cwd)
                        .replace("{dir}", dir);
                    text.split_whitespace().collect::<Vec<_>>().join(" ")
                })
                .collect();
            let tab_h = (bar - 8.0 * scale).max(m.cell_height + 4.0 * scale);
            let y = if on_top {
                (bar - tab_h) / 2.0
            } else {
                h - bar + (bar - tab_h) / 2.0
            };
            let tabs = Bar {
                x: 8.0 * scale,
                y: y.round(),
                w: w - 16.0 * scale,
                h: tab_h,
                tabs: &titles,
                active: 0,
                style: match self.tabs.style {
                    TabStyle::Pills => BarStyle::Pills,
                    TabStyle::Underline => BarStyle::Underline,
                    TabStyle::Powerline => BarStyle::Powerline,
                    TabStyle::Plain => BarStyle::Plain,
                },
                fg: color(p.foreground),
                bg: color(p.background),
                accent: color(p.accent),
                radius: self.radius,
                scale,
            };
            chrome::tab_bar(renderer, &tabs, &mut scene);
        }
        let top = if on_top { bar } else { 0.0 };
        let (x0, mut y) = (
            f32::from(self.padding[0]) * scale,
            f32::from(self.padding[1]) * scale + top,
        );

        let fg = color(p.foreground);
        let ansi = |i: usize| color(p.ansi[i]);
        type Part<'a> = (&'a str, [f32; 4], Style);
        let lines: [&[Part]; 5] = [
            &[
                ("~/src/vif", ansi(4), Style::Regular),
                (" main ", ansi(5), Style::Regular),
                ("❯ ", ansi(2), Style::Bold),
                ("eza --icons=never", fg, Style::Regular),
            ],
            &[
                ("assets  ", ansi(6), Style::Bold),
                ("crates  ", ansi(6), Style::Bold),
                ("Cargo.toml  ", fg, Style::Regular),
                ("README.md", ansi(3), Style::Regular),
            ],
            &[
                ("fn ", ansi(5), Style::Regular),
                ("render", ansi(4), Style::Regular),
                ("(a: u8) -> bool { a != 0 && a >= 2 }", fg, Style::Regular),
            ],
            &[
                ("bold ", fg, Style::Bold),
                ("italic ", fg, Style::Italic),
                (
                    "dim ",
                    tone(mix(p.foreground, p.background, 0.4), linear),
                    Style::Regular,
                ),
                ("undercurl", fg, Style::Regular),
            ],
            &[("❯ ", ansi(2), Style::Bold)],
        ];
        for (row, parts) in lines.iter().enumerate() {
            let mut cells = Vec::new();
            let texts: Vec<(String, [f32; 4], Style)> = parts
                .iter()
                .flat_map(|(text, color, style)| text.chars().map(move |c| (c.to_string(), *color, *style)))
                .collect();
            for (text, color, style) in &texts {
                cells.push(LineCell {
                    text,
                    style: *style,
                    color: *color,
                    wide: false,
                });
            }
            renderer.draw_line(&cells, x0, y, &mut scene);
            if row == 0 && self.badges.show == BadgeShow::All {
                let mark = format!("{} 1.2s", self.badges.ok).trim().to_owned();
                let badge_w = mark.chars().count() as f32 * m.cell_width;
                let tint = tone(mix(p.foreground, p.background, 0.5), linear);
                chrome::label(renderer, &mark, w - x0 - badge_w, y, tint, Style::Regular, &mut scene);
            }
            if row == 3 {
                let start = x0 + 16.0 * m.cell_width;
                let s = m.stroke;
                scene.front.push(
                    Rect::fill(start, y + m.underline - s, 9.0 * m.cell_width, s * 3.0, ansi(1)).kind(RECT_CURL, s),
                );
            }
            if row == 4 {
                let x = x0 + 2.0 * m.cell_width;
                let cursor = color(p.cursor);
                let beam = (m.cell_width / 8.0).round().max(2.0);
                scene.back.push(match self.cursor {
                    CursorShape::Block => Rect::fill(x, y, m.cell_width, m.cell_height, cursor),
                    CursorShape::Beam => Rect::fill(x, y, beam, m.cell_height, cursor),
                    CursorShape::Underline => Rect::fill(x, y + m.cell_height - beam, m.cell_width, beam, cursor),
                });
            }
            y += m.cell_height;
        }
        y += m.cell_height * 0.5;
        let swatch = (m.cell_width * 2.0).round();
        for i in 0..16 {
            let x = x0 + (i % 8) as f32 * (swatch + 4.0 * scale);
            let yy = y + (i / 8) as f32 * (m.cell_height * 0.8 + 4.0 * scale);
            scene
                .back
                .push(Rect::fill(x, yy, swatch, m.cell_height * 0.8, ansi(i)).rounded(3.0 * scale));
        }
        renderer.prepare(&scene, w.round() as u32, h.round() as u32);
    }

    fn draw(&self, painter: &Painter, pass: &mut wgpu::RenderPass<'_>) -> bool {
        if let Some(renderer) = &painter.renderer {
            renderer.draw(pass);
        }
        true
    }
}

fn mix(a: Rgb, b: Rgb, t: f32) -> [f32; 4] {
    let f = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t) / 255.0;
    [f(a.r, b.r), f(a.g, b.g), f(a.b, b.b), 1.0]
}

fn tone(c: [f32; 4], linear: bool) -> [f32; 4] {
    if !linear {
        return c;
    }
    let l = |v: f32| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    [l(c[0]), l(c[1]), l(c[2]), c[3]]
}
