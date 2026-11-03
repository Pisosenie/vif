//! Keys: bindings and the actions they run, typing into find and hints, and opening what a hint
//! points at.

use super::*;

impl Win {
    pub(super) fn key(&mut self, event: KeyEvent, shared: &Shared) -> bool {
        if event.state == ElementState::Pressed
            && let Some(action) = self.binding(&event)
        {
            return self.action(&action, shared);
        }
        if self.palette.is_some() {
            return self.palette_key(&event, shared);
        }
        if self.history.is_some() {
            return self.history_key(&event, shared);
        }
        if self.sessions.is_some() {
            self.sessions_key(&event, shared);
            return false;
        }
        if self.block.is_some() {
            self.block_key(&event, shared);
            return false;
        }
        if let Some(id) = self.tabs.get(self.active).map(|t| t.focus)
            && self.panes.get(&id).is_some_and(|p| p.hints.is_some())
        {
            if event.state == ElementState::Pressed {
                self.hints_key(id, &event, shared);
            }
            return false;
        }
        if let Some(id) = self.tabs.get(self.active).map(|t| t.focus)
            && self.panes.get(&id).is_some_and(|p| p.search.is_some())
        {
            if event.state == ElementState::Pressed {
                self.search_key(id, &event);
            }
            return false;
        }
        if !self.preedit.is_empty() {
            return false;
        }
        let Some(pane) = self.focused_pane() else {
            return false;
        };
        let mode = pane.mode();
        if let Some(bytes) = input::encode(&event, self.mods.state(), self.alt_meta(), mode) {
            if event.state == ElementState::Pressed {
                self.hud_pressed();
                if !mode.contains(TermMode::REPORT_EVENT_TYPES) {
                    self.window.set_cursor_visible(false);
                }
                self.wake_cursor();
                if self.notice.is_some() {
                    self.notice = None;
                }
                let pane = self.focused_pane().expect("checked above");
                pane.clear_selection_if_typing();
            }
            if let Some(pane) = self.focused_pane() {
                pane.input(bytes);
            }
        }
        false
    }

