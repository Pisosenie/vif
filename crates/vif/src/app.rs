use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use alacritty_terminal::event::Event;
use notify::{EventKind, RecursiveMode, Watcher};
use vif_config::{Config, Diagnostic};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::window::{Window, WindowId};

use crate::layout::PaneId;
use crate::layouts::Saved;
use crate::terminfo;
use crate::window::{Shared, Win};

static NEXT_PANE: AtomicU64 = AtomicU64::new(1);
static STARTED: OnceLock<Instant> = OnceLock::new();

pub fn next_pane() -> PaneId {
    NEXT_PANE.fetch_add(1, Ordering::Relaxed)
}

pub fn started() -> Instant {
    *STARTED.get_or_init(Instant::now)
}

pub fn mark(stage: &str) {
    if std::env::var_os("VIF_TIMING").is_some() {
        eprintln!("{stage:>14} {:?}", started().elapsed());
    }
}

#[derive(Debug)]
pub enum UserEvent {
    Term {
        window: WindowId,
        pane: PaneId,
        event: Event,
    },
    ConfigChanged,
    /// Show or hide the quick terminal.
    Quick,
    /// A window of its own for a session.
    OpenSession(String),
    CloseWindow(WindowId),
    /// A shader or the background picture changed on disk.
    Looks,
    Wallpaper {
        window: WindowId,
        picture: Result<crate::wallpaper::Picture, String>,
    },
    NewWindow,
}

pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Gpu {
    /// Metal or Vulkan first, GL where neither works; returns the first window's surface too.
    pub fn new(window: Arc<Window>) -> Result<(Self, wgpu::Surface<'static>), String> {
        let tries = match wgpu::Backends::from_env() {
            Some(chosen) => vec![chosen],
            None => vec![wgpu::Backends::PRIMARY, wgpu::Backends::GL],
        };
        let mut last = String::from("no GPU backend");
        for backends in tries {
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                backends,
                ..Default::default()
            });
            let surface = match instance.create_surface(window.clone()) {
                Ok(surface) => surface,
                Err(err) => {
                    last = err.to_string();
                    continue;
                }
            };
            let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            }));
            let adapter = match adapter {
                Ok(adapter) => adapter,
                Err(err) => {
                    last = err.to_string();
                    continue;
                }
            };
            let device = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("vif"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
            }));
            match device {
                Ok((device, queue)) => {
                    let gpu = Self {
                        instance,
                        adapter,
                        device,
                        queue,
                    };
                    return Ok((gpu, surface));
                }
                Err(err) => last = err.to_string(),
            }
        }
        Err(format!("no GPU to draw with: {last}"))
    }
}

pub struct App {
    shared: Shared,
    gpu: Option<Gpu>,
    windows: HashMap<WindowId, Win>,
    config: Arc<Config>,
    diagnostics: Vec<Diagnostic>,
    _watcher: Option<notify::RecommendedWatcher>,
    /// Windows of sessions the last vif left, for the first windows to take over.
    restore: Vec<Saved>,
    quick: Option<WindowId>,
    /// The quick terminal's sessions while it has no window.
    quick_saved: Option<Saved>,
    /// Started by `vif quick`: the quick terminal is the first window.
    quick_start: bool,
    /// Another app was in front when the quick terminal came, and gets the focus back after.
    quick_back: bool,
    /// The quick terminal has had the focus since it last showed, so losing it may hide it.
    quick_focused: bool,
    /// This vif answers `vif quick`.
    listening: bool,
    #[cfg(target_os = "macos")]
    hotkey: Option<crate::quick::Hotkey>,
}

impl App {
    pub fn new(
        proxy: EventLoopProxy<UserEvent>,
        command: Option<Vec<String>>,
        attach: Option<String>,
        quick: bool,
    ) -> Self {
        let (config, diagnostics) = Config::load();
        let mut restore = match attach {
            Some(session) => vec![Saved {
                tabs: vec![crate::layouts::SavedTab {
                    root: crate::layouts::Shape::Pane(session.clone()),
                    focus: session,
                }],
                ..Saved::default()
            }],
            None if config.terminal.sessions && command.is_none() => crate::layouts::take(crate::session::free()),
            None => Vec::new(),
        };
        let quick_saved = restore.iter().position(|s| s.quick).map(|i| restore.remove(i));
        let listening = crate::quick::listen(proxy.clone());
        let days = config.terminal.archive_days;
        if days > 0 {
            std::thread::spawn(move || crate::archive::prune(days));
        }
        let watcher = watch(proxy.clone(), &config);
        Self {
            shared: Shared {
                proxy,
                term: terminfo::setup(&config.terminal.term),
                command,
                outside: vif_config::apolline_accent(),
            },
            gpu: None,
            windows: HashMap::new(),
            config: Arc::new(config),
            diagnostics,
            _watcher: watcher,
            restore,
            quick: None,
            quick_saved,
            quick_start: quick,
            quick_back: false,
            quick_focused: false,
            listening,
            #[cfg(target_os = "macos")]
            hotkey: None,
        }
    }

