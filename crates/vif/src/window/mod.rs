use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::grid::Scroll;
use alacritty_terminal::term::TermMode;
use vif_config::{Config, Decorations, Diagnostic, OptionAsAlt, TabPosition};
use vif_render::{FontConfig, Renderer, Scene};
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, Modifiers, MouseButton, MouseScrollDelta};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersKeyState, NamedKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::window::{CursorIcon, Window, WindowAttributes};

use crate::app::{Gpu, UserEvent};
use crate::clipboard::Clipboard;
use crate::colors::Scheme;
use crate::hints::{Kind, Target};
use crate::input;
use crate::layout::{Area, Direction, Node, PaneId, Seam, neighbor};
use crate::layouts::{self, Saved, SavedTab, Shape};
use crate::pane::{Pane, term_config};

mod blocks;
mod events;
mod footer;
mod frame;
mod history;
mod hud;
mod keys;
pub use keys::open_settings;
pub mod looks;
mod mouse;
mod overlays;
mod panes;

use panes::first_session;
mod palette;
mod reply;
mod scrollbar;
mod sessions;
mod tabs;

use tabs::{bar_shown, insets};

const BLINK: Duration = Duration::from_millis(530);
const NOTICE: Duration = Duration::from_secs(12);

static WINDOWS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub struct Shared {
    pub proxy: EventLoopProxy<UserEvent>,
    pub term: (String, Option<PathBuf>),
    pub command: Option<Vec<String>>,
    /// Apolline's accent while it colors apps.
    pub outside: Option<vif_config::Rgb>,
}

struct Tab {
    root: Node,
    focus: PaneId,
    /// The pane this tab was opened from, focused again when it closes.
    back: Option<PaneId>,
    /// The focused pane takes the whole tab, the others wait behind it.
    zoomed: bool,
}

impl Tab {
    fn contains(&self, id: PaneId) -> bool {
        let mut ids = Vec::new();
        self.root.panes(&mut ids);
        ids.contains(&id)
    }
}

/// The link under the mouse while the link modifier is held.
#[derive(Clone, PartialEq)]
struct Link {
    pane: PaneId,
    /// The OSC 8 id, or where a plain URL sits.
    key: String,
    uri: String,
    /// Line and first and last column of a plain URL; OSC 8 links are underlined by the pane.
    span: Option<(usize, usize, usize)>,
    /// A path, opened the way hints open one.
    target: Option<Target>,
}

struct Notice {
    lines: Vec<String>,
    until: Instant,
    area: Area,
    /// The buttons under a config notice, and where they were drawn.
    buttons: Vec<(Answer, Area)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Answer {
    Fix,
    Settings,
    Close,
}

pub struct Win {
    pub window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    renderer: Renderer,
    scene: Scene,
    config: Arc<Config>,
    scheme: Scheme,
    /// The look of each rule in the config, in its order.
    rule_schemes: Vec<Scheme>,
    bindings: Vec<(String, String)>,
    tabs: Vec<Tab>,
    active: usize,
    panes: HashMap<PaneId, Pane>,
    areas: Vec<(PaneId, Area)>,
    tab_hits: Vec<(usize, Area)>,
    plus_hit: Option<Area>,
    font_delta: f32,
    mods: Modifiers,
    mouse: (f32, f32),
    pressed: Option<(PaneId, MouseButton)>,
    /// For what happens outside the window to reach it, like a notification's button.
    proxy: EventLoopProxy<UserEvent>,
    /// A divider the mouse is dragging, in the tab in front.
    dragging: Option<Seam>,
    on_divider: bool,
    clicks: (Instant, u8, (f32, f32)),
    focused: bool,
    blink: Option<Instant>,
    /// When running commands' times next move on.
    second: Option<Instant>,
    /// When to look again where the panes are, for rules and tab titles.
    places: Option<Instant>,
    palette: Option<palette::Palette>,
    history: Option<history::History>,
    cursor_on: bool,
    link: Option<Link>,
    preedit: String,
    notice: Option<Notice>,
    clipboard: Clipboard,
    shown: bool,
    doomed: Vec<PaneId>,
    /// This window's number in this vif, naming its saved layout.
    seq: u64,
    /// The layout as last saved.
    pub saved: Saved,
    /// The drop-down quick terminal rather than an ordinary window.
    pub quick: bool,
    /// The background picture as uploaded: its key and size.
    wallpaper: Option<(u64, (u32, u32))>,
    block: Option<blocks::BlockMode>,
    /// Running commands to tell about when they end, whatever `notify_after` says.
    notify: std::collections::HashSet<(PaneId, u32)>,
    /// A short word about something just done, and until when it shows.
    toast: Option<(String, Instant)>,
    ticks: HashMap<PaneId, scrollbar::Ticks>,
    sessions: Option<sessions::Sessions>,
    /// What the model is saying, or about to.
    reply: Option<reply::Reply>,
    hud: Option<hud::Hud>,
}

pub fn attributes(config: &Config) -> WindowAttributes {
    let mut attrs = Window::default_attributes()
        .with_title("vif")
        .with_visible(false)
        .with_transparent(config.window.opacity < 1.0)
        .with_blur(config.window.blur > 0)
        .with_inner_size(winit::dpi::LogicalSize::new(900.0, 560.0));
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::WindowAttributesExtMacOS;
        attrs = match config.window.decorations {
            Decorations::Full => attrs,
            Decorations::Transparent => attrs
                .with_titlebar_transparent(true)
                .with_fullsize_content_view(true)
                .with_title_hidden(true),
            Decorations::None => attrs.with_decorations(false),
        };
        attrs = attrs.with_option_as_alt(option_as_alt(config.terminal.option_as_alt));
    }
    #[cfg(target_os = "linux")]
    {
        use winit::platform::wayland::WindowAttributesExtWayland;
        // Tiling compositors like niri draw borders themselves; only "full" asks for a title bar.
        attrs = attrs.with_decorations(config.window.decorations == Decorations::Full);
        attrs = WindowAttributesExtWayland::with_name(attrs, "vif", "vif");
    }
    attrs
}

