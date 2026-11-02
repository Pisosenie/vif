//! Themes: the whole look in one TOML file, the way a shell framework swaps prompts. vif.toml
//! picks one with `theme = "name"`; a theme may extend another, and vif.toml overrides any of it.

use std::fs;
use std::path::PathBuf;

use toml::de::{DeTable, DeValue};

use crate::{BadgeShow, Config, CursorShape, Diagnostic, Divider, Editor, TabPosition, TabShow, TabStyle, read};

pub const DEFAULT: &str = "night";

const BUILT_IN: [(&str, &str); 9] = [
    ("night", include_str!("../../../assets/themes/night.toml")),
    ("day", include_str!("../../../assets/themes/day.toml")),
    ("dracula", include_str!("../../../assets/themes/dracula.toml")),
    ("catppuccin", include_str!("../../../assets/themes/catppuccin.toml")),
    ("gruvbox", include_str!("../../../assets/themes/gruvbox.toml")),
    ("nord", include_str!("../../../assets/themes/nord.toml")),
    ("phosphor", include_str!("../../../assets/themes/phosphor.toml")),
    ("paper", include_str!("../../../assets/themes/paper.toml")),
    ("alarm", include_str!("../../../assets/themes/alarm.toml")),
];
const DEPTH: usize = 4;

#[derive(Clone, Debug, PartialEq)]
pub struct ThemeFile {
    /// What vif.toml calls it: the file name without `.toml`.
    pub name: String,
    pub title: String,
    pub author: String,
    pub extends: Option<String>,
    /// None for the ones built into vif.
    pub path: Option<PathBuf>,
    pub source: String,
}

impl ThemeFile {
    fn read(name: &str, source: String, path: Option<PathBuf>) -> Self {
        let (doc, _) = DeTable::parse_recoverable(&source);
        let text = |key: &str| match doc.get_ref().get(key).map(|v| v.get_ref()) {
            Some(DeValue::String(s)) => Some(s.to_string()),
            _ => None,
        };
        Self {
            name: name.to_owned(),
            title: text("name").unwrap_or_else(|| name.to_owned()),
            author: text("author").unwrap_or_default(),
            extends: text("extends"),
            path,
            source,
        }
    }
}

pub fn dir() -> PathBuf {
    crate::dir().join("themes")
}

/// Every theme: the built-in ones, then the user's, which win on a name clash.
pub fn all() -> Vec<ThemeFile> {
    let mut themes: Vec<ThemeFile> = BUILT_IN
        .iter()
        .map(|(name, src)| ThemeFile::read(name, (*src).to_owned(), None))
        .collect();
    let mut own: Vec<PathBuf> = fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    own.sort();
    for path in own {
        let (Some(name), Ok(src)) = (path.file_stem().and_then(|s| s.to_str()), fs::read_to_string(&path)) else {
            continue;
        };
        let theme = ThemeFile::read(name, src, Some(path.clone()));
        match themes.iter_mut().find(|t| t.name == theme.name) {
            Some(slot) => *slot = theme,
            None => themes.push(theme),
        }
    }
    themes
}

pub fn find(name: &str) -> Option<ThemeFile> {
    all().into_iter().find(|t| t.name == name)
}

/// Lays a theme and the ones it extends onto `config`; problems come back as sentences.
pub(crate) fn lay(name: &str, config: &mut Config) -> Vec<String> {
    let mut problems = Vec::new();
    apply(name, config, 0, &mut problems);
    config.colors.settle();
    problems
}

fn apply(name: &str, config: &mut Config, depth: usize, problems: &mut Vec<String>) {
    let Some(theme) = find(name) else {
        problems.push(format!("there is no theme “{name}”"));
        return;
    };
    if let Some(base) = theme.extends.as_deref() {
        if depth >= DEPTH || base == name {
            problems.push(format!("{name}: extends goes round in circles"));
        } else {
            apply(base, config, depth + 1, problems);
        }
    }
    for d in read::apply(&theme.source, config, true) {
        let hint = d.hint.map(|h| format!(", {h}")).unwrap_or_default();
        problems.push(format!("{}.toml line {}: {}{hint}", theme.name, d.line, d.message));
    }
}

/// A theme's own problems, against its own text.
pub fn check(name: &str) -> Option<(ThemeFile, Vec<Diagnostic>)> {
    let theme = find(name)?;
    let diagnostics = read::apply(&theme.source, &mut Config::default(), true);
    Some((theme, diagnostics))
}

