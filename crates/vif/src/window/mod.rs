use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::event::Event;
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::term::{ClipboardType, TermMode};
use alacritty_terminal::vte::ansi::Rgb as TermRgb;
use vif_config::{Config, Decorations, Diagnostic, Divider, OptionAsAlt, TabPosition};
use vif_render::{FontConfig, Rect, Renderer, Scene};
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, KeyEvent, Modifiers, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersKeyState, NamedKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::window::{CursorIcon, UserAttentionType, Window, WindowAttributes};

use crate::app::{Gpu, UserEvent};
use crate::clipboard::Clipboard;
use crate::colors::{Scheme, mix};
use crate::hints::{Kind, Target};
use crate::input;
use crate::layout::{Area, Direction, Node, PaneId, neighbor};
use crate::layouts::{self, Saved, SavedTab, Shape};
use crate::marks;
use crate::pane::{Look, Pane, Spawn, term_config};

mod blocks;
mod footer;
mod history;
mod hud;
mod keys;
pub mod looks;
mod mouse;
mod overlays;
mod palette;
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
        let mut renderer = Renderer::new(&gpu.device, &gpu.queue, format, font_config(&config, scale, 0.0));
        renderer.set_gl(gpu.adapter.get_info().backend == wgpu::Backend::Gl);
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

    fn relayout(&mut self) {
        let m = self.renderer.metrics();
        let gap = pad(&self.config, self.scale()).0.max(m.cell_width / 2.0).round();
        self.areas.clear();
        let mut all = Vec::new();
        if let Some(tab) = self.tabs.get(self.active) {
            tab.root.layout(self.content(), gap, &mut all);
        }
        // Panes in the other tabs take the size they will show at, so their programs see it
        // and their lines stay where they are when the tab comes back.
        let mut hidden = Vec::new();
        for (i, tab) in self.tabs.iter().enumerate() {
            if i != self.active {
                tab.root.layout(self.content(), gap, &mut hidden);
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
        self.remember();
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
        let saved = Saved {
            tabs,
            active,
            quick: self.quick,
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

    /// Takes over a saved window's sessions, in its tabs and splits.
    fn bring_back(&mut self, shared: &Shared, saved: Saved) {
        for tab in saved.tabs {
            let Some(root) = self.build(shared, tab.root) else {
                continue;
            };
            let mut ids = Vec::new();
            root.panes(&mut ids);
            let focus = ids
                .iter()
                .copied()
                .find(|id| self.panes.get(id).and_then(|p| p.session.as_deref()) == Some(tab.focus.as_str()))
                .unwrap_or(ids[0]);
            self.tabs.push(Tab {
                root,
                focus,
                back: None,
            });
        }
        self.active = saved.active.min(self.tabs.len().saturating_sub(1));
        if !self.tabs.is_empty() {
            self.relayout();
            self.update_title();
        }
    }

    fn build(&mut self, shared: &Shared, shape: Shape) -> Option<Node> {
        match shape {
            Shape::Pane(session) => match self.spawn_pane(shared, None, None, Some(session)) {
                Ok(id) => Some(Node::Pane(id)),
                Err(err) => {
                    log::warn!("can't take a session back: {err}");
                    None
                }
            },
            Shape::Split {
                across,
                ratio,
                first,
                second,
            } => match (self.build(shared, *first), self.build(shared, *second)) {
                (Some(first), Some(second)) => Some(Node::Split {
                    across,
                    ratio,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (one, other) => one.or(other),
            },
        }
    }

    fn spawn_pane(
        &mut self,
        shared: &Shared,
        run: Option<&[String]>,
        cwd: Option<PathBuf>,
        attach: Option<String>,
    ) -> std::io::Result<PaneId> {
        let id = crate::app::next_pane();
        let spawn = Spawn {
            config: &self.config,
            cwd: cwd.or_else(|| self.focused_pane().and_then(Pane::cwd)),
            term: &shared.term,
            command: run.or_else(|| shared.command.as_deref().filter(|_| self.panes.is_empty())),
            proxy: shared.proxy.clone(),
            window: self.window.id(),
            attach,
        };
        let pane = Pane::spawn(id, spawn, self.content(), self.renderer.metrics())?;
        self.panes.insert(id, pane);
        Ok(id)
    }

    fn new_tab(&mut self, shared: &Shared) -> std::io::Result<()> {
        self.push_tab(shared, None)
    }

    /// A tab for a session a closed window left running.
    fn attach_tab(&mut self, shared: &Shared, session: String) -> std::io::Result<()> {
        self.push_tab(shared, Some(session))
    }

    fn push_tab(&mut self, shared: &Shared, attach: Option<String>) -> std::io::Result<()> {
        let id = self.spawn_pane(shared, None, None, attach)?;
        self.tabs.push(Tab {
            root: Node::Pane(id),
            focus: id,
            back: None,
        });
        self.active = self.tabs.len() - 1;
        self.relayout();
        self.update_title();
        Ok(())
    }

    /// A tab next to this one that runs `run` (or the shell) and comes back here when it ends.
    fn open_tab(&mut self, shared: &Shared, run: Option<&[String]>, cwd: Option<PathBuf>) {
        let back = self.tabs.get(self.active).map(|t| t.focus);
        let id = match self.spawn_pane(shared, run, cwd, None) {
            Ok(id) => id,
            Err(err) => return log::warn!("can't open a tab: {err}"),
        };
        let at = (self.active + 1).min(self.tabs.len());
        self.tabs.insert(
            at,
            Tab {
                root: Node::Pane(id),
                focus: id,
                back,
            },
        );
        self.active = at;
        self.relayout();
        self.update_title();
    }

    fn split(&mut self, shared: &Shared, across: bool) {
        let Some(focus) = self.tabs.get(self.active).map(|t| t.focus) else {
            return;
        };
        let Ok(id) = self.spawn_pane(shared, None, None, None) else {
            return;
        };
        let tab = &mut self.tabs[self.active];
        tab.root.split(focus, id, across);
        tab.focus = id;
        self.relayout();
    }

    /// Closes a pane, ending its shell; returns true when the window has nothing left.
    pub fn close_pane(&mut self, id: PaneId) -> bool {
        self.remove_pane(id, true)
    }

    /// Takes a pane out of the window and leaves its session running; true when nothing is left.
    pub(super) fn detach_pane(&mut self, id: PaneId) -> bool {
        self.remove_pane(id, false)
    }

    fn remove_pane(&mut self, id: PaneId, end: bool) -> bool {
        if let Some(pane) = self.panes.remove(&id) {
            if end {
                pane.end();
            } else {
                pane.shutdown();
            }
        }
        self.forget_ticks(id);
        if let Some(index) = self.tabs.iter().position(|t| t.contains(id)) {
            let tab = &mut self.tabs[index];
            if tab.root.remove(id) {
                if tab.focus == id {
                    let mut ids = Vec::new();
                    tab.root.panes(&mut ids);
                    tab.focus = ids[0];
                }
            } else {
                let closed = self.tabs.remove(index);
                let was_active = index == self.active;
                if index < self.active {
                    self.active -= 1;
                }
                if was_active && let Some(home) = closed.back.and_then(|b| self.tabs.iter().position(|t| t.contains(b)))
                {
                    self.active = home;
                }
                self.active = self.active.min(self.tabs.len().saturating_sub(1));
            }
        }
        if self.tabs.is_empty() {
            layouts::forget(self.seq);
            return true;
        }
        self.relayout();
        self.update_title();
        false
    }

    pub fn shutdown(&mut self) {
        for pane in self.panes.values() {
            pane.shutdown();
        }
    }

    fn focused_pane(&self) -> Option<&Pane> {
        self.tabs.get(self.active).and_then(|t| self.panes.get(&t.focus))
    }

    fn focus(&mut self, id: PaneId) {
        if let Some(tab) = self.tabs.get_mut(self.active)
            && tab.focus != id
        {
            let old = tab.focus;
            tab.focus = id;
            for (pane, on) in [(old, false), (id, true)] {
                if let Some(pane) = self.panes.get_mut(&pane) {
                    if pane.mode().contains(TermMode::FOCUS_IN_OUT) {
                        pane.write(if on { &b"\x1b[I"[..] } else { &b"\x1b[O"[..] });
                    }
                    pane.invalidate();
                }
            }
            self.update_title();
            self.window.request_redraw();
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
                    area: Area {
                        x: 0.0,
                        y: 0.0,
                        w: 0.0,
                        h: 0.0,
                    },
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
        self.notice = Some(Notice {
            lines,
            until: Instant::now() + NOTICE,
            area: Area {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            },
        });
        self.window.request_redraw();
    }

    /// When the window next needs to wake up by itself.
    pub fn deadline(&self) -> Option<Instant> {
        let blink = self.blink.filter(|_| self.config.cursor.blink && self.focused);
        let notice = self.notice.as_ref().map(|n| n.until);
        let second = self.second.filter(|_| self.ticking());
        let places = self.places.filter(|_| self.follows_places());
        let toast = self.toast.as_ref().map(|t| t.1);
        [blink, notice, second, places, toast].into_iter().flatten().min()
    }

    /// Rules or tabs depend on where the panes are and what runs in them.
    fn follows_places(&self) -> bool {
        let tabs = &self.config.tabs;
        let shown = bar_shown(&self.config, self.tabs.len());
        let title = tabs.title.contains("{cwd}") || tabs.title.contains("{dir}");
        !self.config.rules.is_empty() || (shown && (tabs.detail || title))
    }

    fn ticking(&self) -> bool {
        self.areas
            .iter()
            .any(|(id, _)| self.panes.get(id).is_some_and(|p| p.ticking))
    }

    pub fn tick(&mut self, now: Instant) {
        if let Some(at) = self.blink
            && now >= at
            && self.config.cursor.blink
        {
            self.cursor_on = !self.cursor_on;
            self.blink = Some(now + BLINK);
            self.window.request_redraw();
        }
        if self.toast.as_ref().is_some_and(|t| now >= t.1) {
            self.toast = None;
            self.window.request_redraw();
        }
        if self.notice.as_ref().is_some_and(|n| now >= n.until) {
            self.notice = None;
            self.window.request_redraw();
        }
        if !self.follows_places() {
            self.places = None;
        } else if self.places.is_none_or(|at| now >= at) {
            self.places = Some(now + Duration::from_secs(1));
            let mut moved = false;
            for pane in self.panes.values_mut() {
                let before = pane.last_location();
                moved |= before.as_ref() != Some(&pane.location());
            }
            if moved {
                self.window.request_redraw();
            }
        }
        if !self.ticking() {
            self.second = None;
        } else if self.second.is_none_or(|at| now >= at) {
            self.second = Some(now + Duration::from_secs(1));
            self.window.request_redraw();
        }
    }

    /// Keeps finished commands in the archive, and tells about long ones that ended where
    /// nobody was looking.
    fn report_finished(&mut self, id: PaneId) {
        let Some(pane) = self.panes.get_mut(&id) else {
            return;
        };
        let done = pane.blocks.take_done();
        if done.is_empty() {
            return;
        }
        let keep = self.config.terminal.archive_days > 0 && !pane.replaying();
        let place = if keep {
            pane.location()
        } else {
            crate::rules::Place::default()
        };
        let after = Duration::from_secs(u64::from(self.config.terminal.notify_after));
        let seen = self.focused && self.areas.iter().any(|(p, _)| *p == id);
        for block in done {
            let Some((took, exit)) = pane.blocks.get(block).and_then(|b| b.ended) else {
                continue;
            };
            let typed = pane.command(block);
            if keep && let Some(command) = typed.as_deref().filter(|c| !c.starts_with(' ')) {
                crate::archive::record(&crate::archive::Entry {
                    at: crate::archive::now(),
                    took: took.as_secs_f32(),
                    exit,
                    cwd: place.cwd.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
                    host: place.host.clone(),
                    command: command.trim().to_owned(),
                    output: crate::archive::trimmed(&pane.output(block).unwrap_or_default()),
                });
            }
            let asked = self.notify.remove(&(id, block));
            if !asked && (after.is_zero() || seen || took < after || pane.replaying()) {
                continue;
            }
            let command = typed.map_or_else(|| "A command".to_owned(), |c| c.trim().to_owned());
            crate::notify::send(
                &format!("{command} finished"),
                &format!("exit {exit} · {}", marks::spoken(took)),
            );
            self.window
                .request_user_attention(Some(UserAttentionType::Informational));
        }
    }

    fn wake_cursor(&mut self) {
        self.cursor_on = true;
        self.blink = Some(Instant::now() + BLINK);
    }

    pub fn term_event(&mut self, pane: PaneId, event: Event) -> bool {
        match event {
            Event::Wakeup => {
                self.window.request_redraw();
                self.report_finished(pane);
            }
            Event::CursorBlinkingChange | Event::MouseCursorDirty => self.window.request_redraw(),
            Event::Title(title) => {
                if let Some(p) = self.panes.get_mut(&pane) {
                    p.title = Some(title);
                }
                self.update_title();
                self.window.request_redraw();
            }
            Event::ResetTitle => {
                if let Some(p) = self.panes.get_mut(&pane) {
                    p.title = None;
                }
                self.update_title();
            }
            Event::ClipboardStore(kind, text) => match kind {
                ClipboardType::Selection => self.clipboard.set_primary(text),
                ClipboardType::Clipboard => self.clipboard.set(text),
            },
            Event::ClipboardLoad(kind, format) => {
                let text = match kind {
                    ClipboardType::Selection => self.clipboard.get_primary().unwrap_or_default(),
                    ClipboardType::Clipboard => self.clipboard.get(),
                };
                if let Some(p) = self.panes.get(&pane) {
                    p.write(format(&text).into_bytes());
                }
            }
            Event::ColorRequest(index, format) => {
                let color = self.color_for(index);
                if let Some(p) = self.panes.get(&pane) {
                    p.write(format(color).into_bytes());
                }
            }
            Event::PtyWrite(text) => {
                if let Some(p) = self.panes.get(&pane) {
                    p.write(text.into_bytes());
                }
            }
            Event::TextAreaSizeRequest(format) => {
                if let Some(p) = self.panes.get(&pane) {
                    let m = self.renderer.metrics();
                    let size = alacritty_terminal::event::WindowSize {
                        num_lines: p.lines as u16,
                        num_cols: p.cols as u16,
                        cell_width: m.cell_width as u16,
                        cell_height: m.cell_height as u16,
                    };
                    p.write(format(size).into_bytes());
                }
            }
            Event::Bell => {
                if !self.focused {
                    self.window
                        .request_user_attention(Some(UserAttentionType::Informational));
                }
            }
            // Closing before the first frame would make `vif -e true` look faster than it is.
            Event::Exit | Event::ChildExit(_) if !self.shown => {
                self.doomed.push(pane);
                self.window.request_redraw();
            }
            Event::Exit | Event::ChildExit(_) => return self.close_pane(pane),
        }
        false
    }

    fn color_for(&self, index: usize) -> TermRgb {
        let p = &self.scheme.palette;
        let c = match index {
            0..=15 => p.ansi[index],
            256 => p.foreground,
            257 => p.background,
            258 => p.cursor,
            _ => p.foreground,
        };
        TermRgb { r: c.r, g: c.g, b: c.b }
    }

    /// Handles a window event; returns true when the window should close.
    pub fn event(&mut self, event: WindowEvent, shared: &Shared) -> bool {
        match event {
            WindowEvent::CloseRequested => return true,
            WindowEvent::Resized(size) => {
                self.surface_config.width = size.width.max(1);
                self.surface_config.height = size.height.max(1);
                self.surface.configure(&self.device, &self.surface_config);
                self.relayout();
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                let scale = self.scale();
                self.renderer
                    .set_font(font_config(&self.config, scale, self.font_delta));
                self.relayout();
            }
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                self.wake_cursor();
                if let Some(pane) = self.focused_pane()
                    && pane.mode().contains(TermMode::FOCUS_IN_OUT)
                {
                    pane.write(if focused { &b"\x1b[I"[..] } else { &b"\x1b[O"[..] });
                }
                for pane in self.panes.values_mut() {
                    pane.invalidate();
                }
                self.window.request_redraw();
            }
            WindowEvent::ModifiersChanged(mods) => {
                self.mods = mods;
                self.hover();
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } => {
                return self.key(event, shared);
            }
            WindowEvent::Ime(ime) => match ime {
                Ime::Preedit(text, _) => {
                    self.preedit = text;
                    self.window.request_redraw();
                }
                Ime::Commit(text) if self.palette.is_some() => {
                    self.preedit.clear();
                    self.palette_type(&text);
                }
                Ime::Commit(text) if self.history.is_some() => {
                    self.preedit.clear();
                    self.history_type(&text);
                }
                Ime::Commit(text) if self.sessions.is_some() => {
                    self.preedit.clear();
                    self.sessions_type(&text);
                }
                Ime::Commit(text) => {
                    self.preedit.clear();
                    let id = self.tabs.get(self.active).map(|t| t.focus);
                    match id.and_then(|id| self.panes.get_mut(&id)) {
                        Some(pane) if pane.hints.is_some() => {}
                        Some(pane) if pane.search.is_some() => {
                            pane.search_edit(|q| q.push_str(&text));
                            self.window.request_redraw();
                        }
                        Some(pane) => pane.input(text.into_bytes()),
                        None => {}
                    }
                }
                _ => {}
            },
            WindowEvent::CursorMoved { position, .. } => self.mouse_moved(position),
            WindowEvent::CursorLeft { .. } => {
                if self.link.take().is_some() {
                    self.invalidate_all();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => self.mouse_button(state, button, shared),
            WindowEvent::MouseWheel { delta, .. } => self.wheel(delta),
            WindowEvent::RedrawRequested => {
                self.redraw();
                if self.shown {
                    for pane in std::mem::take(&mut self.doomed) {
                        if self.close_pane(pane) {
                            return true;
                        }
                    }
                }
            }
            _ => {}
        }
        false
    }

    fn invalidate_all(&mut self) {
        for pane in self.panes.values_mut() {
            pane.invalidate();
        }
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

    fn redraw(&mut self) {
        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                self.surface.configure(&self.device, &self.surface_config);
                self.window.request_redraw();
                return;
            }
            Err(err) => {
                log::warn!("no frame: {err}");
                return;
            }
        };
        let mut scene = std::mem::take(&mut self.scene);
        scene.clear();
        self.draw_wallpaper(&mut scene);
        self.compose(&mut scene);
        self.feed_effects();
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bg = self.scheme.bg();
        let a = f64::from(self.scheme.opacity);
        let clear = wgpu::Color {
            r: f64::from(bg[0]) * a,
            g: f64::from(bg[1]) * a,
            b: f64::from(bg[2]) * a,
            a,
        };
        self.renderer.render(
            &view,
            self.surface_config.width,
            self.surface_config.height,
            clear,
            &scene,
        );
        self.window.pre_present_notify();
        frame.present();
        self.hud_presented();
        self.scene = scene;
        if self.effects_playing() {
            self.window.request_redraw();
        }
        if !self.shown {
            self.shown = true;
            self.window.set_visible(true);
            self.wake_cursor();
            crate::app::mark("first frame");
        }
        if let Some((col, line)) = self
            .focused_pane()
            .and_then(|p| p.cursor_cell().map(|c| (p.area, c)))
            .map(|(a, (c, l))| {
                let m = self.renderer.metrics();
                (a.x + c as f32 * m.cell_width, a.y + (l + 1) as f32 * m.cell_height)
            })
        {
            self.window.set_ime_cursor_area(
                PhysicalPosition::new(col as f64, line as f64),
                PhysicalSize::new(1u32, 1u32),
            );
        }
    }

    fn compose(&mut self, scene: &mut Scene) {
        let m = self.renderer.metrics();
        let focus = self.tabs.get(self.active).map(|t| t.focus);
        let link = self.link.as_ref().filter(|l| l.span.is_none()).map(|l| l.key.clone());
        let scheme = self.scheme;
        let cursor_on = self.cursor_on || !self.config.cursor.blink;
        let split = self.areas.len() > 1;
        let mut labels = Vec::new();
        for (id, area) in self.areas.clone() {
            let Some(pane) = self.panes.get_mut(&id) else {
                continue;
            };
            let rule = if self.config.rules.is_empty() {
                None
            } else {
                crate::rules::pick(&self.config.rules, &pane.location())
            };
            if rule != pane.rule {
                pane.rule = rule;
                pane.invalidate();
            }
            let own = rule.and_then(|i| self.rule_schemes.get(i).copied()).unwrap_or(scheme);
            if own.bg() != scheme.bg() {
                let o = pane.outer;
                let mut bg = own.bg();
                bg[3] = own.opacity;
                scene.back.push(Rect::fill(o.x, o.y, o.w, o.h, bg));
            }
            if let Some(label) = rule.and_then(|i| self.config.rules[i].label.clone()) {
                labels.push((area, label, own));
            }
            let focused = self.focused && Some(id) == focus;
            let look = Look {
                scheme: &own,
                focused,
                cursor_on,
                link: link.as_deref(),
                badges: &self.config.badges,
            };
            pane.draw(&mut self.renderer, &look, scene);
            if let Some(Link {
                pane: owner,
                span: Some((line, start, end)),
                target,
                ..
            }) = &self.link
                && *owner == id
            {
                let x = area.x + *start as f32 * m.cell_width;
                let y = area.y + *line as f32 * m.cell_height + m.underline;
                let w = (*end - *start + 1) as f32 * m.cell_width;
                let under = Rect::fill(x, y, w, m.stroke, scheme.accent());
                scene.front.push(match target {
                    Some(_) => under.kind(vif_render::RECT_DOTTED, m.stroke.max(1.0)),
                    None => under,
                });
            }
            if split && Some(id) != focus && self.config.panes.dim > 0.0 {
                let mut dim = scheme.bg();
                dim[3] = self.config.panes.dim;
                scene.front.push(Rect::fill(area.x, area.y, area.w, area.h, dim));
            }
        }
        if split && self.config.panes.divider == Divider::Line {
            let line = mix(scheme.fg(), scheme.bg(), 0.86);
            let gap = pad(&self.config, self.scale()).0.max(m.cell_width / 2.0).round();
            for (_, a) in &self.areas {
                let right = a.x + a.w;
                if right + gap < self.size().0 - 1.0 {
                    scene
                        .back
                        .push(Rect::fill((right + gap / 2.0).floor(), a.y, 1.0, a.h, line));
                }
                let bottom = a.y + a.h;
                if bottom + gap < self.size().1 - 1.0 {
                    scene
                        .back
                        .push(Rect::fill(a.x, (bottom + gap / 2.0).floor(), a.w, 1.0, line));
                }
            }
        }
        for (area, label, own) in labels {
            self.draw_label(&area, &label, &own, scene);
        }
        self.draw_tabs(scene);
        self.draw_preedit(scene);
        self.draw_scrollbars(scene);
        self.draw_search(scene);
        self.draw_block(scene);
        self.draw_hints(scene);
        self.draw_link_tip(scene);
        self.draw_palette(scene);
        self.draw_sessions(scene);
        self.draw_history(scene);
        self.draw_notice(scene);
        self.draw_toast(scene);
        self.draw_hud(scene);
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

fn first_session(shape: &Shape) -> String {
    match shape {
        Shape::Pane(id) => id.clone(),
        Shape::Split { first, .. } => first_session(first),
    }
}