#[cfg(target_os = "macos")]
fn option_as_alt(v: OptionAsAlt) -> winit::platform::macos::OptionAsAlt {
    use winit::platform::macos::OptionAsAlt as Mac;
    match v {
        OptionAsAlt::None => Mac::None,
        OptionAsAlt::Left => Mac::OnlyLeft,
        OptionAsAlt::Right => Mac::OnlyRight,
        OptionAsAlt::Both => Mac::Both,
    }
}

fn font_config(config: &Config, scale: f32, delta: f32) -> FontConfig {
    FontConfig {
        family: config.font.family.clone(),
        size: (config.font.size + delta).clamp(4.0, 96.0),
        line_height: config.font.line_height,
        ligatures: config.font.ligatures,
        scale,
        faces: vif_render::Faces {
            weight: config.font.weight,
            bold_weight: config.font.bold_weight,
            bold_family: config.font.bold_family.clone(),
            italic_family: config.font.italic_family.clone(),
            features: config.font.feature_settings(),
        },
    }
}

impl Win {
    /// A window, taking over a saved window's sessions if given; `quick` makes it the drop-down.
    pub fn new(
        event_loop: &ActiveEventLoop,
        gpu: &mut Option<Gpu>,
        config: Arc<Config>,
        diagnostics: &[Diagnostic],
        shared: &Shared,
        restore: Option<Saved>,
        quick: bool,
    ) -> Result<Self, String> {
        let mut attrs = attributes(&config);
        if quick {
            attrs = attrs
                .with_decorations(false)
                .with_window_level(winit::window::WindowLevel::AlwaysOnTop);
            #[cfg(target_os = "linux")]
            {
                use winit::platform::wayland::WindowAttributesExtWayland;
                attrs = WindowAttributesExtWayland::with_name(attrs, "vif-quick", "vif-quick");
            }
        }
        let window = Arc::new(event_loop.create_window(attrs).map_err(|e| e.to_string())?);
        crate::app::mark("window");
        let scale = window.scale_factor() as f32;
        let (gpu, surface) = match gpu {
            Some(gpu) => {
                let surface = gpu.instance.create_surface(window.clone()).map_err(|e| e.to_string())?;
                (gpu, surface)
            }
            None => {
                let (new, surface) = Gpu::new(window.clone())?;
                (gpu.insert(new), surface)
            }
        };
        crate::app::mark("gpu");
        let caps = surface.get_capabilities(&gpu.adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let alpha_mode = [
            wgpu::CompositeAlphaMode::PreMultiplied,
            wgpu::CompositeAlphaMode::PostMultiplied,
        ]
        .into_iter()
        .find(|m| caps.alpha_modes.contains(m))
        .unwrap_or(caps.alpha_modes[0]);
        let renderer = Renderer::new(&gpu.device, &gpu.queue, format, font_config(&config, scale, 0.0));
        crate::app::mark("renderer");
        let m = renderer.metrics();
        let (top, bottom) = insets(&config, scale, 1);
        let width = (f32::from(config.window.columns) * m.cell_width + 2.0 * pad(&config, scale).0).round();
        let height =
            (f32::from(config.window.lines) * m.cell_height + 2.0 * pad(&config, scale).1 + top + bottom).round();
        let mut wanted = PhysicalSize::new(width as u32, height as u32);
        if quick && let Some(screen) = window.current_monitor().or_else(|| event_loop.primary_monitor()) {
            let size = screen.size();
            wanted = PhysicalSize::new(size.width, (size.height as f32 * config.quick.height) as u32);
            window.set_outer_position(screen.position());
        }
        // On Wayland the compositor places the quick terminal by its rules; asking for a size
        // would override them.
        let ask = !(quick && cfg!(target_os = "linux"));
        let size = if ask {
            window.request_inner_size(wanted).unwrap_or_else(|| window.inner_size())
        } else {
            window.inner_size()
        };
        #[cfg(target_os = "macos")]
        if quick {
            crate::quick::everywhere(&window);
        }
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 1,
            alpha_mode,
            view_formats: vec![],
        };
        surface.configure(&gpu.device, &surface_config);
        window.set_ime_allowed(true);
        window.set_cursor(CursorIcon::Text);

        let scheme = Scheme {
            palette: config.palette_with(shared.outside),
            opacity: config.window.opacity,
        };
        let rule_schemes = rule_schemes(&config, shared.outside);
        let clipboard = Clipboard::new(&window);
        let mut win = Self {
            window,
            surface,
            surface_config,
            device: gpu.device.clone(),
            renderer,
            scene: Scene::default(),
            bindings: config.bindings(),
            config,
            scheme,
            rule_schemes,
            tabs: Vec::new(),
            active: 0,
            panes: HashMap::new(),
            areas: Vec::new(),
            tab_hits: Vec::new(),
            plus_hit: None,
            font_delta: 0.0,
            mods: Modifiers::default(),
            mouse: (0.0, 0.0),
            pressed: None,
            proxy: shared.proxy.clone(),
            dragging: None,
            on_divider: false,
            clicks: (Instant::now(), 0, (0.0, 0.0)),
            focused: true,
            blink: None,
            second: None,
            places: None,
            palette: None,
            history: None,
            cursor_on: true,
            link: None,
            preedit: String::new(),
            notice: None,
            clipboard,
            shown: false,
            doomed: Vec::new(),
            seq: WINDOWS.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            saved: Saved {
                quick,
                ..Saved::default()
            },
            quick,
            wallpaper: None,
            block: None,
            notify: std::collections::HashSet::new(),
            toast: None,
            ticks: HashMap::new(),
            sessions: None,
            reply: None,
            hud: None,
        };
        win.set_notice(diagnostics);
        win.refresh_looks(shared);
        if let Some(saved) = restore {
            win.bring_back(shared, saved);
        }
        if win.tabs.is_empty() {
            win.new_tab(shared).map_err(|e| format!("can't start the shell: {e}"))?;
        }
        crate::app::mark("shell");
        Ok(win)
    }