    fn open(&mut self, event_loop: &ActiveEventLoop, restore: Option<Saved>, quick: bool) -> Option<WindowId> {
        match Win::new(
            event_loop,
            &mut self.gpu,
            self.config.clone(),
            &self.diagnostics,
            &self.shared,
            restore,
            quick,
        ) {
            Ok(win) => {
                let id = win.window.id();
                self.windows.insert(id, win);
                Some(id)
            }
            Err(err) => {
                eprintln!("vif: {err}");
                if self.windows.is_empty() {
                    event_loop.exit();
                }
                None
            }
        }
    }

    /// Shows the quick terminal, focuses it, or hides it when it already has the focus.
    fn toggle_quick(&mut self, event_loop: &ActiveEventLoop) {
        let shown = self.quick.and_then(|id| self.windows.get(&id)).map(|w| {
            let visible = w.window.is_visible().unwrap_or(true);
            (visible, visible && w.has_focus())
        });
        #[cfg(target_os = "macos")]
        if shown.is_none_or(|(visible, _)| !visible) {
            self.quick_back = !crate::quick::active();
        }
        match shown {
            Some((_, true)) => self.hide_quick(event_loop),
            Some(_) => {
                if let Some(win) = self.quick.and_then(|id| self.windows.get(&id)) {
                    win.window.set_visible(true);
                    win.window.focus_window();
                }
            }
            None => {
                let restore = self.quick_saved.take();
                self.quick = self.open(event_loop, restore, true);
            }
        }
    }

    fn hide_quick(&mut self, event_loop: &ActiveEventLoop) {
        let Some(id) = self.quick else {
            return;
        };
        self.quick_focused = false;
        #[cfg(target_os = "macos")]
        {
            let _ = event_loop;
            if let Some(win) = self.windows.get(&id) {
                win.window.set_visible(false);
            }
            if self.quick_back {
                crate::quick::step_back();
            }
        }
        // Wayland can't hide a window: it goes, and its shells wait in their sessions for the next.
        #[cfg(not(target_os = "macos"))]
        {
            let _ = event_loop;
            if let Some(mut win) = self.windows.remove(&id) {
                self.quick_saved = Some(win.saved.clone()).filter(|s| !s.tabs.is_empty());
                win.shutdown();
            }
            self.quick = None;
        }
    }

    fn close(&mut self, id: WindowId, event_loop: &ActiveEventLoop) {
        if let Some(mut win) = self.windows.remove(&id) {
            win.shutdown();
        }
        if self.quick == Some(id) {
            self.quick = None;
        }
        // A hidden quick terminal keeps vif running, so it comes back at once.
        if self.windows.is_empty() && self.quick_saved.is_none() {
            event_loop.exit();
        }
    }

