//! ⇧⌘F: every command kept in the archive, found by what it was or by what it printed.

use vif_render::{Rect, Scene, Style};
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, NamedKey};

use super::keys::{quote, shell_run};
use super::{Shared, Win};
use crate::archive::{self, Entry};
use crate::colors::{mix, rgba};

const SHOWN: usize = 7;
const LOADED: usize = 20_000;

pub(super) struct History {
    query: String,
    selected: usize,
    lines: Vec<String>,
    found: Vec<Entry>,
}

impl History {
    pub(super) fn open() -> Self {
        let lines = archive::lines(LOADED);
        let found = archive::find(&lines, "", SHOWN);
        Self {
            query: String::new(),
            selected: 0,
            lines,
            found,
        }
    }

    fn refind(&mut self) {
        self.found = archive::find(&self.lines, &self.query, SHOWN);
        self.selected = 0;
    }
}

impl Win {
    pub(super) fn history_key(&mut self, event: &KeyEvent, shared: &Shared) -> bool {
        if event.state != ElementState::Pressed {
            return false;
        }
        let Some(history) = self.history.as_mut() else {
            return false;
        };
        let shown = history.found.len().max(1);
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.history = None,
            Key::Named(NamedKey::ArrowDown) => history.selected = (history.selected + 1) % shown,
            Key::Named(NamedKey::ArrowUp) => history.selected = (history.selected + shown - 1) % shown,
            Key::Named(NamedKey::Backspace) => {
                history.query.pop();
                history.refind();
            }
            Key::Named(NamedKey::Enter) => {
                let entry = history.found.get(history.selected).cloned();
                let query = history.query.clone();
                self.history = None;
                if let Some(entry) = entry {
                    self.open_entry(&entry, &query, shared);
                }
            }
            _ => {
                if let Some(text) = event.text.as_ref().filter(|t| !t.chars().any(char::is_control)) {
                    history.query.push_str(text);
                    history.refind();
                }
            }
        }
        self.window.request_redraw();
        false
    }

    pub(super) fn history_type(&mut self, text: &str) {
        if let Some(history) = self.history.as_mut() {
            history.query.push_str(text);
            history.refind();
            self.window.request_redraw();
        }
    }

    /// Shows a kept command and all it printed in a pager, in a tab of its own.
    fn open_entry(&mut self, entry: &Entry, query: &str, shared: &Shared) {
        let path = std::env::temp_dir().join(format!("vif-history-{}.txt", entry.at));
        let text = format!(
            "$ {}\n# {} · exit {} · {} · {}\n\n{}\n",
            entry.command,
            entry.host.as_deref().unwrap_or(&entry.cwd),
            entry.exit,
            crate::marks::clock(std::time::Duration::from_secs_f32(entry.took)),
            archive::ago(entry.at),
            entry.output
        );
        if let Err(err) = std::fs::write(&path, text) {
            return log::warn!("can't show the command: {err}");
        }
        let find = query
            .split_whitespace()
            .next()
            .map(|w| format!(" -p {}", quote(w)))
            .unwrap_or_default();
        // The output is UTF-8 whatever the shell's locale says.
        let script = format!(
            "LESSCHARSET=utf-8 exec less -R{find} {}",
            quote(&path.to_string_lossy())
        );
        let run = shell_run(&self.config, &script);
        self.open_tab(shared, Some(&run), None);
    }

    pub(super) fn draw_history(&mut self, scene: &mut Scene) {
        let Some(history) = &self.history else {
            return;
        };
        let (query, selected, found) = (history.query.clone(), history.selected, history.found.clone());
        let m = self.renderer.metrics();
        let scale = self.scale();
        let scheme = self.scheme;
        let (w, h) = self.size();
        let (cw, ch) = (m.cell_width, m.cell_height);
        let row = ch * 2.0 + 14.0 * scale;
        let pad = 12.0 * scale;
        let width = (90.0 * cw + 2.0 * pad).min(w - 32.0 * scale);
        let height = (ch + 10.0 * scale) + row * found.len().max(1) as f32 + 2.0 * pad;
        let (x, y) = (((w - width) / 2.0).round(), (h * 0.1).round());
        let cols = ((width - 2.0 * pad) / cw) as usize;
        let mut text = Scene::default();
        scene
            .front
            .push(Rect::fill(x, y, width, height, mix(scheme.fg(), scheme.bg(), 0.9)).rounded(self.radius()));
        let muted = mix(scheme.fg(), scheme.bg(), 0.45);
        let ty = (y + pad + 5.0 * scale).round();
        let typed = if query.is_empty() {
            self.label(
                "Find a command by what it was or what it printed",
                x + pad,
                ty,
                muted,
                Style::Regular,
                &mut text,
            );
            0.0
        } else {
            self.label(&query, x + pad, ty, scheme.fg(), Style::Regular, &mut text)
        };
        scene.front.push(Rect::fill(
            x + pad + typed,
            ty,
            (2.0 * scale).round(),
            ch,
            scheme.accent(),
        ));
        let top = y + pad + ch + 10.0 * scale;
        scene
            .front
            .push(Rect::fill(x, top, width, 1.0, mix(scheme.fg(), scheme.bg(), 0.8)));
        if found.is_empty() {
            let note = if self.config.terminal.archive_days == 0 {
                "keeping commands is off: archive_days = 0"
            } else {
                "nothing kept like that"
            };
            self.label(
                note,
                x + pad,
                (top + 7.0 * scale).round(),
                muted,
                Style::Regular,
                &mut text,
            );
        }
        let red = rgba(scheme.palette.ansi[1], 1.0);
        for (i, entry) in found.iter().enumerate() {
            let ry = top + row * i as f32;
            if i == selected {
                let mut tint = scheme.accent();
                tint[3] = 0.22;
                scene.front.push(
                    Rect::fill(x + pad / 2.0, ry + 3.0 * scale, width - pad, row - 6.0 * scale, tint)
                        .rounded(self.radius() * 0.6),
                );
            }
            let l1 = (ry + 7.0 * scale).round();
            let (mark, color) = if entry.exit == 0 {
                ("✓".to_owned(), muted)
            } else {
                (format!("✗ {}", entry.exit), red)
            };
            let mw = self.label(&mark, x + pad, l1, color, Style::Regular, &mut text);
            let place = entry.host.clone().unwrap_or_else(|| short(&entry.cwd));
            let right = format!("{} · {place}", archive::ago(entry.at));
            let right: String = right.chars().take(cols / 3).collect();
            let room = cols.saturating_sub(right.chars().count() + mark.chars().count() + 3);
            let command: String = entry.command.chars().take(room).collect();
            self.label(&command, x + pad + mw + cw, l1, scheme.fg(), Style::Bold, &mut text);
            let rx = x + width - pad - right.chars().count() as f32 * cw;
            self.label(&right, rx, l1, muted, Style::Regular, &mut text);
            let line: String = archive::snippet(entry, &query).chars().take(cols - 2).collect();
            self.label(&line, x + pad + 2.0 * cw, l1 + ch, muted, Style::Regular, &mut text);
        }
        scene.over.extend(text.mask);
    }
}

fn short(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if path.starts_with(&home) => format!("~{}", &path[home.len()..]),
        _ => path.to_owned(),
    }
}