    pub(super) fn search_key(&mut self, id: PaneId, event: &KeyEvent) {
        let state = self.mods.state();
        let typing = !state.super_key() && !state.control_key();
        let Some(pane) = self.panes.get_mut(&id) else {
            return;
        };
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => pane.search_close(),
            Key::Named(NamedKey::Enter) => pane.search_step(!state.shift_key()),
            Key::Named(NamedKey::Backspace) => pane.search_edit(|q| {
                q.pop();
            }),
            _ => {
                if let Some(text) = event
                    .text
                    .as_ref()
                    .filter(|t| typing && !t.chars().any(char::is_control))
                {
                    let text = text.to_string();
                    pane.search_edit(|q| q.push_str(&text));
                }
            }
        }
        self.window.request_redraw();
    }

    pub(super) fn hints_key(&mut self, id: PaneId, event: &KeyEvent, shared: &Shared) {
        let state = self.mods.state();
        let letter = match event.key_without_modifiers() {
            Key::Character(s) if s.is_ascii() => s.chars().next(),
            _ => physical_char(event),
        }
        .map(|c| c.to_ascii_lowercase())
        .filter(char::is_ascii_lowercase);
        let Some(pane) = self.panes.get_mut(&id) else {
            return;
        };
        let Some(hints) = pane.hints.as_mut() else {
            return;
        };
        let picked = match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                pane.hints = None;
                None
            }
            Key::Named(NamedKey::Backspace) => {
                hints.typed.pop();
                None
            }
            _ => letter.and_then(|c| hints.press(c)),
        };
        if let Some(target) = picked {
            pane.hints = None;
            let cwd = pane.cwd();
            if state.shift_key() {
                self.clipboard.set(target.text);
            } else if state.alt_key() {
                self.paste(target.text);
            } else {
                self.open_target(target, cwd, shared);
            }
        }
        self.window.request_redraw();
    }

    /// Opens what a hint points at: links in the browser, files in the editor, folders and
    /// commits in a tab of their own.
    pub(super) fn open_target(&mut self, target: Target, cwd: Option<PathBuf>, shared: &Shared) {
        match target.kind {
            Kind::Url => {
                let _ = open::that_detached(&target.text);
            }
            Kind::Ip => self.clipboard.set(target.text),
            Kind::Path { path, .. } if path.is_dir() => self.open_tab(shared, None, Some(path)),
            Kind::Path { path, line } => {
                let at = line.map(|l| format!(" +{l}")).unwrap_or_default();
                let script = format!(
                    "exec ${{VISUAL:-${{EDITOR:-vi}}}}{at} {}",
                    quote(&path.to_string_lossy())
                );
                let run = shell_run(&self.config, &script);
                self.open_tab(shared, Some(&run), cwd);
            }
            Kind::Hash => {
                let script = format!("LESS=R exec git --paginate show {}", target.text);
                let run = shell_run(&self.config, &script);
                self.open_tab(shared, Some(&run), cwd);
            }
        }
    }

    pub(super) fn binding(&self, event: &KeyEvent) -> Option<String> {
        let state = self.mods.state();
        if !(state.super_key() || state.control_key() || state.alt_key() || state.shift_key()) {
            return None;
        }
        let key = match &event.logical_key {
            Key::Named(named) => named_key(*named)?.to_owned(),
            Key::Character(_) => {
                let typed = match event.key_without_modifiers() {
                    Key::Character(s) => s.to_lowercase(),
                    _ => return None,
                };
                if typed.is_ascii() {
                    typed
                } else {
                    physical_char(event)?.to_string()
                }
            }
            _ => return None,
        };
        let mut combo = Vec::new();
        if state.alt_key() {
            combo.push("alt");
        }
        if state.super_key() {
            combo.push("cmd");
        }
        if state.control_key() {
            combo.push("ctrl");
        }
        if state.shift_key() {
            combo.push("shift");
        }
        let combo = format!("{}+{key}", combo.join("+"));
        self.bindings
            .iter()
            .find(|(c, _)| *c == combo)
            .map(|(_, action)| action.clone())
    }

    /// Runs a bound action; returns true when the window should close.
    pub(super) fn action(&mut self, action: &str, shared: &Shared) -> bool {
        match action {
            "copy" => {
                if let Some(text) = self.focused_pane().and_then(Pane::selection_text) {
                    self.clipboard.set(text);
                }
            }
            "paste" => {
                let text = self.clipboard.get();
                let id = self.tabs.get(self.active).map(|t| t.focus);
                match id.and_then(|id| self.panes.get_mut(&id)) {
                    Some(pane) if pane.search.is_some() => {
                        let line = text.lines().next().unwrap_or_default().to_owned();
                        pane.search_edit(|q| q.push_str(&line));
                        self.window.request_redraw();
                    }
                    _ => self.paste(text),
                }
            }
            "prompt_previous" | "prompt_next" => self.block_step(action == "prompt_previous"),
            "copy_output" => {
                if let Some(id) = self.tabs.get(self.active).map(|t| t.focus)
                    && let Some(text) = self.panes.get_mut(&id).and_then(Pane::last_output)
                {
                    self.clipboard.set(text);
                }
                self.window.request_redraw();
            }
            "hints" => {
                if let Some(id) = self.tabs.get(self.active).map(|t| t.focus)
                    && let Some(pane) = self.panes.get_mut(&id)
                {
                    if pane.hints.is_some() {
                        pane.hints = None;
                    } else {
                        pane.hints_open(&self.config.hints.alphabet);
                    }
                }
                self.window.request_redraw();
            }
            "search" => {
                if let Some(id) = self.tabs.get(self.active).map(|t| t.focus)
                    && let Some(pane) = self.panes.get_mut(&id)
                {
                    pane.search_open();
                }
                self.window.request_redraw();
            }
            "new_window" => {
                let _ = shared.proxy.send_event(UserEvent::NewWindow);
            }
            "new_tab" => {
                let _ = self.new_tab(shared);
            }
            "close" => {
                if let Some(id) = self.tabs.get(self.active).map(|t| t.focus) {
                    return self.close_pane(id);
                }
            }
            "split_right" => self.split(shared, true),
            "zoom" => {
                if let Some(tab) = self.tabs.get_mut(self.active)
                    && !matches!(tab.root, Node::Pane(_))
                {
                    tab.zoomed = !tab.zoomed;
                    self.relayout();
                    self.update_title();
                }
            }
            "resize_left" | "resize_right" | "resize_up" | "resize_down" => {
                let direction = match action {
                    "resize_left" => Direction::Left,
                    "resize_right" => Direction::Right,
                    "resize_up" => Direction::Up,
                    _ => Direction::Down,
                };
                if let Some(tab) = self.tabs.get_mut(self.active)
                    && !tab.zoomed
                    && tab.root.resize(tab.focus, direction, 0.05)
                {
                    self.relayout();
                }
            }
            "split_down" => self.split(shared, false),
            "focus_next" | "focus_previous" => {
                if let Some(tab) = self.tabs.get(self.active) {
                    let mut ids = Vec::new();
                    tab.root.panes(&mut ids);
                    if let Some(i) = ids.iter().position(|&id| id == tab.focus) {
                        let next = if action == "focus_next" {
                            (i + 1) % ids.len()
                        } else {
                            (i + ids.len() - 1) % ids.len()
                        };
                        self.focus(ids[next]);
                    }
                }
            }
            "focus_left" | "focus_right" | "focus_up" | "focus_down" => {
                let direction = match action {
                    "focus_left" => Direction::Left,
                    "focus_right" => Direction::Right,
                    "focus_up" => Direction::Up,
                    _ => Direction::Down,
                };
                if let Some(tab) = self.tabs.get(self.active)
                    && let Some(id) = neighbor(&self.areas, tab.focus, direction)
                {
                    self.focus(id);
                }
            }
            "next_tab" | "previous_tab" => {
                let n = self.tabs.len();
                if n > 1 {
                    self.active = if action == "next_tab" {
                        (self.active + 1) % n
                    } else {
                        (self.active + n - 1) % n
                    };
                    self.relayout();
                    self.update_title();
                }
            }
            "font_bigger" | "font_smaller" | "font_reset" => {
                self.font_delta = match action {
                    "font_bigger" => self.font_delta + 1.0,
                    "font_smaller" => self.font_delta - 1.0,
                    _ => 0.0,
                };
                let scale = self.scale();
                self.renderer
                    .set_font(font_config(&self.config, scale, self.font_delta));
                self.relayout();
            }
            "scroll_page_up" | "scroll_page_down" | "scroll_top" | "scroll_bottom" => {
                let scroll = match action {
                    "scroll_page_up" => Scroll::PageUp,
                    "scroll_page_down" => Scroll::PageDown,
                    "scroll_top" => Scroll::Top,
                    _ => Scroll::Bottom,
                };
                if let Some(id) = self.tabs.get(self.active).map(|t| t.focus)
                    && let Some(pane) = self.panes.get_mut(&id)
                {
                    pane.scroll(scroll);
                }
                self.window.request_redraw();
            }
            "settings" => open_settings(None),
            "history" => {
                if self.history.take().is_none() {
                    self.open_history();
                }
                self.palette = None;
                self.window.request_redraw();
            }
            "hud" => self.toggle_hud(),
            "quick_terminal" => {
                let _ = shared.proxy.send_event(crate::app::UserEvent::Quick);
            }
            "sessions" => {
                if self.sessions.take().is_none() {
                    self.open_sessions();
                }
                self.window.request_redraw();
            }
            "palette" => {
                self.palette = match self.palette {
                    Some(_) => None,
                    None => Some(super::palette::Palette::open()),
                };
                self.window.request_redraw();
            }
            tab if tab.starts_with("tab_") => {
                if let Ok(n) = tab[4..].parse::<usize>()
                    && n >= 1
                    && n <= self.tabs.len()
                {
                    self.active = n - 1;
                    self.relayout();
                    self.update_title();
                }
            }
            _ => {}
        }
        false
    }

    pub(super) fn paste(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        let Some(pane) = self.focused_pane() else {
            return;
        };
        let bytes = if pane.mode().contains(TermMode::BRACKETED_PASTE) {
            let clean = text.replace('\x1b', "");
            format!("\x1b[200~{clean}\x1b[201~")
        } else {
            text.replace("\r\n", "\r").replace('\n', "\r")
        };
        pane.input(bytes.into_bytes());
    }
}