    fn scale(&self) -> f32 {
        self.window.scale_factor() as f32
    }

    pub fn has_focus(&self) -> bool {
        self.focused
    }

    /// The theme's corner radius in pixels.
    fn radius(&self) -> f32 {
        self.config.window.radius * self.scale()
    }

    fn size(&self) -> (f32, f32) {
        (self.surface_config.width as f32, self.surface_config.height as f32)
    }

    fn content(&self) -> Area {
        let scale = self.scale();
        let (w, h) = self.size();
        let (top, bottom) = self.bands();
        let (px, py) = pad(&self.config, scale);
        Area {
            x: px,
            y: top + py,
            w: (w - 2.0 * px).max(1.0),
            h: (h - top - bottom - 2.0 * py).max(1.0),
        }
    }

    /// Pixels between split panes.
    fn gap(&self) -> f32 {
        let m = self.renderer.metrics();
        pad(&self.config, self.scale()).0.max(m.cell_width / 2.0).round()
    }

    fn relayout(&mut self) {
        let m = self.renderer.metrics();
        let gap = self.gap();
        self.areas.clear();
        let mut all = Vec::new();
        // Panes in the other tabs take the size they will show at, so their programs see it
        // and their lines stay where they are when the tab comes back.
        let mut hidden = Vec::new();
        for (i, tab) in self.tabs.iter().enumerate() {
            if i != self.active {
                tab.root.layout(self.content(), gap, &mut hidden);
            } else if tab.zoomed {
                tab.root.layout(self.content(), gap, &mut hidden);
                hidden.retain(|(id, _)| *id != tab.focus);
                all.push((tab.focus, self.content()));
            } else {
                tab.root.layout(self.content(), gap, &mut all);
            }
        }
        for (id, area) in hidden {
            if let Some(pane) = self.panes.get_mut(&id) {
                pane.place(area, area, m);
            }
        }
        let content = self.content();
        let (w, h) = self.size();
        let (inset, bottom) = self.bands();
        let bar_on_top =
            self.quick || (bar_shown(&self.config, self.tabs.len()) && self.config.tabs.position == TabPosition::Top);
        let top = if bar_on_top { inset } else { 0.0 };
        for (id, area) in &all {
            // Edge colors spread to the window's edges, never into a neighbouring pane.
            let left = if area.x <= content.x + 0.5 { 0.0 } else { area.x };
            let right = if area.x + area.w >= content.x + content.w - 0.5 {
                w
            } else {
                area.x + area.w
            };
            let up = if area.y <= content.y + 0.5 { top } else { area.y };
            let down = if area.y + area.h >= content.y + content.h - 0.5 {
                h - bottom
            } else {
                area.y + area.h
            };
            let outer = Area {
                x: left,
                y: up,
                w: right - left,
                h: down - up,
            };
            if let Some(pane) = self.panes.get_mut(id) {
                pane.place(*area, outer, m);
            }
        }
        self.areas = all;
        // A divider being dragged is written down once, when it's let go.
        if self.dragging.is_none() {
            self.remember();
        }
        self.window.request_redraw();
    }