/// Starts a theme of your own from another one: every key written out with that theme's
/// values and the night theme's notes, so all there is to change is in front of you.
pub fn create(name: &str, from: &str) -> Result<PathBuf, String> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(format!(
            "“{name}” won't do as a file name: use letters, digits, - and _"
        ));
    }
    if find(from).is_none() {
        return Err(format!("there is no theme “{from}”"));
    }
    let path = dir().join(format!("{name}.toml"));
    if path.exists() {
        return Err(format!("{} is there already", path.display()));
    }
    let (c, _) = Config::parse(&format!("theme = \"{from}\"\n"));
    let mut e = Editor::parse(BUILT_IN[0].1)?;
    e.set_top("name", name);
    e.remove_top("author");
    let p = c.colors.base;
    for (key, color) in [
        ("foreground", p.foreground),
        ("background", p.background),
        ("cursor", p.cursor),
        ("selection", p.selection),
        ("accent", p.accent),
    ] {
        e.set_color("colors", key, Some(color));
    }
    e.set_palette(Some(&p.ansi));
    e.set_padding(c.window.padding[0], c.window.padding[1]);
    e.set_float("window", "opacity", c.window.opacity);
    e.set("window", "blur", i64::from(c.window.blur));
    e.set_float("window", "radius", c.window.radius);
    let shape = match c.cursor.shape {
        CursorShape::Block => "block",
        CursorShape::Beam => "beam",
        CursorShape::Underline => "underline",
    };
    e.set("cursor", "shape", shape);
    e.set("cursor", "blink", c.cursor.blink);
    let position = match c.tabs.position {
        TabPosition::Top => "top",
        TabPosition::Bottom => "bottom",
    };
    let style = match c.tabs.style {
        TabStyle::Pills => "pills",
        TabStyle::Underline => "underline",
        TabStyle::Powerline => "powerline",
        TabStyle::Plain => "plain",
    };
    let show = match c.tabs.show {
        TabShow::Many => "many",
        TabShow::Always => "always",
        TabShow::Never => "never",
    };
    e.set("tabs", "position", position);
    e.set("tabs", "style", style);
    e.set("tabs", "show", show);
    e.set("tabs", "title", c.tabs.title.as_str());
    let divider = match c.panes.divider {
        Divider::Line => "line",
        Divider::Gap => "gap",
    };
    e.set("panes", "divider", divider);
    e.set_float("panes", "dim", c.panes.dim);
    let badges = match c.badges.show {
        BadgeShow::All => "all",
        BadgeShow::Failures => "failures",
        BadgeShow::None => "none",
    };
    e.set("badges", "show", badges);
    e.set("badges", "ok", c.badges.ok.as_str());
    e.set("badges", "failed", c.badges.failed.as_str());
    e.set("badges", "running", c.badges.running.as_str());
    e.set("hints", "alphabet", c.hints.alphabet.as_str());
    if c.font.family != crate::DEFAULT_FAMILY {
        e.set("font", "family", c.font.family.as_str());
    }
    fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    fs::write(&path, e.text()).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Copies a theme file into the themes folder, under its file name.
/// A theme file from `text`: as it is when it's one of vif's, converted when it's another
/// terminal's colors.
pub fn adopt(text: &str, title: &str) -> String {
    if read::apply(text, &mut Config::default(), true).is_empty() {
        return text.to_owned();
    }
    crate::import::read(text).map_or_else(|| text.to_owned(), |scheme| scheme.theme(title))
}

pub fn install(text: &str, name: &str) -> Result<(PathBuf, Vec<Diagnostic>), String> {
    let diagnostics = read::apply(text, &mut Config::default(), true);
    let path = dir().join(format!("{name}.toml"));
    fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok((path, diagnostics))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_themes_read_clean() {
        for (name, _) in BUILT_IN {
            let (theme, diagnostics) = check(name).unwrap();
            assert!(diagnostics.is_empty(), "{name}: {diagnostics:?}");
            assert!(!theme.title.is_empty());
            let mut config = Config::default();
            assert!(lay(name, &mut config).is_empty(), "{name}");
        }
    }

    #[test]
    fn a_theme_lays_under_the_config() {
        let (config, diagnostics) = Config::parse("theme = \"gruvbox\"\n[colors]\naccent = \"#ff0000\"\n");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(config.tabs.style, crate::TabStyle::Powerline);
        assert_eq!(config.window.radius, 0.0);
        let palette = config.palette();
        assert_eq!(palette.background, crate::Rgb::hex(0x282828));
        assert_eq!(palette.accent, crate::Rgb::hex(0xff0000));
        // Apolline's accent beats the theme's, not yours.
        let (plain, _) = Config::parse("theme = \"gruvbox\"\n");
        assert_eq!(
            plain.palette_with(Some(crate::Rgb::hex(0x00ff00))).accent,
            crate::Rgb::hex(0x00ff00)
        );
    }

    #[test]
    fn your_own_theme_starts_as_the_one_it_came_from() {
        let home = std::env::temp_dir().join(format!("vif-themes-{}", std::process::id()));
        // SAFETY: the only test that reads the config home.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &home) };
        let path = create("mine", "gruvbox").unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("style = \"powerline\"") && text.contains("# extends"),
            "{text}"
        );
        assert!(!text.contains("author"));
        let (own, problems) = Config::parse("theme = \"mine\"\n");
        let (original, _) = Config::parse("theme = \"gruvbox\"\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(own.palette(), original.palette());
        assert_eq!(own.tabs, original.tabs);
        assert!(create("mine", "night").is_err());
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn a_wrong_theme_name_is_fixed() {
        let src = "theme = \"drakula\"\n";
        let (config, diagnostics) = Config::parse(src);
        assert_eq!(config.theme, DEFAULT);
        let fixed = diagnostics[0].fix.as_ref().unwrap().apply(src).unwrap();
        assert_eq!(fixed, "theme = \"dracula\"\n");
    }
}