fn named_key(key: NamedKey) -> Option<&'static str> {
    Some(match key {
        NamedKey::ArrowLeft => "left",
        NamedKey::ArrowRight => "right",
        NamedKey::ArrowUp => "up",
        NamedKey::ArrowDown => "down",
        NamedKey::Home => "home",
        NamedKey::End => "end",
        NamedKey::PageUp => "pageup",
        NamedKey::PageDown => "pagedown",
        NamedKey::Tab => "tab",
        NamedKey::Enter => "enter",
        NamedKey::Escape => "escape",
        NamedKey::Backspace => "backspace",
        NamedKey::Delete => "delete",
        NamedKey::Insert => "insert",
        NamedKey::Space => "space",
        NamedKey::F1 => "f1",
        NamedKey::F2 => "f2",
        NamedKey::F3 => "f3",
        NamedKey::F4 => "f4",
        NamedKey::F5 => "f5",
        NamedKey::F6 => "f6",
        NamedKey::F7 => "f7",
        NamedKey::F8 => "f8",
        NamedKey::F9 => "f9",
        NamedKey::F10 => "f10",
        NamedKey::F11 => "f11",
        NamedKey::F12 => "f12",
        _ => return None,
    })
}

fn physical_char(event: &KeyEvent) -> Option<char> {
    use winit::keyboard::{KeyCode, PhysicalKey};
    let PhysicalKey::Code(code) = event.physical_key else {
        return None;
    };
    let name = format!("{code:?}");
    if let Some(letter) = name.strip_prefix("Key") {
        return letter.chars().next().map(|c| c.to_ascii_lowercase());
    }
    if let Some(digit) = name.strip_prefix("Digit") {
        return digit.chars().next();
    }
    Some(match code {
        KeyCode::BracketLeft => '[',
        KeyCode::BracketRight => ']',
        KeyCode::Comma => ',',
        KeyCode::Period => '.',
        KeyCode::Minus => '-',
        KeyCode::Equal => '=',
        KeyCode::Semicolon => ';',
        KeyCode::Quote => '\'',
        KeyCode::Slash => '/',
        KeyCode::Backslash => '\\',
        KeyCode::Backquote => '`',
        _ => return None,
    })
}