    /// The sessions' layout, written down whenever it changes.
    fn remember(&mut self) {
        let mut tabs = Vec::new();
        let mut active = 0;
        for (i, tab) in self.tabs.iter().enumerate() {
            let Some(root) = self.shape(&tab.root) else {
                continue;
            };
            if i <= self.active {
                active = tabs.len();
            }
            let focus = match self.panes.get(&tab.focus).and_then(|p| p.session.clone()) {
                Some(id) => id,
                None => first_session(&root),
            };
            tabs.push(SavedTab { root, focus });
        }
        // Folders as last seen, so this costs no questions to the system.
        let places = self
            .panes
            .values()
            .filter_map(|p| Some((p.session.clone()?, p.last_location()?.cwd?)))
            .collect();
        let saved = Saved {
            tabs,
            active,
            quick: self.quick,
            places,
            restart: false,
        };
        if saved != self.saved {
            layouts::save(self.seq, &saved);
            self.saved = saved;
        }
    }

    fn shape(&self, node: &Node) -> Option<Shape> {
        match node {
            Node::Pane(id) => self.panes.get(id)?.session.clone().map(Shape::Pane),
            Node::Split {
                across,
                ratio,
                first,
                second,
            } => match (self.shape(first), self.shape(second)) {
                (Some(first), Some(second)) => Some(Shape::Split {
                    across: *across,
                    ratio: (ratio * 100.0).round() / 100.0,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (one, other) => one.or(other),
            },
        }
    }

    fn update_title(&self) {
        let title = self
            .focused_pane()
            .and_then(|p| p.title.clone())
            .unwrap_or_else(|| "vif".to_owned());
        self.window.set_title(&title);
    }

    pub fn apply(
        &mut self,
        config: Arc<Config>,
        diagnostics: &[Diagnostic],
        outside: Option<vif_config::Rgb>,
        shared: &Shared,
    ) {
        let scale = self.scale();
        self.scheme = Scheme {
            palette: config.palette_with(outside),
            opacity: config.window.opacity,
        };
        self.rule_schemes = rule_schemes(&config, outside);
        self.bindings = config.bindings();
        self.window.set_transparent(config.window.opacity < 1.0);
        self.window.set_blur(config.window.blur > 0);
        #[cfg(target_os = "linux")]
        self.window
            .set_decorations(config.window.decorations == Decorations::Full);
        #[cfg(target_os = "macos")]
        {
            use winit::platform::macos::WindowExtMacOS;
            self.window
                .set_option_as_alt(option_as_alt(config.terminal.option_as_alt));
        }
        let options = term_config(&config);
        for pane in self.panes.values_mut() {
            pane.term.lock().set_options(options.clone());
            pane.invalidate();
        }
        self.config = config;
        self.renderer
            .set_font(font_config(&self.config, scale, self.font_delta));
        self.set_notice(diagnostics);
        self.refresh_looks(shared);
        self.relayout();
    }

    /// Adds lines to the notice, or shows them in one.
    fn tell(&mut self, lines: Vec<String>) {
        if lines.is_empty() {
            return;
        }
        let until = Instant::now() + NOTICE;
        match &mut self.notice {
            Some(notice) => {
                notice.lines.extend(lines);
                notice.until = until;
            }
            None => {
                self.notice = Some(Notice {
                    lines,
                    until,
                    area: Area::default(),
                    buttons: Vec::new(),
                });
            }
        }
        self.window.request_redraw();
    }

    fn set_notice(&mut self, diagnostics: &[Diagnostic]) {
        if diagnostics.is_empty() {
            self.notice = None;
            return;
        }
        let mut lines = vec![format!(
            "vif.toml has {} problem{}. Everything else is applied.",
            diagnostics.len(),
            if diagnostics.len() == 1 { "" } else { "s" }
        )];
        for d in diagnostics.iter().take(4) {
            lines.push(format!("line {}  {d}", d.line));
        }
        if diagnostics.len() > 4 {
            lines.push(format!("…and {} more: run vif check", diagnostics.len() - 4));
        }
        let fix = diagnostics.iter().any(|d| d.fix.is_some()).then_some(Answer::Fix);
        let buttons = [fix, Some(Answer::Settings), Some(Answer::Close)]
            .into_iter()
            .flatten()
            .map(|b| (b, Area::default()))
            .collect();
        self.notice = Some(Notice {
            lines,
            until: Instant::now() + NOTICE,
            area: Area::default(),
            buttons,
        });
        self.window.request_redraw();
    }

    fn alt_meta(&self) -> bool {
        let state = self.mods.state();
        if !state.alt_key() {
            return false;
        }
        if !cfg!(target_os = "macos") {
            return true;
        }
        let left = self.mods.lalt_state() == ModifiersKeyState::Pressed;
        let right = self.mods.ralt_state() == ModifiersKeyState::Pressed;
        match self.config.terminal.option_as_alt {
            OptionAsAlt::None => false,
            OptionAsAlt::Both => true,
            OptionAsAlt::Left => left,
            OptionAsAlt::Right => right,
        }
    }
}

impl Pane {
    fn clear_selection_if_typing(&self) {
        let mut term = self.term.lock();
        if term.selection.as_ref().is_some_and(|s| !s.is_empty()) {
            term.selection = None;
        }
    }
}

/// Each rule's look: its theme's colors, then its own accent and background.
fn rule_schemes(config: &Config, outside: Option<vif_config::Rgb>) -> Vec<Scheme> {
    config
        .rules
        .iter()
        .map(|rule| {
            let mut palette = match &rule.theme {
                Some(theme) => Config::parse(&format!("theme = \"{theme}\"\n")).0.palette_with(outside),
                None => config.palette_with(outside),
            };
            if let Some(accent) = rule.accent {
                palette.accent = accent;
                palette.cursor = accent;
            }
            if let Some(background) = rule.background {
                palette.background = background;
            }
            Scheme {
                palette,
                opacity: config.window.opacity,
            }
        })
        .collect()
}

fn pad(config: &Config, scale: f32) -> (f32, f32) {
    (
        (f32::from(config.window.padding[0]) * scale).round(),
        (f32::from(config.window.padding[1]) * scale).round(),
    )
}
