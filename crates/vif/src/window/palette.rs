//! The command palette: every action, theme and settings page, found by typing a few letters.

use vif_config::{ACTIONS, Editor, themes};
use vif_render::{Rect, Scene, Style};
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, NamedKey};

use super::keys::open_settings;
use super::{Shared, Win};
use crate::colors::mix;

const SHOWN: usize = 9;
const PAGES: [(&str, &str); 8] = [
    ("themes", "Themes"),
    ("font", "Font"),
    ("colors", "Colors"),
    ("window", "Window"),
    ("cursor", "Cursor"),
    ("tabs", "Tabs and splits"),
    ("keys", "Keys"),
    ("terminal", "Terminal"),
];

#[derive(Default)]
pub(super) struct Palette {
    query: String,
    selected: usize,
}

#[derive(Clone)]
enum Command {
    Action(&'static str),
    Theme(String),
    Settings(&'static str),
}

struct Entry {
    label: String,
    keys: String,
    command: Command,
}

impl Win {
    fn entries(&self) -> Vec<Entry> {
        let mut all: Vec<Entry> = ACTIONS
            .iter()
            .filter(|a| **a != "palette")
            .map(|action| Entry {
                label: spoken(action),
                keys: self
                    .bindings
                    .iter()
                    .find(|(_, a)| a == action)
                    .map(|(combo, _)| keycaps(combo))
                    .unwrap_or_default(),
                command: Command::Action(action),
            })
            .collect();
        let now = &self.config.theme;
        all.extend(themes::all().into_iter().map(|t| Entry {
            label: format!("Theme: {}", t.title),
            keys: if &t.name == now {
                "in use".to_owned()
            } else {
                String::new()
            },
            command: Command::Theme(t.name),
        }));
        all.extend(PAGES.iter().map(|(page, title)| Entry {
            label: format!("Settings: {title}"),
            keys: String::new(),
            command: Command::Settings(page),
        }));
        all
    }

    fn matches(&self) -> Vec<Entry> {
        let query = self.palette.as_ref().map(|p| p.query.as_str()).unwrap_or_default();
        let mut found: Vec<(i32, Entry)> = self
            .entries()
            .into_iter()
            .filter_map(|e| score(query, &e.label).map(|s| (s, e)))
            .collect();
        found.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
        found.into_iter().take(SHOWN).map(|(_, e)| e).collect()
    }

    /// Typing into the open palette; returns true when the window should close.
    pub(super) fn palette_key(&mut self, event: &KeyEvent, shared: &Shared) -> bool {
        if event.state != ElementState::Pressed {
            return false;
        }
        let shown = self.matches().len();
        let Some(palette) = self.palette.as_mut() else {
            return false;
        };
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.palette = None,
            Key::Named(NamedKey::ArrowDown) => palette.selected = (palette.selected + 1) % shown.max(1),
            Key::Named(NamedKey::ArrowUp) => palette.selected = (palette.selected + shown.max(1) - 1) % shown.max(1),
            Key::Named(NamedKey::Backspace) => {
                palette.query.pop();
                palette.selected = 0;
            }
            Key::Named(NamedKey::Enter) => {
                let selected = palette.selected;
                let command = self.matches().into_iter().nth(selected).map(|e| e.command);
                self.palette = None;
                self.window.request_redraw();
                return command.is_some_and(|c| self.run(c, shared));
            }
            _ => {
                if let Some(text) = event.text.as_ref().filter(|t| !t.chars().any(char::is_control)) {
                    palette.query.push_str(text);
                    palette.selected = 0;
                }
            }
        }
        self.window.request_redraw();
        false
    }

    pub(super) fn palette_type(&mut self, text: &str) {
        if let Some(palette) = self.palette.as_mut() {
            palette.query.push_str(text);
            palette.selected = 0;
            self.window.request_redraw();
        }
    }

    fn run(&mut self, command: Command, shared: &Shared) -> bool {
        match command {
            Command::Action(action) => return self.action(action, shared),
            Command::Theme(name) => {
                let result = Editor::open(&vif_config::path()).and_then(|mut e| {
                    e.set_top("theme", name.as_str());
                    e.save().map(|_| ()).map_err(|err| err.to_string())
                });
                if let Err(err) = result {
                    log::warn!("can't switch the theme: {err}");
                }
            }
            Command::Settings(page) => open_settings(Some(page)),
        }
        false
    }

    pub(super) fn draw_palette(&mut self, scene: &mut Scene) {
        let Some(palette) = &self.palette else {
            return;
        };
        let (query, selected) = (palette.query.clone(), palette.selected);
        let entries = self.matches();
        let m = self.renderer.metrics();
        let scale = self.scale();
        let scheme = self.scheme;
        let (w, h) = self.size();
        let (cw, ch) = (m.cell_width, m.cell_height);
        let row = ch + 10.0 * scale;
        let pad = 12.0 * scale;
        let width = (64.0 * cw + 2.0 * pad).min(w - 32.0 * scale);
        let height = row * (entries.len().max(1) as f32 + 1.0) + 2.0 * pad;
        let (x, y) = (((w - width) / 2.0).round(), (h * 0.12).round());
        let mut text = Scene::default();
        scene
            .front
            .push(Rect::fill(x, y, width, height, mix(scheme.fg(), scheme.bg(), 0.9)).rounded(self.radius()));
        let muted = mix(scheme.fg(), scheme.bg(), 0.45);
        let ty = (y + pad + (row - ch) / 2.0).round();
        let typed = if query.is_empty() {
            self.label(
                "Run a command, switch a theme…",
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
        let line = mix(scheme.fg(), scheme.bg(), 0.8);
        scene.front.push(Rect::fill(x, y + pad + row, width, 1.0, line));
        if entries.is_empty() {
            let ny = (y + pad + row + (row - ch) / 2.0).round();
            self.label("nothing like that", x + pad, ny, muted, Style::Regular, &mut text);
        }
        for (i, entry) in entries.iter().enumerate() {
            let ry = y + pad + row * (i as f32 + 1.0);
            if i == selected {
                let mut tint = scheme.accent();
                tint[3] = 0.22;
                scene.front.push(
                    Rect::fill(x + pad / 2.0, ry + 2.0 * scale, width - pad, row - 4.0 * scale, tint)
                        .rounded(self.radius() * 0.6),
                );
            }
            let ty = (ry + (row - ch) / 2.0).round();
            let max = ((width - 2.0 * pad) / cw) as usize - entry.keys.chars().count() - 2;
            let label: String = entry.label.chars().take(max).collect();
            self.label(&label, x + pad, ty, scheme.fg(), Style::Regular, &mut text);
            let kx = x + width - pad - entry.keys.chars().count() as f32 * cw;
            self.label(&entry.keys, kx, ty, muted, Style::Regular, &mut text);
        }
        scene.over.extend(text.mask);
    }
}

/// "split_right" as people say it: "Split right".
fn spoken(action: &str) -> String {
    let words = action.replace('_', " ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => words,
    }
}

/// A binding the way the keyboard shows it.
fn keycaps(combo: &str) -> String {
    let mac = cfg!(target_os = "macos");
    let parts: Vec<String> = combo
        .split('+')
        .map(|part| match (part, mac) {
            ("cmd", true) => "⌘".to_owned(),
            ("shift", true) => "⇧".to_owned(),
            ("alt", true) => "⌥".to_owned(),
            ("ctrl", true) => "⌃".to_owned(),
            ("up", _) => "↑".to_owned(),
            ("down", _) => "↓".to_owned(),
            ("left", _) => "←".to_owned(),
            ("right", _) => "→".to_owned(),
            (key, _) => {
                let mut c = key.chars();
                c.next()
                    .map(|f| f.to_uppercase().chain(c).collect())
                    .unwrap_or_default()
            }
        })
        .collect();
    parts.join(if mac { "" } else { "+" })
}

/// How well the letters typed fit a label, in order; words they start count most.
fn score(query: &str, label: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let label: Vec<char> = label.to_lowercase().chars().collect();
    let mut at = 0;
    let mut total = 0;
    let mut last: Option<usize> = None;
    for q in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        let found = (at..label.len()).find(|&i| label[i] == q)?;
        let starts_word = found == 0 || !label[found - 1].is_alphanumeric();
        total += if starts_word { 10 } else { 1 };
        if last == Some(found.wrapping_sub(1)) {
            total += 5;
        }
        last = Some(found);
        at = found + 1;
    }
    Some(total - label.len() as i32 / 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_find_their_command() {
        let labels = [
            "Split right",
            "Split down",
            "Scroll page up",
            "Theme: Gruvbox",
            "Settings: Font",
        ];
        let best = |q: &str| {
            labels
                .iter()
                .filter_map(|l| score(q, l).map(|s| (s, *l)))
                .max_by_key(|(s, _)| *s)
                .map(|(_, l)| l)
        };
        assert_eq!(best("sr"), Some("Split right"));
        assert_eq!(best("gruv"), Some("Theme: Gruvbox"));
        assert_eq!(best("font"), Some("Settings: Font"));
        assert_eq!(best("zzz"), None);
        assert_eq!(spoken("prompt_previous"), "Prompt previous");
    }
}