/// The user's shell running `script` with their login environment, so PATH and EDITOR are theirs.
pub(super) fn shell_run(config: &Config, script: &str) -> Vec<String> {
    let shell = config
        .terminal
        .shell
        .clone()
        .or_else(|| std::env::var("SHELL").ok())
        .unwrap_or_else(|| "/bin/sh".to_owned());
    let name = shell.rsplit('/').next().unwrap_or_default();
    let posix = matches!(name, "sh" | "bash" | "zsh" | "ksh" | "dash");
    let shell = if posix { shell } else { "/bin/sh".to_owned() };
    vec![
        shell,
        "-l".to_owned(),
        "-i".to_owned(),
        "-c".to_owned(),
        script.to_owned(),
    ]
}

pub(super) fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The settings window, on a page if one is named.
pub fn open_settings(page: Option<&str>) {
    let exe = std::env::current_exe().ok();
    let sibling = exe
        .as_ref()
        .and_then(|e| e.parent())
        .map(|dir| dir.join("vif-settings"))
        .filter(|p| p.exists());
    let program = sibling.unwrap_or_else(|| PathBuf::from("vif-settings"));
    let mut command = std::process::Command::new(&program);
    if let Some(page) = page {
        command.args(["--page", page]);
    }
    if let Err(err) = command.spawn() {
        log::warn!("can't open settings ({}): {err}", program.display());
    }
}
