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
}

impl App {
    pub fn new(proxy: EventLoopProxy<UserEvent>, command: Option<Vec<String>>, attach: Option<String>) -> Self {
        let (config, diagnostics) = Config::load();
        let restore = match attach {
            Some(session) => vec![session],
            None if config.terminal.sessions && command.is_none() => crate::session::free(),
            None => Vec::new(),
        };
        let days = config.terminal.archive_days;
        if days > 0 {
            std::thread::spawn(move || crate::archive::prune(days));
        }
        let watcher = watch(proxy.clone());
        Self {
            shared: Shared {
                proxy,
                term: terminfo::setup(&config.terminal.term),
                command,
                restore: std::cell::RefCell::new(restore),
                outside: vif_config::apolline_accent(),
            },
            gpu: None,
            windows: HashMap::new(),
            config: Arc::new(config),
            diagnostics,
            _watcher: watcher,
        }
    }

    fn open(&mut self, event_loop: &ActiveEventLoop) {
        match Win::new(
            event_loop,
            &mut self.gpu,
            self.config.clone(),
            &self.diagnostics,
            &self.shared,
        ) {
            Ok(win) => {
                self.windows.insert(win.window.id(), win);
            }
            Err(err) => {
                eprintln!("vif: {err}");
                if self.windows.is_empty() {
                    event_loop.exit();
                }
            }
        }
    }

    fn close(&mut self, id: WindowId, event_loop: &ActiveEventLoop) {
        if let Some(mut win) = self.windows.remove(&id) {
            win.shutdown();
        }
        if self.windows.is_empty() {
            event_loop.exit();
        }
    }

    fn reload(&mut self) {
        let (config, diagnostics) = Config::load();
        let outside = vif_config::apolline_accent();
        if config == *self.config && diagnostics == self.diagnostics && outside == self.shared.outside {
            return;
        }
        self.shared.outside = outside;
        if config.terminal.term != self.config.terminal.term {
            self.shared.term = terminfo::setup(&config.terminal.term);
        }
        self.config = Arc::new(config);
        self.diagnostics = diagnostics;
        for win in self.windows.values_mut() {
            win.apply(self.config.clone(), &self.diagnostics, outside);
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.windows.is_empty() {
            self.open(event_loop);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
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
            UserEvent::NewWindow => self.open(event_loop),
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
        for win in self.windows.values_mut() {
            win.shutdown();
        }
        // EGL still talks to the Wayland connection while it tears down, so the GPU goes before the event loop.
        self.windows.clear();
        self.gpu = None;
    }
}

/// Watches vif.toml, the themes, and Apolline's accent file where Apolline colors apps.
fn watch(proxy: EventLoopProxy<UserEvent>) -> Option<notify::RecommendedWatcher> {
    let path = vif_config::path();
    let dir = path.parent()?.to_owned();
    std::fs::create_dir_all(&dir).ok()?;
    let apolline = vif_config::apolline_css();
    let names = [path.file_name()?.to_owned(), apolline.file_name()?.to_owned()];
    let themes = vif_config::themes::dir();
    std::fs::create_dir_all(&themes).ok()?;
    let theme_dir = themes.clone();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else {
            return;
        };
        let ours = event.paths.iter().any(|p| {
            let theme = p.parent() == Some(theme_dir.as_path()) && p.extension().is_some_and(|e| e == "toml");
            theme || p.file_name().is_some_and(|n| names.iter().any(|name| name == n))
        });
        let relevant = matches!(
            event.kind,
            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
        );
        if ours && relevant {
            let _ = proxy.send_event(UserEvent::ConfigChanged);
        }
    })
    .ok()?;
    watcher.watch(&dir, RecursiveMode::NonRecursive).ok()?;
    let _ = watcher.watch(&themes, RecursiveMode::NonRecursive);
    if let Some(gtk) = apolline.parent().filter(|d| d.is_dir()) {
        let _ = watcher.watch(gtk, RecursiveMode::NonRecursive);
    }
    Some(watcher)
}
