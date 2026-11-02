use std::ops::{Range, RangeInclusive};

use toml::Spanned;
use toml::de::{DeString, DeTable, DeValue};

use crate::{
    ACTIONS, BadgeShow, Config, CursorShape, Decorations, Diagnostic, Divider, Fit, GreetingShow, OptionAsAlt, Osc52,
    Rgb, TabPosition, TabShow, TabStyle, themes,
};

type Key<'i> = Spanned<DeString<'i>>;
type Value<'i> = Spanned<DeValue<'i>>;

const SECTIONS: [&str; 13] = [
    "font",
    "colors",
    "window",
    "cursor",
    "terminal",
    "keys",
    "tabs",
    "panes",
    "badges",
    "hints",
    "background",
    "quick",
    "greeting",
];
const QUICK: [&str; 3] = ["key", "height", "hide_on_blur"];
const GREETING: [&str; 6] = ["show", "file", "blocks", "info", "height", "loops"];
const GREETING_SHOW: [(&str, GreetingShow); 3] = [
    ("off", GreetingShow::Off),
    ("window", GreetingShow::Window),
    ("always", GreetingShow::Always),
];
const BACKGROUND: [&str; 4] = ["image", "fit", "dim", "blur"];
const FITS: [(&str, Fit); 5] = [
    ("cover", Fit::Cover),
    ("contain", Fit::Contain),
    ("stretch", Fit::Stretch),
    ("center", Fit::Center),
    ("tile", Fit::Tile),
];
const TOP: [&str; 2] = ["theme", "rules"];
const RULE: [&str; 6] = ["host", "dir", "theme", "accent", "background", "label"];
const THEME_TOP: [&str; 3] = ["name", "author", "extends"];
const TABS: [&str; 6] = ["position", "style", "show", "title", "detail", "plus"];
const PANES: [&str; 2] = ["divider", "dim"];
const BADGES: [&str; 5] = ["show", "ok", "failed", "running", "clock"];
const HINTS: [&str; 1] = ["alphabet"];
const FONT: [&str; 9] = [
    "family",
    "size",
    "line_height",
    "ligatures",
    "weight",
    "bold_weight",
    "bold_family",
    "italic_family",
    "features",
];
const COLORS: [&str; 8] = [
    "from",
    "apolline",
    "foreground",
    "background",
    "cursor",
    "selection",
    "accent",
    "palette",
];
const WINDOW: [&str; 8] = [
    "shader",
    "padding",
    "opacity",
    "blur",
    "columns",
    "lines",
    "decorations",
    "radius",
];
const CURSOR: [&str; 3] = ["shape", "blink", "trail"];
const TERMINAL: [&str; 11] = [
    "shell",
    "args",
    "scrollback",
    "term",
    "osc52",
    "kitty_keyboard",
    "option_as_alt",
    "shell_integration",
    "notify_after",
    "archive_days",
    "sessions",
];

const DECORATIONS: [(&str, Decorations); 3] = [
    ("full", Decorations::Full),
    ("transparent", Decorations::Transparent),
    ("none", Decorations::None),
];
const SHAPES: [(&str, CursorShape); 3] = [
    ("block", CursorShape::Block),
    ("beam", CursorShape::Beam),
    ("underline", CursorShape::Underline),
];
const OSC52: [(&str, Osc52); 4] = [
    ("off", Osc52::Off),
    ("copy", Osc52::Copy),
    ("paste", Osc52::Paste),
    ("both", Osc52::Both),
];
const POSITIONS: [(&str, TabPosition); 2] = [("top", TabPosition::Top), ("bottom", TabPosition::Bottom)];
const TAB_STYLES: [(&str, TabStyle); 4] = [
    ("pills", TabStyle::Pills),
    ("underline", TabStyle::Underline),
    ("powerline", TabStyle::Powerline),
    ("plain", TabStyle::Plain),
];
const TAB_SHOWS: [(&str, TabShow); 3] = [
    ("many", TabShow::Many),
    ("always", TabShow::Always),
    ("never", TabShow::Never),
];
const DIVIDERS: [(&str, Divider); 2] = [("line", Divider::Line), ("gap", Divider::Gap)];
const BADGE_SHOWS: [(&str, BadgeShow); 3] = [
    ("all", BadgeShow::All),
    ("failures", BadgeShow::Failures),
    ("none", BadgeShow::None),
];
const OPTION_AS_ALT: [(&str, OptionAsAlt); 4] = [
    ("none", OptionAsAlt::None),
    ("left", OptionAsAlt::Left),
    ("right", OptionAsAlt::Right),
    ("both", OptionAsAlt::Both),
];

