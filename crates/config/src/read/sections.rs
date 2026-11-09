//! Each section of vif.toml read into the config, its values checked and every problem reported.

use super::*;

impl Reader<'_> {
    pub(super) fn font(&mut self, table: &DeTable, config: &mut Config) {
        let font = &mut config.font;
        for (key, value) in table.iter() {
            match key.get_ref().as_ref() {
                "family" => {
                    if let Some(v) = self.string(key, value) {
                        font.family = v.trim().to_owned();
                    }
                }
                "size" => {
                    if let Some(v) = self.float(key, value, 4.0..=96.0) {
                        font.size = v;
                    }
                }
                "line_height" => {
                    if let Some(v) = self.float(key, value, 0.8..=2.5) {
                        font.line_height = v;
                    }
                }
                "ligatures" => {
                    if let Some(v) = self.boolean(key, value) {
                        font.ligatures = v;
                    }
                }
                "weight" | "bold_weight" => {
                    if let Some(v) = self.integer(key, value, 100..=900) {
                        let slot = if key.get_ref() == "weight" {
                            &mut font.weight
                        } else {
                            &mut font.bold_weight
                        };
                        *slot = v as u16;
                    }
                }
                "bold_family" | "italic_family" => {
                    if let Some(v) = self.string(key, value) {
                        let v = Some(v.trim().to_owned()).filter(|v| !v.is_empty());
                        if key.get_ref() == "bold_family" {
                            font.bold_family = v;
                        } else {
                            font.italic_family = v;
                        }
                    }
                }
                "features" => {
                    if let Some(list) = self.strings(key, value) {
                        match list.iter().find(|f| crate::feature(f).is_none()) {
                            Some(bad) => {
                                let message = format!(
                                    "“{bad}” isn't a feature; they look like \"ss01\", \"-calt\" or \"cv02=3\""
                                );
                                let d = Diagnostic::new(self.line(value.span()), message).at(value.span());
                                self.diagnostics.push(d);
                            }
                            None => font.features = list,
                        }
                    }
                }
                _ => self.unknown(key, "font setting", &FONT),
            }
        }
    }

    pub(super) fn colors(&mut self, table: &DeTable, config: &mut Config) {
        let colors = &mut config.colors;
        for (key, value) in table.iter() {
            match key.get_ref().as_ref() {
                "foreground" => colors.foreground = self.color(key, value).or(colors.foreground),
                "background" => colors.background = self.color(key, value).or(colors.background),
                "cursor" => colors.cursor = self.color(key, value).or(colors.cursor),
                "selection" => colors.selection = self.color(key, value).or(colors.selection),
                "accent" => colors.accent = self.color(key, value).or(colors.accent),
                "apolline" => {
                    if let Some(v) = self.boolean(key, value) {
                        colors.apolline = v;
                    }
                }
                "from" => {
                    let Some(from) = self.string(key, value) else {
                        continue;
                    };
                    let path = crate::scheme_path(from);
                    let scheme = std::fs::read_to_string(&path)
                        .ok()
                        .and_then(|t| crate::import::read(&t));
                    match &scheme {
                        Some(scheme) => colors.base = scheme.over(colors.base),
                        None => {
                            let message = format!("no color scheme in {}; the colors stay as they are", path.display());
                            let d = Diagnostic::new(self.line(value.span()), message).at(value.span());
                            self.diagnostics.push(d);
                        }
                    }
                    colors.from = Some(from.to_owned());
                    colors.scheme = scheme;
                }
                "palette" => {
                    if let Some(list) = self.palette(key, value) {
                        colors.palette = list;
                    }
                }
                _ => self.unknown(key, "color setting", &COLORS),
            }
        }
    }

    pub(super) fn quick(&mut self, table: &DeTable, config: &mut Config) {
        let quick = &mut config.quick;
        for (key, value) in table.iter() {
            match key.get_ref().as_ref() {
                "key" => {
                    if let Some(v) = self.string(key, value) {
                        quick.key = crate::normalize_combo(v);
                    }
                }
                "height" => quick.height = self.float(key, value, 0.15..=1.0).unwrap_or(quick.height),
                "hide_on_blur" => quick.hide_on_blur = self.boolean(key, value).unwrap_or(quick.hide_on_blur),
                _ => self.unknown(key, "quick setting", &QUICK),
            }
        }
    }

    pub(super) fn ssh(&mut self, table: &DeTable, config: &mut Config) {
        for (key, value) in table.iter() {
            match key.get_ref().as_ref() {
                "udp" => config.ssh.udp = self.choice(key, value, &UDP).unwrap_or(config.ssh.udp),
                _ => self.unknown(key, "ssh setting", &SSH),
            }
        }
    }

    pub(super) fn ai(&mut self, table: &DeTable, config: &mut Config) {
        let ai = &mut config.ai;
        for (key, value) in table.iter() {
            match key.get_ref().as_ref() {
                "enabled" => ai.enabled = self.boolean(key, value).unwrap_or(ai.enabled),
                "url" | "model" | "key_env" => {
                    let Some(v) = self.string(key, value).map(|v| v.trim().to_owned()) else {
                        continue;
                    };
                    match key.get_ref().as_ref() {
                        "url" => ai.url = v.trim_end_matches('/').to_owned(),
                        "model" => ai.model = v,
                        _ => ai.key_env = v,
                    }
                }
                "send" => {
                    if let Some(v) = self.integer(key, value, 500..=200_000) {
                        ai.send = v as u32;
                    }
                }
                _ => self.unknown(key, "ai setting", &AI),
            }
        }
    }

    pub(super) fn greeting(&mut self, table: &DeTable, config: &mut Config) {
        let greeting = &mut config.greeting;
        for (key, value) in table.iter() {
            match key.get_ref().as_ref() {
                "show" => greeting.show = self.choice(key, value, &GREETING_SHOW).unwrap_or(greeting.show),
                "file" => {
                    if let Some(v) = self.string(key, value) {
                        greeting.file = Some(v.trim().to_owned()).filter(|v| !v.is_empty());
                    }
                }
                "blocks" => greeting.blocks = self.boolean(key, value).unwrap_or(greeting.blocks),
                "info" => greeting.info = self.boolean(key, value).unwrap_or(greeting.info),
                "height" => {
                    if let Some(v) = self.integer(key, value, 3..=60) {
                        greeting.height = v as u16;
                    }
                }
                "loops" => {
                    if let Some(v) = self.integer(key, value, 1..=10) {
                        greeting.loops = v as u8;
                    }
                }
                _ => self.unknown(key, "greeting setting", &GREETING),
            }
        }
    }

    pub(super) fn background(&mut self, table: &DeTable, config: &mut Config) {
        let background = &mut config.background;
        for (key, value) in table.iter() {
            match key.get_ref().as_ref() {
                "image" => {
                    if let Some(v) = self.string(key, value) {
                        background.image = Some(v.trim().to_owned()).filter(|v| !v.is_empty());
                    }
                }
                "fit" => {
                    if let Some(v) = self.choice(key, value, &FITS) {
                        background.fit = v;
                    }
                }
                "dim" => {
                    if let Some(v) = self.float(key, value, 0.0..=1.0) {
                        background.dim = v;
                    }
                }
                "blur" => {
                    if let Some(v) = self.integer(key, value, 0..=64) {
                        background.blur = v as u8;
                    }
                }
                _ => self.unknown(key, "background setting", &BACKGROUND),
            }
        }
    }

    pub(super) fn window(&mut self, table: &DeTable, config: &mut Config) {
        let window = &mut config.window;
        for (key, value) in table.iter() {
            match key.get_ref().as_ref() {
                "shader" => {
                    let list = match value.get_ref().as_str() {
                        Some(one) => Some(vec![one.to_owned()]),
                        None => self.strings(key, value),
                    };
                    if let Some(list) = list {
                        window.shaders = list.into_iter().filter(|s| !s.trim().is_empty()).collect();
                    }
                }
                "padding" => {
                    if let Some(v) = self.padding(key, value) {
                        window.padding = v;
                    }
                }
                "opacity" => {
                    if let Some(v) = self.float(key, value, 0.0..=1.0) {
                        window.opacity = v;
                    }
                }
                "blur" => {
                    if let Some(v) = self.integer(key, value, 0..=64) {
                        window.blur = v as u8;
                    }
                }
                "columns" => {
                    if let Some(v) = self.integer(key, value, 10..=1000) {
                        window.columns = v as u16;
                    }
                }
                "lines" => {
                    if let Some(v) = self.integer(key, value, 4..=500) {
                        window.lines = v as u16;
                    }
                }
                "decorations" => {
                    if let Some(v) = self.choice(key, value, &DECORATIONS) {
                        window.decorations = v;
                    }
                }
                "radius" => {
                    if let Some(v) = self.float(key, value, 0.0..=24.0) {
                        window.radius = v;
                    }
                }
                _ => self.unknown(key, "window setting", &WINDOW),
            }
        }
    }

    pub(super) fn cursor(&mut self, table: &DeTable, config: &mut Config) {
        for (key, value) in table.iter() {
            match key.get_ref().as_ref() {
                "shape" => {
                    if let Some(v) = self.choice(key, value, &SHAPES) {
                        config.cursor.shape = v;
                    }
                }
                "blink" => {
                    if let Some(v) = self.boolean(key, value) {
                        config.cursor.blink = v;
                    }
                }
                "trail" => {
                    if let Some(v) = self.boolean(key, value) {
                        config.cursor.trail = v;
                    }
                }
                _ => self.unknown(key, "cursor setting", &CURSOR),
            }
        }
    }

    pub(super) fn terminal(&mut self, table: &DeTable, config: &mut Config) {
        let term = &mut config.terminal;
        for (key, value) in table.iter() {
            match key.get_ref().as_ref() {
                "shell" => {
                    if let Some(v) = self.string(key, value) {
                        term.shell = (!v.trim().is_empty()).then(|| v.trim().to_owned());
                    }
                }
                "args" => {
                    if let Some(v) = self.strings(key, value) {
                        term.args = v;
                    }
                }
                "scrollback" => {
                    if let Some(v) = self.integer(key, value, 0..=1_000_000) {
                        term.scrollback = v as u32;
                    }
                }
                "term" => {
                    if let Some(v) = self.string(key, value) {
                        term.term = v.trim().to_owned();
                    }
                }
                "osc52" => {
                    if let Some(v) = self.choice(key, value, &OSC52) {
                        term.osc52 = v;
                    }
                }
                "kitty_keyboard" => {
                    if let Some(v) = self.boolean(key, value) {
                        term.kitty_keyboard = v;
                    }
                }
                "option_as_alt" => {
                    if let Some(v) = self.choice(key, value, &OPTION_AS_ALT) {
                        term.option_as_alt = v;
                    }
                }
                "shell_integration" => {
                    if let Some(v) = self.boolean(key, value) {
                        term.shell_integration = v;
                    }
                }
                "notify_after" => {
                    if let Some(v) = self.integer(key, value, 0..=86_400) {
                        term.notify_after = v as u32;
                    }
                }
                "sessions" => {
                    if let Some(v) = self.boolean(key, value) {
                        term.sessions = v;
                    }
                }
                "archive_days" => {
                    if let Some(v) = self.integer(key, value, 0..=3650) {
                        term.archive_days = v as u32;
                    }
                }
                _ => self.unknown(key, "terminal setting", &TERMINAL),
            }
        }
    }

    pub(super) fn tabs(&mut self, table: &DeTable, config: &mut Config) {
        let tabs = &mut config.tabs;
        for (key, value) in table.iter() {
            match key.get_ref().as_ref() {
                "position" => tabs.position = self.choice(key, value, &POSITIONS).unwrap_or(tabs.position),
                "style" => tabs.style = self.choice(key, value, &TAB_STYLES).unwrap_or(tabs.style),
                "show" => tabs.show = self.choice(key, value, &TAB_SHOWS).unwrap_or(tabs.show),
                "title" => {
                    if let Some(v) = self.string(key, value) {
                        tabs.title = v.to_owned();
                    }
                }
                "detail" => tabs.detail = self.boolean(key, value).unwrap_or(tabs.detail),
                "plus" => tabs.plus = self.boolean(key, value).unwrap_or(tabs.plus),
                _ => self.unknown(key, "tabs setting", &TABS),
            }
        }
    }

    pub(super) fn panes(&mut self, table: &DeTable, config: &mut Config) {
        let panes = &mut config.panes;
        for (key, value) in table.iter() {
            match key.get_ref().as_ref() {
                "divider" => panes.divider = self.choice(key, value, &DIVIDERS).unwrap_or(panes.divider),
                "dim" => panes.dim = self.float(key, value, 0.0..=0.9).unwrap_or(panes.dim),
                _ => self.unknown(key, "panes setting", &PANES),
            }
        }
    }

    pub(super) fn badges(&mut self, table: &DeTable, config: &mut Config) {
        let badges = &mut config.badges;
        for (key, value) in table.iter() {
            let slot = match key.get_ref().as_ref() {
                "show" => {
                    badges.show = self.choice(key, value, &BADGE_SHOWS).unwrap_or(badges.show);
                    continue;
                }
                "clock" => {
                    badges.clock = self.boolean(key, value).unwrap_or(badges.clock);
                    continue;
                }
                "ok" => &mut badges.ok,
                "failed" => &mut badges.failed,
                "running" => &mut badges.running,
                _ => {
                    self.unknown(key, "badges setting", &BADGES);
                    continue;
                }
            };
            match value.get_ref().as_str() {
                Some(v) if v.chars().count() <= 4 => *slot = v.to_owned(),
                _ => {
                    self.wrong::<()>(key, value, "a mark of up to 4 characters, like \"✓\"");
                }
            }
        }
    }

    pub(super) fn hints(&mut self, table: &DeTable, config: &mut Config) {
        for (key, value) in table.iter() {
            if key.get_ref() != "alphabet" {
                self.unknown(key, "hints setting", &HINTS);
                continue;
            }
            let letters = value.get_ref().as_str().filter(|v| {
                let mut seen: Vec<char> = v.chars().collect();
                seen.sort_unstable();
                seen.dedup();
                seen.len() == v.chars().count() && seen.len() >= 2 && v.chars().all(|c| c.is_ascii_lowercase())
            });
            match letters {
                Some(v) => config.hints.alphabet = v.to_owned(),
                None => {
                    self.wrong::<()>(
                        key,
                        value,
                        "two or more different small latin letters, like \"asdfjkl\"",
                    );
                }
            }
        }
    }

    pub(super) fn rules(&mut self, key: &Key, value: &Value, config: &mut Config) {
        let tables: Option<Vec<&Value>> = match value.get_ref() {
            DeValue::Array(items) => items
                .iter()
                .map(|item| matches!(item.get_ref(), DeValue::Table(_)).then_some(item))
                .collect(),
            _ => None,
        };
        let Some(tables) = tables else {
            self.wrong::<()>(key, value, "[[rules]] sections");
            return;
        };
        let names: Vec<String> = themes::all().into_iter().map(|t| t.name).collect();
        let known: Vec<&str> = names.iter().map(String::as_str).collect();
        for item in tables {
            let DeValue::Table(table) = item.get_ref() else {
                continue;
            };
            let mut rule = crate::Rule {
                host: None,
                dir: None,
                branch: None,
                theme: None,
                accent: None,
                background: None,
                label: None,
            };
            for (k, v) in table.iter() {
                match k.get_ref().as_ref() {
                    "host" => rule.host = self.string(k, v).map(str::to_owned),
                    "dir" => rule.dir = self.string(k, v).map(str::to_owned),
                    "branch" => rule.branch = self.string(k, v).map(str::to_owned),
                    "label" => rule.label = self.string(k, v).map(str::to_owned),
                    "accent" => rule.accent = self.color(k, v),
                    "background" => rule.background = self.color(k, v),
                    "theme" => {
                        let Some(name) = self.string(k, v) else {
                            continue;
                        };
                        if known.contains(&name) {
                            rule.theme = Some(name.to_owned());
                            continue;
                        }
                        let message = format!("there is no theme “{name}”");
                        let mut d = Diagnostic::new(self.line(v.span()), message).at(v.span());
                        if let Some(close) = suggest(name, &known) {
                            d = d
                                .hint(format!("did you mean “{close}”?"))
                                .fix(v.span(), format!("\"{close}\""));
                        }
                        self.diagnostics.push(d);
                    }
                    _ => self.unknown(k, "rule setting", &RULE),
                }
            }
            if rule.host.is_none() && rule.dir.is_none() && rule.branch.is_none() {
                let d = Diagnostic::new(
                    self.line(item.span()),
                    "a rule needs a host, a dir or a branch to match",
                )
                .at(item.span());
                self.diagnostics.push(d);
                continue;
            }
            config.rules.push(rule);
        }
    }

    pub(super) fn keys(&mut self, table: &DeTable, config: &mut Config) {
        let mut known: Vec<&str> = ACTIONS.to_vec();
        known.push("none");
        for (combo, value) in table.iter() {
            if combo.get_ref() == "like" {
                config.keys_like = self.choice(combo, value, &PRESETS.map(|p| (p, p))).or(config.keys_like);
                continue;
            }
            let Some(action) = self.string(combo, value) else {
                continue;
            };
            if !combo_is_valid(combo.get_ref()) {
                let message = format!("“{}” is not a key combination", combo.get_ref());
                let d = Diagnostic::new(self.line(combo.span()), message)
                    .at(combo.span())
                    .hint("write it like cmd+t or ctrl+shift+c");
                self.diagnostics.push(d);
                continue;
            }
            if known.contains(&action) {
                config.keys.push((combo.get_ref().to_string(), action.to_owned()));
                continue;
            }
            let message = format!("there is no action “{action}”");
            let mut d = Diagnostic::new(self.line(value.span()), message).at(value.span());
            d = match suggest(action, &known) {
                Some(close) => d
                    .hint(format!("did you mean “{close}”?"))
                    .fix(value.span(), format!("\"{close}\"")),
                None => d.hint("see the list in settings → Keys"),
            };
            self.diagnostics.push(d);
        }
    }
}
