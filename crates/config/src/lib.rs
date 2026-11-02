mod diagnostic;
mod edit;
mod read;
mod theme;
pub mod themes;

use std::path::PathBuf;
use std::{env, fs, io};

pub use diagnostic::{Diagnostic, Fix, report};
pub use edit::Editor;
pub use theme::{Palette, Rgb};

pub const ACTIONS: [&str; 39] = [
    "copy",
    "paste",
    "search",
    "hints",
    "prompt_previous",
    "prompt_next",
    "copy_output",
    "palette",
    "history",
    "new_window",
    "new_tab",
    "close",
    "split_right",
    "split_down",
    "focus_next",
    "focus_previous",
    "focus_left",
    "focus_right",
    "focus_up",
    "focus_down",
    "next_tab",
    "previous_tab",
    "tab_1",
    "tab_2",
    "tab_3",
    "tab_4",
    "tab_5",
    "tab_6",
    "tab_7",
    "tab_8",
    "tab_9",
    "font_bigger",
    "font_smaller",
    "font_reset",
    "scroll_page_up",
    "scroll_page_down",
    "scroll_top",
    "scroll_bottom",
    "settings",
];

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// The theme everything else is laid over.
    pub theme: String,
    pub font: Font,
    pub colors: Colors,
    pub window: Window,
    pub cursor: Cursor,
    pub terminal: Terminal,
    pub keys: Vec<(String, String)>,
    pub tabs: Tabs,
    pub panes: Panes,
    pub badges: Badges,
    pub hints: HintKeys,
    pub rules: Vec<Rule>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    pub family: String,
    pub size: f32,
    pub line_height: f32,
    pub ligatures: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Colors {
    /// What the theme settled on; the keys below and an outside accent lay over it.
    pub base: Palette,
    pub foreground: Option<Rgb>,
    pub background: Option<Rgb>,
    pub cursor: Option<Rgb>,
    pub selection: Option<Rgb>,
    pub accent: Option<Rgb>,
    pub palette: [Option<Rgb>; 16],
    /// Take the accent from Apolline while it runs.
    pub apolline: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub padding: [u16; 2],
    pub opacity: f32,
    pub blur: u8,
    pub columns: u16,
    pub lines: u16,
    pub decorations: Decorations,
    /// Corners of tabs, panels and hint labels, in points.
    pub radius: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decorations {
    Full,
    Transparent,
    None,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cursor {
    pub shape: CursorShape,
    pub blink: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    Block,
    Beam,
    Underline,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Terminal {
    pub shell: Option<String>,
    pub args: Vec<String>,
    pub scrollback: u32,
    pub term: String,
    pub osc52: Osc52,
    pub kitty_keyboard: bool,
    pub option_as_alt: OptionAsAlt,
    pub shell_integration: bool,
    /// Seconds a command has to run before its end is worth a notification; 0 never notifies.
    pub notify_after: u32,
    /// Days to keep each command and its output for `vif history`; 0 keeps nothing new.
    pub archive_days: u32,
    /// Shells live on when their window closes, and come back with the next one.
    pub sessions: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Osc52 {
    Off,
    Copy,
    Paste,
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionAsAlt {
    None,
    Left,
    Right,
    Both,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tabs {
    pub position: TabPosition,
    pub style: TabStyle,
    pub show: TabShow,
    /// What a tab says: {index}, {title}, {cwd} and {dir} are filled in.
    pub title: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabPosition {
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabStyle {
    Pills,
    Underline,
    Powerline,
    Plain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabShow {
    Many,
    Always,
    Never,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Panes {
    pub divider: Divider,
    /// How far splits without focus fade toward the background.
    pub dim: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Divider {
    Line,
    Gap,
}

/// The exit status and time shown at the end of a command line.
#[derive(Clone, Debug, PartialEq)]
pub struct Badges {
    pub show: BadgeShow,
    pub ok: String,
    pub failed: String,
    pub running: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeShow {
    All,
    Failures,
    None,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HintKeys {
    /// Letters for hint labels, the easiest first.
    pub alphabet: String,
}

/// A look for panes that are somewhere: ssh'd into a host, or in a folder.
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    /// A host name pattern, like "prod-*", matched against what `ssh` connects to.
    pub host: Option<String>,
    /// A folder pattern, like "~/work/infra*"; folders inside it match too.
    pub dir: Option<String>,
    pub theme: Option<String>,
    pub accent: Option<Rgb>,
    pub background: Option<Rgb>,
    /// Shown in the pane's corner, like "PROD".
    pub label: Option<String>,
}

pub const DEFAULT_FAMILY: &str = "Geist Mono";

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: themes::DEFAULT.to_owned(),
            font: Font {
                family: DEFAULT_FAMILY.to_owned(),
                size: 13.0,
                line_height: 1.2,
                ligatures: true,
            },
            colors: Colors {
                base: Palette::NIGHT,
                foreground: None,
                background: None,
                cursor: None,
                selection: None,
                accent: None,
                palette: [None; 16],
                apolline: true,
            },
            window: Window {
                padding: [12, 10],
                opacity: 1.0,
                blur: 0,
                columns: 100,
                lines: 30,
                decorations: Decorations::Transparent,
                radius: 8.0,
            },
            cursor: Cursor {
                shape: CursorShape::Block,
                blink: false,
            },
            terminal: Terminal {
                shell: None,
                args: Vec::new(),
                scrollback: 10_000,
                term: "xterm-vif".to_owned(),
                osc52: Osc52::Copy,
                kitty_keyboard: true,
                option_as_alt: OptionAsAlt::None,
                shell_integration: true,
                notify_after: 10,
                archive_days: 30,
                sessions: true,
            },
            keys: Vec::new(),
            tabs: Tabs {
                position: TabPosition::Top,
                style: TabStyle::Pills,
                show: TabShow::Many,
                title: "{index} {title}".to_owned(),
            },
            panes: Panes {
                divider: Divider::Line,
                dim: 0.28,
            },
            badges: Badges {
                show: BadgeShow::All,
                ok: "✓".to_owned(),
                failed: "✗".to_owned(),
                running: "●".to_owned(),
            },
            hints: HintKeys {
                alphabet: "asdfghjklqwertyuiopzxcvbnm".to_owned(),
            },
            rules: Vec::new(),
        }
    }
}

impl Config {
    pub fn parse(src: &str) -> (Self, Vec<Diagnostic>) {
        read::document(src)
    }

    pub fn load() -> (Self, Vec<Diagnostic>) {
        match fs::read_to_string(path()) {
            Ok(src) => Self::parse(&src),
            Err(err) if err.kind() == io::ErrorKind::NotFound => (Self::default(), Vec::new()),
            Err(err) => (
                Self::default(),
                vec![Diagnostic::new(0, format!("can't read the config: {err}"))],
            ),
        }
    }

    pub fn palette(&self) -> Palette {
        self.colors.resolve(None)
    }

    /// The palette with an accent from outside, like Apolline's, where the config allows it.
    pub fn palette_with(&self, outside: Option<Rgb>) -> Palette {
        self.colors.resolve(outside.filter(|_| self.colors.apolline))
    }

    /// User bindings on top of the defaults; "none" unbinds.
    pub fn bindings(&self) -> Vec<(String, String)> {
        let mut all: Vec<(String, String)> = default_keys()
            .iter()
            .map(|(combo, action)| (normalize_combo(combo), (*action).to_owned()))
            .collect();
        for (combo, action) in &self.keys {
            let combo = normalize_combo(combo);
            all.retain(|(c, _)| *c != combo);
            if action != "none" {
                all.push((combo, action.clone()));
            }
        }
        all
    }
}

fn config_home() -> PathBuf {
    env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn dir() -> PathBuf {
    config_home().join("vif")
}

/// Where Apolline writes its accent for other apps; it is gone when Apolline doesn't color them.
pub fn apolline_css() -> PathBuf {
    config_home().join("gtk-4.0").join("apolline.css")
}

pub fn apolline_accent() -> Option<Rgb> {
    let css = fs::read_to_string(apolline_css()).ok()?;
    css.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("@define-color accent_color")?;
        Rgb::parse(rest.trim().trim_end_matches(';').trim())
    })
}

/// The known name closest to a misspelled one, if any is close.
pub fn closest<'a>(input: &str, known: &[&'a str]) -> Option<&'a str> {
    read::suggest(input, known)
}

pub fn path() -> PathBuf {
    env::var_os("VIF_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| dir().join("vif.toml"))
}

pub fn normalize_combo(combo: &str) -> String {
    let mut mods = Vec::new();
    let mut key = String::new();
    for part in combo.split('+').filter(|p| !p.is_empty()) {
        let part = part.trim().to_ascii_lowercase();
        match part.as_str() {
            "cmd" | "super" | "command" | "logo" | "mod" => mods.push("cmd"),
            "ctrl" | "control" => mods.push("ctrl"),
            "alt" | "option" | "opt" => mods.push("alt"),
            "shift" => mods.push("shift"),
            _ => key = part,
        }
    }
    if combo.ends_with("++") || combo == "+" {
        key = "+".to_owned();
    }
    mods.sort_unstable();
    mods.dedup();
    let mut out = mods.join("+");
    if !out.is_empty() {
        out.push('+');
    }
    out.push_str(&key);
    out
}

pub fn default_keys() -> &'static [(&'static str, &'static str)] {
    if cfg!(target_os = "macos") {
        &[
            ("cmd+c", "copy"),
            ("cmd+v", "paste"),
            ("cmd+f", "search"),
            ("cmd+shift+h", "hints"),
            ("cmd+up", "prompt_previous"),
            ("cmd+down", "prompt_next"),
            ("cmd+shift+c", "copy_output"),
            ("cmd+shift+p", "palette"),
            ("cmd+shift+f", "history"),
            ("cmd+n", "new_window"),
            ("cmd+t", "new_tab"),
            ("cmd+w", "close"),
            ("cmd+d", "split_right"),
            ("cmd+shift+d", "split_down"),
            ("cmd+]", "focus_next"),
            ("cmd+[", "focus_previous"),
            ("cmd+alt+left", "focus_left"),
            ("cmd+alt+right", "focus_right"),
            ("cmd+alt+up", "focus_up"),
            ("cmd+alt+down", "focus_down"),
            ("cmd+shift+]", "next_tab"),
            ("cmd+shift+[", "previous_tab"),
            ("cmd+1", "tab_1"),
            ("cmd+2", "tab_2"),
            ("cmd+3", "tab_3"),
            ("cmd+4", "tab_4"),
            ("cmd+5", "tab_5"),
            ("cmd+6", "tab_6"),
            ("cmd+7", "tab_7"),
            ("cmd+8", "tab_8"),
            ("cmd+9", "tab_9"),
            ("cmd+=", "font_bigger"),
            ("cmd+-", "font_smaller"),
            ("cmd+0", "font_reset"),
            ("shift+pageup", "scroll_page_up"),
            ("shift+pagedown", "scroll_page_down"),
            ("cmd+home", "scroll_top"),
            ("cmd+end", "scroll_bottom"),
            ("cmd+,", "settings"),
        ]
    } else {
        &[
            ("ctrl+shift+c", "copy"),
            ("ctrl+shift+v", "paste"),
            ("ctrl+shift+f", "search"),
            ("ctrl+shift+h", "hints"),
            ("ctrl+shift+up", "prompt_previous"),
            ("ctrl+shift+down", "prompt_next"),
            ("ctrl+alt+c", "copy_output"),
            ("ctrl+shift+p", "palette"),
            ("ctrl+alt+f", "history"),
            ("ctrl+shift+n", "new_window"),
            ("ctrl+shift+t", "new_tab"),
            ("ctrl+shift+w", "close"),
            ("ctrl+shift+d", "split_right"),
            ("ctrl+shift+e", "split_down"),
            ("ctrl+shift+]", "focus_next"),
            ("ctrl+shift+[", "focus_previous"),
            ("ctrl+alt+left", "focus_left"),
            ("ctrl+alt+right", "focus_right"),
            ("ctrl+alt+up", "focus_up"),
            ("ctrl+alt+down", "focus_down"),
            ("ctrl+tab", "next_tab"),
            ("ctrl+shift+tab", "previous_tab"),
            ("alt+1", "tab_1"),
            ("alt+2", "tab_2"),
            ("alt+3", "tab_3"),
            ("alt+4", "tab_4"),
            ("alt+5", "tab_5"),
            ("alt+6", "tab_6"),
            ("alt+7", "tab_7"),
            ("alt+8", "tab_8"),
            ("alt+9", "tab_9"),
            ("ctrl+=", "font_bigger"),
            ("ctrl+-", "font_smaller"),
            ("ctrl+0", "font_reset"),
            ("shift+pageup", "scroll_page_up"),
            ("shift+pagedown", "scroll_page_down"),
            ("ctrl+shift+home", "scroll_top"),
            ("ctrl+shift+end", "scroll_bottom"),
            ("ctrl+,", "settings"),
        ]
    }
}