    fn reload(&mut self) {
        let started = Instant::now();
        let (config, diagnostics) = Config::load();
        let outside = vif_config::apolline_accent();
        if config == *self.config && diagnostics == self.diagnostics && outside == self.shared.outside {
            return;
        }
        self.shared.outside = outside;
        if config.terminal.term != self.config.terminal.term {
            self.shared.term = terminfo::setup(&config.terminal.term);
        }
        #[cfg(target_os = "macos")]
        if config.quick.key != self.config.quick.key
            && let Some(hotkey) = &mut self.hotkey
            && let Err(err) = hotkey.set(&config.quick.key)
        {
            log::warn!("{err}");
        }
        if config.colors.from != self.config.colors.from || config.background.image != self.config.background.image {
            self._watcher = watch(self.shared.proxy.clone(), &config);
        }
        self.config = Arc::new(config);
        self.diagnostics = diagnostics;
        for win in self.windows.values_mut() {
            win.apply(self.config.clone(), &self.diagnostics, outside, &self.shared);
            win.reloaded(started.elapsed());
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        #[cfg(target_os = "macos")]
        if self.listening && self.hotkey.is_none() {
            match crate::quick::Hotkey::new(self.shared.proxy.clone()) {
                Ok(mut hotkey) => {
                    if let Err(err) = hotkey.set(&self.config.quick.key) {
                        log::warn!("{err}");
                    }
                    self.hotkey = Some(hotkey);
                }
                Err(err) => log::warn!("no quick terminal key: {err}"),
            }
        }
        if !self.windows.is_empty() {
            return;
        }
        if self.quick_start {
            return self.toggle_quick(event_loop);
        }
        let mut windows = std::mem::take(&mut self.restore).into_iter();
        self.open(event_loop, windows.next(), false);
        // The rest of the windows the last vif left.
        for saved in windows {
            self.open(event_loop, Some(saved), false);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.quick == Some(id) {
            match event {
                WindowEvent::Focused(true) => self.quick_focused = true,
                WindowEvent::Focused(false) if self.quick_focused && self.config.quick.hide_on_blur => {
                    self.hide_quick(event_loop);
                    if !self.windows.contains_key(&id) {
                        return;
                    }
                }
                _ => {}
            }
        }
        let close = match self.windows.get_mut(&id) {
            Some(win) => win.event(event, &self.shared),
            None => false,
        };
        if close {
            self.close(id, event_loop);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Term { window, pane, event } => {
                let close = match self.windows.get_mut(&window) {
                    Some(win) => win.term_event(pane, event),
                    None => false,
                };
                if close {
                    self.close(window, event_loop);
                }
            }
            UserEvent::ConfigChanged => self.reload(),
            UserEvent::Looks => {
                for win in self.windows.values_mut() {
                    win.refresh(&self.shared);
                }
            }
            UserEvent::Wallpaper { window, picture } => {
                if let Some(win) = self.windows.get_mut(&window) {
                    win.set_wallpaper(picture);
                }
            }
            UserEvent::NewWindow => {
                self.open(event_loop, None, false);
            }
            UserEvent::Quick => self.toggle_quick(event_loop),
            UserEvent::OpenSession(id) => {
                let saved = Saved {
                    tabs: vec![crate::layouts::SavedTab {
                        root: crate::layouts::Shape::Pane(id.clone()),
                        focus: id,
                    }],
                    ..Saved::default()
                };
                self.open(event_loop, Some(saved), false);
            }
            UserEvent::CloseWindow(id) => self.close(id, event_loop),
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        for win in self.windows.values_mut() {
            win.tick(now);
        }
        match self.windows.values().filter_map(Win::deadline).min() {
            Some(at) => event_loop.set_control_flow(ControlFlow::WaitUntil(at)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if self.listening {
            crate::quick::forget();
        }
        for win in self.windows.values_mut() {
            win.shutdown();
        }
        // EGL still talks to the Wayland connection while it tears down, so the GPU goes before the event loop.
        self.windows.clear();
        self.gpu = None;
    }
}

/// Watches vif.toml, the themes, Apolline's accent file where Apolline colors apps and the
/// color scheme `[colors] from` follows; and, for the looks, the shaders and the background picture.
fn watch(proxy: EventLoopProxy<UserEvent>, config: &Config) -> Option<notify::RecommendedWatcher> {
    let path = vif_config::path();
    let dir = path.parent()?.to_owned();
    std::fs::create_dir_all(&dir).ok()?;
    let apolline = vif_config::apolline_css();
    let scheme = config.colors.from.as_deref().map(vif_config::scheme_path);
    let picture = config.background.image.as_deref().map(|i| vif_config::file(i, ""));
    let shaders = vif_config::dir().join("shaders");
    let mut names = vec![path.file_name()?.to_owned(), apolline.file_name()?.to_owned()];
    names.extend(scheme.as_ref().and_then(|s| s.file_name()).map(ToOwned::to_owned));
    let themes = vif_config::themes::dir();
    std::fs::create_dir_all(&themes).ok()?;
    let (theme_dir, shader_dir, picture_path) = (themes.clone(), shaders.clone(), picture.clone());
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else {
            return;
        };
        if !matches!(
            event.kind,
            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
        ) {
            return;
        }
        let config = event.paths.iter().any(|p| {
            let theme = p.parent() == Some(theme_dir.as_path()) && p.extension().is_some_and(|e| e == "toml");
            theme || p.file_name().is_some_and(|n| names.iter().any(|name| name == n))
        });
        let looks = event
            .paths
            .iter()
            .any(|p| p.parent() == Some(shader_dir.as_path()) || Some(p) == picture_path.as_ref());
        if config {
            let _ = proxy.send_event(UserEvent::ConfigChanged);
        } else if looks {
            let _ = proxy.send_event(UserEvent::Looks);
        }
    })
    .ok()?;
    watcher.watch(&dir, RecursiveMode::NonRecursive).ok()?;
    let _ = watcher.watch(&themes, RecursiveMode::NonRecursive);
    let extras = [
        apolline.parent(),
        scheme.as_deref().and_then(|s| s.parent()),
        picture.as_deref().and_then(|p| p.parent()),
        Some(shaders.as_path()),
    ];
    for extra in extras.into_iter().flatten() {
        if extra.is_dir() && extra != dir {
            let _ = watcher.watch(extra, RecursiveMode::NonRecursive);
        }
    }
    Some(watcher)
}