/// Every key vif.toml knows, as `section.key`, and the top-level ones bare.
pub fn known() -> Vec<String> {
    let sections: [(&str, &[&str]); 12] = [
        ("font", &FONT),
        ("colors", &COLORS),
        ("window", &WINDOW),
        ("cursor", &CURSOR),
        ("terminal", &TERMINAL),
        ("tabs", &TABS),
        ("panes", &PANES),
        ("badges", &BADGES),
        ("hints", &HINTS),
        ("background", &BACKGROUND),
        ("quick", &QUICK),
        ("greeting", &GREETING),
    ];
    let mut out: Vec<String> = TOP.iter().map(|k| (*k).to_owned()).collect();
    for (section, keys) in sections {
        out.extend(keys.iter().map(|k| format!("{section}.{k}")));
    }
    out
}

/// vif.toml over its theme over the defaults.
pub(crate) fn document(src: &str) -> (Config, Vec<Diagnostic>) {
    let mut config = Config::default();
    let diagnostics = apply(src, &mut config, false);
    (config, diagnostics)
}

/// Puts what `src` sets onto `config`. A theme file may name and extend itself but only sets
/// the look; vif.toml lays its theme down first.
pub(crate) fn apply(src: &str, config: &mut Config, theme_file: bool) -> Vec<Diagnostic> {
    let (doc, errors) = DeTable::parse_recoverable(src);
    let mut reader = Reader {
        src,
        diagnostics: Vec::new(),
        theme_file,
    };
    for error in &errors {
        let span = error.span().unwrap_or(0..0);
        let message = error.message().lines().next().unwrap_or("syntax error");
        let d = Diagnostic::new(reader.line(span.clone()), message).at(span);
        reader.diagnostics.push(d);
    }
    reader.root(doc.get_ref(), config);
    reader.diagnostics.sort_by_key(|d| d.line);
    reader.diagnostics
}

struct Reader<'s> {
    src: &'s str,
    diagnostics: Vec<Diagnostic>,
    theme_file: bool,
}

