//! What each message does to the settings: an edit to vif.toml, a page turned, a key recorded.

use super::*;

impl Settings {
    pub(super) fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Page(page) => {
                self.page = page;
                self.query.clear();
            }
            Message::Query(query) => self.query = query,
            Message::More(change) => self.change(change),
            Message::Greet(change) => return self.greet(change),
            Message::Ai(change) => return self.ai(change),
            Message::Family(family) => self.edit(|e| e.set("font", "family", family)),
            Message::Size(v) => self.edit(|e| e.set_float("font", "size", v)),
            Message::LineHeight(v) => self.edit(|e| e.set_float("font", "line_height", v)),
            Message::Ligatures(v) => self.edit(|e| e.set("font", "ligatures", v)),
            Message::UseTheme(name) => self.edit(|e| e.set_top("theme", name)),
            Message::NewTheme => {
                let from = self.config.theme.clone();
                let base = format!("{}-mine", from.trim_end_matches("-mine"));
                let name = (1..)
                    .map(|n| if n == 1 { base.clone() } else { format!("{base}-{n}") })
                    .find(|n| !themes::dir().join(format!("{n}.toml")).exists())
                    .unwrap_or(base);
                match themes::create(&name, &from) {
                    Ok(path) => {
                        self.edit(|e| e.set_top("theme", name.as_str()));
                        self.gallery = gallery::cards();
                        let _ = open::that_detached(path);
                    }
                    Err(err) => self.error = Some(err),
                }
            }
            Message::OpenThemes => {
                let _ = std::fs::create_dir_all(themes::dir());
                let _ = open::that_detached(themes::dir());
            }
            Message::TabPlace(Choice(v)) => self.edit(|e| e.set("tabs", "position", v)),
            Message::TabLook(Choice(v)) => self.edit(|e| e.set("tabs", "style", v)),
            Message::TabShowing(Choice(v)) => self.edit(|e| e.set("tabs", "show", v)),
            Message::TabTitle(v) => {
                self.drafts.insert("tab_title", v.clone());
                self.edit(|e| e.set("tabs", "title", v));
            }
            Message::Divide(Choice(v)) => self.edit(|e| e.set("panes", "divider", v)),
            Message::Dim(v) => self.edit(|e| e.set_float("panes", "dim", v)),
            Message::Radius(v) => self.edit(|e| e.set_float("window", "radius", v)),
            Message::BadgesShow(Choice(v)) => self.edit(|e| e.set("badges", "show", v)),
            Message::BadgeMark(key, v) => {
                if v.chars().count() <= 4 {
                    self.edit(|e| e.set("badges", key, v));
                }
            }
            Message::Alphabet(v) => {
                self.drafts.insert("alphabet", v.clone());
                let mut letters: Vec<char> = v.chars().collect();
                letters.sort_unstable();
                letters.dedup();
                let valid = letters.len() == v.chars().count()
                    && letters.len() >= 2
                    && v.chars().all(|c| c.is_ascii_lowercase());
                if valid {
                    self.edit(|e| e.set("hints", "alphabet", v));
                }
            }
            Message::Apolline(v) => self.edit(|e| e.set("colors", "apolline", v)),
            Message::ColorDraft(key, draft) => {
                if let Some(color) = Rgb::parse(draft.trim()) {
                    self.edit(|e| e.set_color("colors", key, Some(color)));
                }
                self.drafts.insert(key, draft);
            }
            Message::ColorReset(key) => self.edit(|e| e.remove("colors", key)),
            Message::PadX(v) => {
                let y = self.config.window.padding[1];
                self.edit(|e| e.set_padding(v.round() as u16, y));
            }
            Message::PadY(v) => {
                let x = self.config.window.padding[0];
                self.edit(|e| e.set_padding(x, v.round() as u16));
            }
            Message::Opacity(v) => self.edit(|e| e.set_float("window", "opacity", v)),
            Message::Decorations(Choice(v)) => self.edit(|e| e.set("window", "decorations", v)),
            Message::Columns(v) => self.edit(|e| e.set("window", "columns", v.round() as i64)),
            Message::Lines(v) => self.edit(|e| e.set("window", "lines", v.round() as i64)),
            Message::Shape(shape) => {
                let name = match shape {
                    CursorShape::Block => "block",
                    CursorShape::Beam => "beam",
                    CursorShape::Underline => "underline",
                };
                self.edit(|e| e.set("cursor", "shape", name));
            }
            Message::Blink(v) => self.edit(|e| e.set("cursor", "blink", v)),
            Message::ComboDraft(v) => self.combo = v,
            Message::Record => self.recording = !self.recording,
            Message::Slot(i) => {
                self.slot = i;
                self.slot_draft = None;
            }
            Message::SlotDraft(draft) => {
                if let Some(color) = Rgb::parse(draft.trim()) {
                    let mut colors = self.config.palette().ansi;
                    colors[self.slot] = color;
                    self.edit(|e| e.set_palette(Some(&colors)));
                }
                self.slot_draft = Some(draft);
            }
            Message::SlotReset => {
                let theme = self.config.colors.base.ansi;
                let mut colors = self.config.palette().ansi;
                colors[self.slot] = theme[self.slot];
                let custom = colors != theme;
                self.edit(|e| e.set_palette(custom.then_some(&colors)));
                self.slot_draft = None;
            }
            Message::ActionDraft(v) => self.action = Some(v),
            Message::AddKey => {
                let combo = vif_config::normalize_combo(&self.combo);
                if let Some(Choice(action)) = self.action.clone()
                    && combo.split('+').next_back().is_some_and(|k| !k.is_empty())
                {
                    self.edit(|e| e.set("keys", &combo, action));
                    self.combo.clear();
                }
            }
            Message::RemoveKey(combo) => {
                let default = vif_config::default_keys()
                    .iter()
                    .any(|(c, _)| vif_config::normalize_combo(c) == combo);
                let own = self
                    .config
                    .keys
                    .iter()
                    .find(|(c, _)| vif_config::normalize_combo(c) == combo)
                    .map(|(c, _)| c.clone());
                self.edit(|e| {
                    if let Some(own) = &own {
                        e.remove("keys", own);
                    }
                    if default {
                        e.set("keys", &combo, "none");
                    }
                });
            }
            Message::Shell(v) => {
                if v.trim().is_empty() {
                    self.edit(|e| e.remove("terminal", "shell"));
                } else {
                    self.edit(|e| e.set("terminal", "shell", v));
                }
            }
            Message::Scrollback(v) => self.edit(|e| e.set("terminal", "scrollback", v.round() as i64)),
            Message::KeysLike(Choice("vif")) => self.edit(|e| e.remove("keys", "like")),
            Message::KeysLike(Choice(v)) => self.edit(|e| e.set("keys", "like", v)),
            Message::Term(Choice(v)) => self.edit(|e| e.set("terminal", "term", v)),
            Message::Osc52(Choice(v)) => self.edit(|e| e.set("terminal", "osc52", v)),
            Message::Kitty(v) => self.edit(|e| e.set("terminal", "kitty_keyboard", v)),
            Message::Integration(v) => self.edit(|e| e.set("terminal", "shell_integration", v)),
            Message::NotifyAfter(v) => self.edit(|e| e.set("terminal", "notify_after", v.round() as i64)),
            Message::Sessions(v) => self.edit(|e| e.set("terminal", "sessions", v)),
            Message::ArchiveDays(v) => self.edit(|e| e.set("terminal", "archive_days", v.round() as i64)),
            Message::OptionAsAlt(Choice(v)) => self.edit(|e| e.set("terminal", "option_as_alt", v)),
            Message::Fix(i) => {
                if let (Some(fix), Ok(editor)) = (self.diagnostics.get(i).and_then(|d| d.fix.clone()), &self.editor) {
                    let before = editor.text();
                    if let Some(text) = fix.apply(&before) {
                        self.undo.push(before);
                        self.replace(text);
                    }
                }
            }
            Message::FixAll => {
                if let Ok(editor) = &self.editor {
                    let before = editor.text();
                    if let Some(text) = vif_config::fix_all(&before, &self.diagnostics) {
                        self.undo.push(before);
                        self.replace(text);
                    }
                }
            }
            Message::Undo => {
                if let (Some(text), Ok(editor)) = (self.undo.pop(), &self.editor) {
                    self.redo.push(editor.text());
                    self.replace(text);
                }
            }
            Message::Redo => {
                if let (Some(text), Ok(editor)) = (self.redo.pop(), &self.editor) {
                    self.undo.push(editor.text());
                    self.replace(text);
                }
            }
            Message::Open => {
                if !self.path.exists() {
                    let _ = std::fs::create_dir_all(self.path.parent().unwrap_or(&self.path));
                    let _ = std::fs::write(&self.path, "");
                }
                let _ = open::that_detached(&self.path);
            }
            Message::Tick => {
                self.outside = vif_config::apolline_accent();
                let stamp = gallery::stamp();
                if stamp != self.gallery_stamp {
                    self.gallery_stamp = stamp;
                    self.gallery = gallery::cards();
                    self.reparse();
                }
                let now = modified(&self.path);
                if now != self.modified {
                    self.modified = now;
                    self.editor = Editor::open(&self.path);
                    self.changed = None;
                    self.reparse();
                }
            }
            Message::Families(list) => self.families = list,
            Message::Key(keyboard::Event::KeyPressed {
                key,
                physical_key,
                modifiers,
                ..
            }) if self.recording => {
                if key == keyboard::Key::Named(keyboard::key::Named::Escape) {
                    self.recording = false;
                } else if let Some(combo) = recorded(&key, physical_key, modifiers) {
                    self.combo = combo;
                    self.recording = false;
                }
            }
            Message::Key(keyboard::Event::KeyPressed { key, modifiers, .. }) => {
                if key == keyboard::Key::Named(keyboard::key::Named::Escape) {
                    self.query.clear();
                }
                let undo = matches!(key.as_ref(), keyboard::Key::Character("z" | "я")) && modifiers.command();
                if undo && modifiers.shift() {
                    return self.update(Message::Redo);
                }
                if undo {
                    return self.update(Message::Undo);
                }
            }
            Message::Key(_) => {}
        }
        Task::none()
    }
}
