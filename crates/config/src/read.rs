use std::ops::{Range, RangeInclusive};

use toml::Spanned;
use toml::de::{DeString, DeTable, DeValue};

use crate::{
    ACTIONS, BadgeShow, Config, CursorShape, Decorations, Diagnostic, Divider, Fit, GreetingShow, OptionAsAlt, Osc52,
    PRESETS, Rgb, TabPosition, TabShow, TabStyle, Udp, themes,
};

mod sections;

type Key<'i> = Spanned<DeString<'i>>;
type Value<'i> = Spanned<DeValue<'i>>;

const SECTIONS: [&str; 15] = [
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
    "ai",
    "ssh",
];
const QUICK: [&str; 3] = ["key", "height", "hide_on_blur"];
const SSH: [&str; 1] = ["udp"];
const UDP: [(&str, Udp); 3] = [("auto", Udp::Auto), ("on", Udp::On), ("off", Udp::Off)];
const AI: [&str; 5] = ["enabled", "url", "model", "key_env", "send"];
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
const RULE: [&str; 7] = ["host", "dir", "branch", "theme", "accent", "background", "label"];
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
    let sections: [(&str, &[&str]); 15] = [
        ("ssh", &SSH),
        ("keys", &["like"]),
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
        ("ai", &AI),
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
                "ai" => self.ai(section, config),
                "ssh" => self.ssh(section, config),
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
        d = match suggest(&text.to_ascii_lowercase(), &names) {
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
    fn every_fix_at_once() {
        let src = "[cursor]\nshape = \"bem\"\n[tabs]\nposition = \"bottm\"\n";
        let (_, diagnostics) = document(src);
        let fixed = crate::fix_all(src, &diagnostics).unwrap();
        assert_eq!(fixed, "[cursor]\nshape = \"beam\"\n[tabs]\nposition = \"bottom\"\n");
        assert!(document(&fixed).1.is_empty());
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
    fn keys_like_another_terminal_lie_under_your_own() {
        let src = "[keys]\nlike = \"kitty\"\n\"ctrl+shift+x\" = \"zoom\"\n";
        let (config, diagnostics) = document(src);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(config.keys_like, Some("kitty"));
        let bindings = config.bindings();
        if cfg!(not(target_os = "macos")) {
            assert!(
                bindings
                    .iter()
                    .any(|(c, a)| c == "ctrl+shift+z" && a == "prompt_previous")
            );
        }
        assert!(bindings.iter().any(|(c, a)| c == "ctrl+shift+x" && a == "zoom"));
        let (_, diagnostics) = document("[keys]\nlike = \"Kity\"\n");
        assert_eq!(diagnostics[0].hint.as_deref(), Some("did you mean “kitty”?"));
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
