#![warn(clippy::undocumented_unsafe_blocks)]

mod diagnostic;
mod edit;
pub mod import;
mod read;
mod theme;
pub mod themes;

use std::path::PathBuf;
use std::{env, fs, io};

pub use diagnostic::{Diagnostic, Fix, fix_all, report};
pub use edit::{Editor, setting_changed};
pub use read::known;
pub use theme::{Palette, Rgb};

pub const ACTIONS: [&str; 48] = [
    "copy",
    "paste",
    "search",
    "hints",
    "prompt_previous",
    "prompt_next",
    "copy_output",
    "palette",
    "sessions",
    "hud",
    "quick_terminal",
    "history",
    "new_window",
    "new_tab",
    "close",
    "split_right",
    "split_down",
    "zoom",
    "resize_left",
    "resize_right",
    "resize_up",
    "resize_down",
    "ai_command",
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
    pub background: Background,
    pub quick: Quick,
    pub greeting: Greeting,
    pub ai: Ai,
}

/// A language model for explaining failed commands and turning words into one. Off until it's
/// turned on; nothing leaves the machine unless `url` points elsewhere.
#[derive(Clone, Debug, PartialEq)]
pub struct Ai {
    pub enabled: bool,
    /// An OpenAI-compatible API: Ollama's by default, or any other.
    pub url: String,
    pub model: String,
    /// The environment variable holding the API key, if the API wants one. The key itself is
    /// never written in vif.toml.
    pub key_env: String,
    /// The most characters of a command's output one question sends.
    pub send: u32,
}

