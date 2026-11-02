//! What lies under and over the text: the background picture and the shaders.

use vif_config::{Config, CursorShape};
use vif_render::{Rect, Scene};

use super::{Shared, Win};
use crate::app::UserEvent;
use crate::wallpaper::{self, Picture};

impl Win {
    /// Reads the shaders and starts reading the background picture, which arrives as an event.
    pub(super) fn refresh_looks(&mut self, shared: &Shared) {
        let mut problems = Vec::new();
        let sources = shader_sources(&self.config, &mut problems);
        if let Err(err) = self.renderer.set_effects(&sources) {
            problems.push(err);
        }
        let background = self.config.background.clone();
        if background.image.is_none() {
            self.wallpaper = None;
        } else {
            let (proxy, window) = (shared.proxy.clone(), self.window.id());
            std::thread::spawn(move || {
                if let Some(picture) = wallpaper::load(&background) {
                    let _ = proxy.send_event(UserEvent::Wallpaper { window, picture });
                }
            });
        }
        self.tell(problems);
        self.window.request_redraw();
    }

    pub fn refresh(&mut self, shared: &Shared) {
        self.refresh_looks(shared);
    }

    pub fn set_wallpaper(&mut self, picture: Result<Picture, String>) {
        match picture {
            Ok(p) => {
                self.renderer.image(p.key, &p.rgba, p.width, p.height);
                self.wallpaper = Some((p.key, (p.width, p.height)));
            }
            Err(err) => {
                self.wallpaper = None;
                self.tell(vec![err]);
            }
        }
        self.window.request_redraw();
    }

    pub(super) fn draw_wallpaper(&self, scene: &mut Scene) {
        let Some((key, size)) = self.wallpaper else {
            return;
        };
        let (w, h) = self.size();
        let opacity = self.scheme.opacity;
        let sprites = wallpaper::place(self.config.background.fit, size, w, h, opacity);
        scene.under.extend(sprites.into_iter().map(|s| (key, s)));
        let dim = self.config.background.dim;
        if dim > 0.0 {
            let mut bg = self.scheme.bg();
            bg[3] = dim * opacity;
            scene.back.insert(0, Rect::fill(0.0, 0.0, w, h, bg));
        }
    }

    /// Tells the shaders where the cursor and the mouse are; true when they want another frame.
    pub(super) fn feed_effects(&mut self) {
        let cursor = self.cursor_rect();
        let color = self.scheme.cursor();
        let (x, y) = self.mouse;
        let (focused, bg, fg) = (self.focused, self.scheme.bg(), self.scheme.fg());
        if let Some(effects) = self.renderer.effects() {
            effects.cursor(cursor, color);
            effects.mouse(x, y);
            effects.window(focused, bg, fg);
        }
    }

    pub(super) fn effects_playing(&mut self) -> bool {
        self.renderer.effects().is_some_and(|e| e.playing())
    }

    /// The focused cursor as drawn: left, top, width and height in pixels.
    fn cursor_rect(&self) -> Option<[f32; 4]> {
        let pane = self.focused_pane()?;
        let (col, line) = pane.cursor_cell()?;
        let m = self.renderer.metrics();
        let (x, y) = (
            pane.area.x + col as f32 * m.cell_width,
            pane.area.y + line as f32 * m.cell_height,
        );
        let thin = (m.cell_width * 0.12).max(self.scale() * 2.0);
        Some(match self.config.cursor.shape {
            CursorShape::Block => [x, y, m.cell_width, m.cell_height],
            CursorShape::Beam => [x, y, thin, m.cell_height],
            CursorShape::Underline => [x, y + m.cell_height - thin, m.cell_width, thin],
        })
    }
}

/// Each shader the config names, as (name, source): a file in the shaders folder, or one of vif's
/// own by name. `[cursor] trail` adds vif's cursor trail last.
pub fn shader_sources(config: &Config, problems: &mut Vec<String>) -> Vec<(String, String)> {
    let trail = config.cursor.trail.then_some("cursor-trail");
    let names = config.window.shaders.iter().map(String::as_str).chain(trail);
    let mut out = Vec::new();
    for name in names {
        let path = vif_config::file(name, "shaders");
        let path = if path.extension().is_none() {
            path.with_extension("glsl")
        } else {
            path
        };
        let bare = name.trim_end_matches(".glsl");
        let own = vif_render::BUILT_IN_SHADERS.iter().find(|(n, _)| *n == bare);
        match (std::fs::read_to_string(&path), own) {
            (Ok(text), _) => out.push((name.to_owned(), text)),
            (Err(_), Some((n, text))) => out.push(((*n).to_owned(), (*text).to_owned())),
            (Err(err), None) => problems.push(format!("shader {}: {err}", crate::tilde(&path))),
        }
    }
    out
}