impl Reader<'_> {
    fn root(&mut self, table: &DeTable, config: &mut Config) {
        if !self.theme_file {
            self.theme(table, config);
        }
        let top: &[&str] = if self.theme_file { &THEME_TOP } else { &TOP };
        for (key, value) in table.iter() {
            let name: &str = key.get_ref();
            let DeValue::Table(section) = value.get_ref() else {
                match name {
                    "theme" if !self.theme_file => {}
                    "rules" if !self.theme_file => self.rules(key, value, config),
                    "name" | "author" | "extends" if self.theme_file => {
                        self.string(key, value);
                    }
                    _ if SECTIONS.contains(&name) => {
                        self.wrong::<()>(key, value, "a section like [font]");
                    }
                    _ => self.unknown(key, "setting", top),
                }
                continue;
            };
            if !SECTIONS.contains(&name) {
                self.unknown(key, "section", &SECTIONS);
                continue;
            }
            if self.theme_file && matches!(name, "terminal" | "keys") {
                let message = format!("a theme sets the look; [{name}] belongs in vif.toml");
                let d = Diagnostic::new(self.line(key.span()), message).at(key.span());
                self.diagnostics.push(d);
                continue;
            }
            match name {
                "font" => self.font(section, config),
                "colors" => self.colors(section, config),
                "window" => self.window(section, config),
                "cursor" => self.cursor(section, config),
                "terminal" => self.terminal(section, config),
                "tabs" => self.tabs(section, config),
                "panes" => self.panes(section, config),
                "badges" => self.badges(section, config),
                "hints" => self.hints(section, config),
                "background" => self.background(section, config),
                "quick" => self.quick(section, config),
                "greeting" => self.greeting(section, config),
                _ => self.keys(section, config),
            }
        }
    }

    /// Lays the theme down before anything else here, so all of this goes on top of it.
    fn theme(&mut self, table: &DeTable, config: &mut Config) {
        let mut at = None;
        if let Some((key, value)) = table.iter().find(|(k, _)| k.get_ref() == "theme")
            && let Some(name) = self.string(key, value)
        {
            let names: Vec<String> = themes::all().into_iter().map(|t| t.name).collect();
            let name = name.trim();
            if names.iter().any(|n| n == name) {
                config.theme = name.to_owned();
                at = Some(value.span());
            } else {
                let known: Vec<&str> = names.iter().map(String::as_str).collect();
                let message = format!("there is no theme “{name}”");
                let mut d = Diagnostic::new(self.line(value.span()), message).at(value.span());
                d = match suggest(name, &known) {
                    Some(close) => d
                        .hint(format!("did you mean “{close}”?"))
                        .fix(value.span(), format!("\"{close}\"")),
                    None => d.hint(format!("there are {}", known.join(", "))),
                };
                self.diagnostics.push(d);
            }
        }
        for problem in themes::lay(&config.theme.clone(), config) {
            let line = at.clone().map_or(0, |span| self.line(span));
            let d = Diagnostic::new(line, format!("theme {problem}")).at(at.clone().unwrap_or(0..0));
            self.diagnostics.push(d);
        }
    }

    fn font(&mut self, table: &DeTable, config: &mut Config) {
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

    fn colors(&mut self, table: &DeTable, config: &mut Config) {
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

    fn quick(&mut self, table: &DeTable, config: &mut Config) {
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

    fn greeting(&mut self, table: &DeTable, config: &mut Config) {
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

    fn background(&mut self, table: &DeTable, config: &mut Config) {
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

    fn window(&mut self, table: &DeTable, config: &mut Config) {
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

    fn cursor(&mut self, table: &DeTable, config: &mut Config) {
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

    fn terminal(&mut self, table: &DeTable, config: &mut Config) {
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

    fn tabs(&mut self, table: &DeTable, config: &mut Config) {
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

    fn panes(&mut self, table: &DeTable, config: &mut Config) {
        let panes = &mut config.panes;
        for (key, value) in table.iter() {
            match key.get_ref().as_ref() {
                "divider" => panes.divider = self.choice(key, value, &DIVIDERS).unwrap_or(panes.divider),
                "dim" => panes.dim = self.float(key, value, 0.0..=0.9).unwrap_or(panes.dim),
                _ => self.unknown(key, "panes setting", &PANES),
            }
        }
    }

    fn badges(&mut self, table: &DeTable, config: &mut Config) {
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

    fn hints(&mut self, table: &DeTable, config: &mut Config) {
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

    fn rules(&mut self, key: &Key, value: &Value, config: &mut Config) {
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
                theme: None,
                accent: None,
                background: None,
                label: None,
            };
            for (k, v) in table.iter() {
                match k.get_ref().as_ref() {
                    "host" => rule.host = self.string(k, v).map(str::to_owned),
                    "dir" => rule.dir = self.string(k, v).map(str::to_owned),
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
            if rule.host.is_none() && rule.dir.is_none() {
                let d =
                    Diagnostic::new(self.line(item.span()), "a rule needs a host or a dir to match").at(item.span());
                self.diagnostics.push(d);
                continue;
            }
            config.rules.push(rule);
        }
    }

    fn keys(&mut self, table: &DeTable, config: &mut Config) {
        let mut known: Vec<&str> = ACTIONS.to_vec();
        known.push("none");
        for (combo, value) in table.iter() {
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

    fn string<'a>(&mut self, key: &Key, value: &'a Value) -> Option<&'a str> {
        value
            .get_ref()
            .as_str()
            .or_else(|| self.wrong(key, value, "text in quotes"))
    }

    fn strings(&mut self, key: &Key, value: &Value) -> Option<Vec<String>> {
        let list = value.get_ref().as_array().and_then(|items| {
            items
                .iter()
                .map(|item| item.get_ref().as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
        });
        list.or_else(|| self.wrong(key, value, r#"a list like ["-l", "-i"]"#))
    }

    fn boolean(&mut self, key: &Key, value: &Value) -> Option<bool> {
        value
            .get_ref()
            .as_bool()
            .or_else(|| self.wrong(key, value, "true or false"))
    }

    fn integer(&mut self, key: &Key, value: &Value, range: RangeInclusive<i64>) -> Option<i64> {
        let parsed = value
            .get_ref()
            .as_integer()
            .and_then(|n| i64::from_str_radix(n.as_str(), n.radix()).ok());
        match parsed {
            Some(n) if range.contains(&n) => Some(n),
            _ => self.wrong(
                key,
                value,
                &format!("a whole number from {} to {}", range.start(), range.end()),
            ),
        }
    }

    fn float(&mut self, key: &Key, value: &Value, range: RangeInclusive<f32>) -> Option<f32> {
        match number(value) {
            Some(v) if range.contains(&v) => Some(v),
            _ => self.wrong(
                key,
                value,
                &format!("a number from {} to {}", range.start(), range.end()),
            ),
        }
    }

    fn color(&mut self, key: &Key, value: &Value) -> Option<Rgb> {
        value
            .get_ref()
            .as_str()
            .and_then(Rgb::parse)
            .or_else(|| self.wrong(key, value, r##"a color like "#86bdf0""##))
    }

    fn palette(&mut self, key: &Key, value: &Value) -> Option<[Option<Rgb>; 16]> {
        let items = value.get_ref().as_array();
        let colors = items.and_then(|items| {
            (items.len() <= 16)
                .then(|| {
                    items
                        .iter()
                        .map(|item| item.get_ref().as_str().and_then(Rgb::parse))
                        .collect::<Option<Vec<_>>>()
                })
                .flatten()
        });
        let Some(colors) = colors else {
            return self.wrong(key, value, "a list of up to 16 colors like \"#1e2026\"");
        };
        let mut out = [None; 16];
        for (slot, color) in out.iter_mut().zip(colors) {
            *slot = Some(color);
        }
        Some(out)
    }

    fn padding(&mut self, key: &Key, value: &Value) -> Option<[u16; 2]> {
        let whole = |v: &Value| {
            v.get_ref()
                .as_integer()
                .and_then(|n| i64::from_str_radix(n.as_str(), n.radix()).ok())
                .filter(|n| (0..=200).contains(n))
                .map(|n| n as u16)
        };
        if let Some(n) = whole(value) {
            return Some([n, n]);
        }
        let pair = value.get_ref().as_array().and_then(|items| {
            let items: Vec<&Value> = items.iter().collect();
            match items[..] {
                [x, y] => Some([whole(x)?, whole(y)?]),
                _ => None,
            }
        });
        pair.or_else(|| self.wrong(key, value, "pixels like 12 or [12, 10] from 0 to 200"))
    }

    fn choice<T: Copy>(&mut self, key: &Key, value: &Value, options: &[(&str, T)]) -> Option<T> {
        let names: Vec<&str> = options.iter().map(|(name, _)| *name).collect();
        let Some(text) = value.get_ref().as_str() else {
            return self.wrong(key, value, &format!("one of {}", quoted(&names)));
        };
        if let Some((_, choice)) = options.iter().find(|(name, _)| name.eq_ignore_ascii_case(text)) {
            return Some(*choice);
        }
        let message = format!("“{text}” is not a valid {}", key.get_ref());
        let mut d = Diagnostic::new(self.line(value.span()), message).at(value.span());
        d = match suggest(text, &names) {
            Some(close) => d
                .hint(format!("did you mean “{close}”?"))
                .fix(value.span(), format!("\"{close}\"")),
            None => d.hint(format!("use one of {}", quoted(&names))),
        };
        self.diagnostics.push(d);
        None
    }

    fn wrong<T>(&mut self, key: &Key, value: &Value, expected: &str) -> Option<T> {
        let message = format!("“{}” wants {expected}", key.get_ref());
        let d = Diagnostic::new(self.line(value.span()), message)
            .at(value.span())
            .hint("kept the default");
        self.diagnostics.push(d);
        None
    }

    fn unknown(&mut self, key: &Key, what: &str, known: &[&str]) {
        let name = key.get_ref();
        let mut d = Diagnostic::new(self.line(key.span()), format!("unknown {what} “{name}”")).at(key.span());
        d = match suggest(name, known) {
            Some(close) => d
                .hint(format!("did you mean “{close}”?"))
                .fix(self.key_span(key), close),
            None => d.hint(format!("known: {}", known.join(", "))),
        };
        self.diagnostics.push(d);
    }

    /// The key's span without surrounding quotes or brackets.
    fn key_span(&self, key: &Key) -> Range<usize> {
        let span = key.span();
        let text = &self.src[span.clone()];
        match text.find(key.get_ref().as_ref() as &str) {
            Some(offset) => span.start + offset..span.start + offset + key.get_ref().len(),
            None => span,
        }
    }

    fn line(&self, span: Range<usize>) -> usize {
        let end = span.start.min(self.src.len());
        self.src.as_bytes()[..end].iter().filter(|&&b| b == b'\n').count() + 1
    }
}

fn number(value: &Value) -> Option<f32> {
    match value.get_ref() {
        DeValue::Float(f) => f.as_str().parse::<f32>().ok(),
        DeValue::Integer(n) => n.as_str().parse::<f32>().ok(),
        _ => None,
    }
}

fn combo_is_valid(combo: &str) -> bool {
    let parts: Vec<&str> = combo.split('+').collect();
    !combo.is_empty() && parts.last().is_some_and(|key| !key.is_empty() || combo.ends_with("++"))
}

pub(crate) fn suggest<'a>(input: &str, known: &[&'a str]) -> Option<&'a str> {
    known
        .iter()
        .map(|candidate| (*candidate, strsim::jaro_winkler(input, candidate)))
        .filter(|(_, score)| *score >= 0.8)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(candidate, _)| candidate)
}

fn quoted(names: &[&str]) -> String {
    names
        .iter()
        .map(|name| format!("“{name}”"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apolline_accent_fills_unset_colors_only() {
        let outside = Some(Rgb::hex(0xff6a3d));
        let (config, _) = document("[colors]\ncursor = \"#ffffff\"\n");
        let p = config.palette_with(outside);
        assert_eq!(p.accent, Rgb::hex(0xff6a3d));
        assert_eq!(p.cursor, Rgb::hex(0xffffff));
        let (off, _) = document("[colors]\napolline = false\n");
        assert_eq!(off.palette_with(outside), off.palette());
    }

    #[test]
    fn typo_is_reported_and_the_rest_applies() {
        let src = "[window]\nopactiy = 0.9\nblur = 20\n\n[font]\nsize = \"big\"\nfamily = \"JetBrains Mono\"\n";
        let (config, diagnostics) = document(src);
        assert_eq!(config.window.blur, 20);
        assert_eq!(config.font.family, "JetBrains Mono");
        assert_eq!(config.font.size, 13.0);
        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].line, 2);
        assert_eq!(diagnostics[0].hint.as_deref(), Some("did you mean “opacity”?"));
        let fixed = diagnostics[0].fix.as_ref().unwrap().apply(src).unwrap();
        assert!(fixed.contains("\nopacity = 0.9\n"));
        assert_eq!(diagnostics[1].line, 6);
    }

    #[test]
    fn greeting_reads_its_section() {
        let src = "[greeting]\nshow = \"always\"\nfile = \" ~/cat.gif \"\nblocks = false\nheight = 80\n";
        let (config, diagnostics) = document(src);
        assert_eq!(config.greeting.show, GreetingShow::Always);
        assert_eq!(config.greeting.file.as_deref(), Some("~/cat.gif"));
        assert!(!config.greeting.blocks);
        assert_eq!(config.greeting.height, 12);
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn choices_suggest_and_fix() {
        let src = "[cursor]\nshape = \"bem\"\n";
        let (config, diagnostics) = document(src);
        assert_eq!(config.cursor.shape, CursorShape::Block);
        let fixed = diagnostics[0].fix.as_ref().unwrap().apply(src).unwrap();
        assert_eq!(fixed, "[cursor]\nshape = \"beam\"\n");
    }

    #[test]
    fn colors_follow_a_scheme_file() {
        let path = std::env::temp_dir().join(format!("vif-wal-{}.json", std::process::id()));
        let colors: Vec<String> = (0..16).map(|i| format!("\"color{i}\": \"#0000{i:02x}\"")).collect();
        let wal = format!(
            r##"{{"special": {{"background": "#101010", "foreground": "#eeeeee"}}, "colors": {{{}}}}}"##,
            colors.join(", ")
        );
        std::fs::write(&path, wal).unwrap();
        let src = format!("[colors]\nfrom = \"{}\"\nforeground = \"#ff0000\"\n", path.display());
        let (config, diagnostics) = Config::parse(&src);
        let _ = std::fs::remove_file(&path);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let palette = config.palette();
        assert_eq!(palette.background, crate::Rgb::hex(0x101010));
        assert_eq!(palette.foreground, crate::Rgb::hex(0xff0000));
        assert_eq!(palette.ansi[12], crate::Rgb::hex(0x00000c));
        assert_eq!(palette.accent, crate::Rgb::hex(0x000004));
        let (_, missing) = Config::parse("[colors]\nfrom = \"/nowhere/colors.json\"\n");
        assert_eq!(missing.len(), 1);
    }

    #[test]
    fn rules_read_and_complain() {
        let src = "[[rules]]\nhost = \"prod-*\"\ntheme = \"alarm\"\nlabel = \"PROD\"\n\n[[rules]]\nlabel = \"nowhere\"\n\n[[rules]]\ndir = \"~/infra\"\ntheme = \"alrm\"\n";
        let (config, diagnostics) = document(src);
        assert_eq!(config.rules.len(), 2);
        assert_eq!(config.rules[0].label.as_deref(), Some("PROD"));
        assert_eq!(config.rules[1].theme, None);
        assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
        let fixed = diagnostics[1].fix.as_ref().unwrap().apply(src).unwrap();
        assert!(fixed.ends_with("theme = \"alarm\"\n"));
    }

    #[test]
    fn syntax_error_keeps_the_rest() {
        let src = "[font]\nsize = 15\nfamily = \"unterminated\n[window]\nblur = 10\n";
        let (config, diagnostics) = document(src);
        assert!(!diagnostics.is_empty());
        assert_eq!(config.font.size, 15.0);
    }

    #[test]
    fn keys_override_defaults() {
        let src = "[keys]\n\"cmd+shift+t\" = \"new_tab\"\n\"cmd+t\" = \"none\"\n\"ctrl+x\" = \"spilt_right\"\n";
        let (config, diagnostics) = document(src);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].hint.as_deref(), Some("did you mean “split_right”?"));
        let bindings = config.bindings();
        assert!(bindings.iter().any(|(c, a)| c == "cmd+shift+t" && a == "new_tab"));
        assert!(!bindings.iter().any(|(c, _)| c == "cmd+t"));
    }
}