/// A picture played in text above the first prompt, the way people run neofetch.
#[derive(Clone, Debug, PartialEq)]
pub struct Greeting {
    pub show: GreetingShow,
    /// A GIF, PNG or JPEG, found the way `background.image` is.
    pub file: Option<String>,
    /// ▀ halves in color rather than characters.
    pub blocks: bool,
    /// Who, where and on what, beside the picture.
    pub info: bool,
    /// Lines the picture may take.
    pub height: u16,
    /// Times a GIF plays through before the prompt.
    pub loops: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GreetingShow {
    Off,
    /// The first shell of each window.
    Window,
    /// Every new shell, tabs and splits too.
    Always,
}

/// The quick terminal, which drops in from the top of the screen.
#[derive(Clone, Debug, PartialEq)]
pub struct Quick {
    /// The key that shows and hides it from anywhere, on macOS; elsewhere the compositor binds
    /// `vif quick`.
    pub key: String,
    /// How much of the screen's height it takes.
    pub height: f32,
    /// Hides when another window takes the focus.
    pub hide_on_blur: bool,
}

/// A picture behind the text.
#[derive(Clone, Debug, PartialEq)]
pub struct Background {
    /// As written: `~/` is home, a bare name sits by vif.toml.
    pub image: Option<String>,
    pub fit: Fit,
    /// How far the picture fades toward the background color, 0 to 1.
    pub dim: f32,
    /// Blur radius in pixels.
    pub blur: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    Cover,
    Contain,
    Stretch,
    Center,
    Tile,
}

/// A file named in the config: `~/` is home, an absolute path is itself, anything else sits in
/// `dir` next to vif.toml.
pub fn file(name: &str, dir: &str) -> PathBuf {
    match name.strip_prefix("~/") {
        Some(rest) => env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(rest),
        None => path()
            .parent()
            .map_or_else(self::dir, PathBuf::from)
            .join(dir)
            .join(name),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    pub family: String,
    pub size: f32,
    pub line_height: f32,
    pub ligatures: bool,
    pub weight: u16,
    pub bold_weight: u16,
    pub bold_family: Option<String>,
    pub italic_family: Option<String>,
    /// OpenType features as written: "ss01", "-calt", "cv02=3".
    pub features: Vec<String>,
}

impl Font {
    /// The features as tags and values: "-calt" is ("calt", 0), "cv02=3" is ("cv02", 3).
    pub fn feature_settings(&self) -> Vec<(String, u16)> {
        self.features.iter().filter_map(|f| feature(f)).collect()
    }
}

pub(crate) fn feature(text: &str) -> Option<(String, u16)> {
    let (tag, value) = match text.split_once('=') {
        Some((tag, value)) => (tag, value.trim().parse().ok()?),
        None => match text.strip_prefix('-') {
            Some(tag) => (tag, 0),
            None => (text.strip_prefix('+').unwrap_or(text), 1),
        },
    };
    let tag = tag.trim();
    (tag.len() == 4 && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ')).then(|| (tag.to_owned(), value))
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
    /// A color scheme file that colors follow as it changes, like pywal's: as written.
    pub from: Option<String>,
    /// What that file held when last read.
    pub scheme: Option<import::Scheme>,
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
    /// Shaders over the whole window, in order: built-in names or files.
    pub shaders: Vec<String>,
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
    /// A streak where the cursor jumps from.
    pub trail: bool,
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
    /// A quieter second part after the title: what runs there, or its folder.
    pub detail: bool,
    /// A "+" after the last tab.
    pub plus: bool,
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
    /// The time of day a command ended, after how long it took.
    pub clock: bool,
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
                weight: 400,
                bold_weight: 700,
                bold_family: None,
                italic_family: None,
                features: Vec::new(),
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
                from: None,
                scheme: None,
            },
            window: Window {
                padding: [12, 10],
                opacity: 1.0,
                blur: 0,
                columns: 100,
                lines: 30,
                decorations: Decorations::Transparent,
                radius: 8.0,
                shaders: Vec::new(),
            },
            cursor: Cursor {
                shape: CursorShape::Block,
                blink: false,
                trail: false,
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
                title: "{dir}".to_owned(),
                detail: true,
                plus: true,
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
                clock: true,
            },
            hints: HintKeys {
                alphabet: "asdfghjklqwertyuiopzxcvbnm".to_owned(),
            },
            rules: Vec::new(),
            background: Background {
                image: None,
                fit: Fit::Cover,
                dim: 0.0,
                blur: 0,
            },
            quick: Quick {
                key: String::new(),
                height: 0.45,
                hide_on_blur: true,
            },
            ai: Ai {
                enabled: false,
                url: "http://localhost:11434/v1".to_owned(),
                model: "qwen2.5-coder:7b".to_owned(),
                key_env: String::new(),
                send: 8000,
            },
            greeting: Greeting {
                show: GreetingShow::Off,
                file: None,
                blocks: true,
                info: true,
                height: 12,
                loops: 1,
            },
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

/// The file `[colors] from` names: "wal" is pywal's, `~/` is home, anything else sits by vif.toml.
pub fn scheme_path(from: &str) -> PathBuf {
    let home = env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    if from == "wal" {
        let cache = env::var_os("XDG_CACHE_HOME")
            .filter(|v| !v.is_empty())
            .map_or_else(|| home.join(".cache"), PathBuf::from);
        return cache.join("wal").join("colors.json");
    }
    match from.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => dir().join(from),
    }
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
            ("cmd+shift+s", "sessions"),
            ("cmd+shift+f", "history"),
            ("cmd+n", "new_window"),
            ("cmd+t", "new_tab"),
            ("cmd+w", "close"),
            ("cmd+d", "split_right"),
            ("cmd+shift+d", "split_down"),
            ("cmd+shift+enter", "zoom"),
            ("cmd+ctrl+left", "resize_left"),
            ("cmd+ctrl+right", "resize_right"),
            ("cmd+ctrl+up", "resize_up"),
            ("cmd+ctrl+down", "resize_down"),
            ("cmd+i", "ai_command"),
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
            ("ctrl+shift+s", "sessions"),
            ("ctrl+alt+f", "history"),
            ("ctrl+shift+n", "new_window"),
            ("ctrl+shift+t", "new_tab"),
            ("ctrl+shift+w", "close"),
            ("ctrl+shift+d", "split_right"),
            ("ctrl+shift+e", "split_down"),
            ("ctrl+shift+z", "zoom"),
            ("alt+ctrl+shift+left", "resize_left"),
            ("alt+ctrl+shift+right", "resize_right"),
            ("alt+ctrl+shift+up", "resize_up"),
            ("alt+ctrl+shift+down", "resize_down"),
            ("ctrl+shift+i", "ai_command"),
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
