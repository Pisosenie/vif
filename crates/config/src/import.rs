//! Colors from other terminals and wallpaper tools: kitty, Ghostty, Alacritty, base16, iTerm,
//! Xresources, Windows Terminal and pywal, told apart by what they say rather than their names.

use crate::Rgb;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scheme {
    pub foreground: Option<Rgb>,
    pub background: Option<Rgb>,
    pub cursor: Option<Rgb>,
    pub selection: Option<Rgb>,
    pub palette: [Option<Rgb>; 16],
}

impl Scheme {
    fn set(&mut self, slot: Slot, color: Rgb) {
        match slot {
            Slot::Foreground => self.foreground = Some(color),
            Slot::Background => self.background = Some(color),
            Slot::Cursor => self.cursor = Some(color),
            Slot::Selection => self.selection = Some(color),
            Slot::Ansi(i) => self.palette[i] = Some(color),
        }
    }

    /// `base` with these colors laid over it, as a theme would; the accent follows the blue.
    pub fn over(&self, base: crate::Palette) -> crate::Palette {
        let mut palette = base;
        for (i, slot) in palette.ansi.iter_mut().enumerate() {
            if let Some(c) = self.palette[i].or(if i >= 8 { self.palette[i - 8] } else { None }) {
                *slot = c;
            }
        }
        palette.foreground = self.foreground.unwrap_or(palette.foreground);
        palette.background = self.background.unwrap_or(palette.background);
        palette.accent = self.palette[4].or(self.cursor).unwrap_or(palette.accent);
        palette.cursor = self.cursor.unwrap_or(palette.accent);
        palette.selection = self
            .selection
            .unwrap_or_else(|| palette.foreground.toward(palette.background, 0.75));
        palette
    }

    fn colors(&self) -> usize {
        self.palette.iter().flatten().count()
            + usize::from(self.foreground.is_some())
            + usize::from(self.background.is_some())
    }

    /// As a theme file that keeps night's look and takes these colors.
    pub fn theme(&self, title: &str) -> String {
        let mut out = format!("name = \"{title}\"\nextends = \"night\"\n\n[colors]\n");
        for (key, color) in [
            ("foreground", self.foreground),
            ("background", self.background),
            ("cursor", self.cursor),
            ("selection", self.selection),
            ("accent", self.palette[4].or(self.cursor)),
        ] {
            if let Some(c) = color {
                out.push_str(&format!("{key} = \"{}\"\n", c.to_hex()));
            }
        }
        let mut palette = self.palette;
        for i in 8..16 {
            palette[i] = palette[i].or(palette[i - 8]);
        }
        let known: Vec<Rgb> = palette.iter().map_while(|c| *c).collect();
        if !known.is_empty() {
            out.push_str("palette = [\n");
            for row in known.chunks(4) {
                let row: Vec<String> = row.iter().map(|c| format!("\"{}\"", c.to_hex())).collect();
                out.push_str(&format!("  {},\n", row.join(", ")));
            }
            out.push_str("]\n");
        }
        out
    }
}

#[derive(Clone, Copy)]
enum Slot {
    Foreground,
    Background,
    Cursor,
    Selection,
    Ansi(usize),
}

const NAMES: [&str; 8] = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];

/// base16's slots for ANSI 0–15, the way base16-shell lays them out.
const BASE16: [usize; 16] = [
    0x0, 0x8, 0xB, 0xA, 0xD, 0xE, 0xC, 0x5, 0x3, 0x8, 0xB, 0xA, 0xD, 0xE, 0xC, 0x7,
];

/// Reads a color scheme in any of the formats above; None when it finds too little to be one.
pub fn read(text: &str) -> Option<Scheme> {
    let scheme = if text.contains("<plist") {
        plist(text)
    } else {
        pairs(text)
    };
    let mut scheme = scheme;
    // A selection as light as the text is drawn inverted where it comes from; vif keeps the text's
    // color, so it takes a quieter one.
    if let (Some(sel), Some(fg), Some(bg)) = (scheme.selection, scheme.foreground, scheme.background)
        && (sel.luma() - fg.luma()).abs() < 0.25
    {
        scheme.selection = Some(sel.toward(bg, 0.65));
    }
    (scheme.colors() >= 8).then_some(scheme)
}

/// Lines of `key value`, `key = value` or `key: value`, under TOML headers, YAML indents or JSON objects.
fn pairs(text: &str) -> Scheme {
    let json = text.trim_start().starts_with('{');
    let text = if json {
        text.replace(['{', ','], "\n").replace('}', "\n}\n")
    } else {
        text.to_owned()
    };
    let mut scheme = Scheme::default();
    let mut base = [None; 24];
    let mut sections: Vec<(usize, String)> = Vec::new();
    let mut header = String::new();
    let mut array: Option<(usize, String)> = None;
    for raw in text.lines() {
        if let Some((offset, mut items)) = array.take() {
            items.push_str(raw);
            match items.find(']') {
                Some(end) => list(&mut scheme, offset, &items[..end]),
                None => array = Some((offset, items)),
            }
            continue;
        }
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('!') || line.starts_with("//") {
            continue;
        }
        if json && line == "}" {
            sections.pop();
            continue;
        }
        if let Some(table) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            header = table.rsplit('.').next().map(key).unwrap_or_default();
            continue;
        }
        let indent = raw.len() - raw.trim_start().len();
        let Some((name, value)) = split(line) else {
            continue;
        };
        let name = key(name);
        if !json {
            sections.retain(|(at, _)| *at < indent);
        }
        if value.is_empty() {
            sections.push((indent, name));
            continue;
        }
        let section = sections.last().map_or(header.as_str(), |(_, s)| s.as_str());
        if let Some(i) = name
            .strip_prefix("base")
            .and_then(|n| usize::from_str_radix(n, 16).ok())
            && name.len() == 6
            && i < 24
        {
            base[i] = color(value);
            continue;
        }
        if let ("ansi" | "brights", Some(items)) = (name.as_str(), value.strip_prefix('[')) {
            let offset = if name == "ansi" { 0 } else { 8 };
            match items.find(']') {
                Some(end) => list(&mut scheme, offset, &items[..end]),
                None => array = Some((offset, items.to_owned())),
            }
            continue;
        }
        if name == "palette"
            && let Some((index, value)) = value.split_once('=')
            && let (Ok(i), Some(c)) = (index.trim().parse::<usize>(), color(value))
            && i < 16
        {
            scheme.palette[i] = Some(c);
            continue;
        }
        if name == "color"
            && let Some(c) = decimal(value)
        {
            // Konsole: `[Color4Intense]` then `Color=98,114,164`.
            let slot = match section.strip_prefix("color") {
                Some(n) => n.strip_suffix("intense").map_or_else(
                    || n.parse::<usize>().ok().filter(|n| *n < 8).map(Slot::Ansi),
                    |n| n.parse::<usize>().ok().filter(|n| *n < 8).map(|n| Slot::Ansi(n + 8)),
                ),
                None => slot("", section),
            };
            if let Some(slot) = slot {
                scheme.set(slot, c);
            }
            continue;
        }
        if let (Some(slot), Some(c)) = (slot(section, &name), color(value)) {
            scheme.set(slot, c);
        }
    }
    if base[0].is_some() && base[5].is_some() {
        for (i, b) in BASE16.iter().enumerate() {
            scheme.palette[i] = scheme.palette[i].or(base[*b]);
        }
        scheme.foreground = scheme.foreground.or(base[5]);
        scheme.background = scheme.background.or(base[0]);
        scheme.cursor = scheme.cursor.or(base[5]);
        scheme.selection = scheme.selection.or(base[2]);
    }
    scheme
}

/// `40,42,54`.
fn decimal(value: &str) -> Option<Rgb> {
    let parts: Vec<u8> = value.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    (parts.len() == 3).then(|| Rgb {
        r: parts[0],
        g: parts[1],
        b: parts[2],
    })
}

/// WezTerm's `ansi = [...]` and `brights = [...]`.
fn list(scheme: &mut Scheme, offset: usize, items: &str) {
    for (i, item) in items.split(',').filter_map(color).take(8).enumerate() {
        scheme.palette[offset + i] = Some(item);
    }
}

fn split(line: &str) -> Option<(&str, &str)> {
    let at = line.find(['=', ':', ' ', '\t'])?;
    let (name, rest) = line.split_at(at);
    let rest = rest.trim_start_matches([' ', '\t']);
    let rest = rest.strip_prefix(['=', ':']).unwrap_or(rest);
    Some((name, rest.trim()))
}

/// A name with its quotes, case, separators and X resource prefixes gone: `*.color4` is `color4`.
fn key(name: &str) -> String {
    let name = name.trim().trim_matches(['"', '\'']);
    let name = name.rsplit(['.', '*']).next().unwrap_or(name);
    name.chars()
        .filter(|c| !matches!(c, '-' | '_' | ' '))
        .flat_map(char::to_lowercase)
        .collect()
}

fn slot(section: &str, name: &str) -> Option<Slot> {
    let name = name.replace("purple", "magenta");
    // VS Code's `terminal.ansiBrightBlack`.
    let name = name.strip_prefix("ansi").unwrap_or(&name);
    let ansi = |offset: usize| NAMES.iter().position(|n| *n == name).map(|i| Slot::Ansi(i + offset));
    match section {
        "selection" => return (name == "background").then_some(Slot::Selection),
        "cursor" => return (name == "cursor").then_some(Slot::Cursor),
        "normal" => return ansi(0),
        "bright" => return ansi(8),
        "dim" | "indexedcolors" | "hints" | "search" | "footerbar" => return None,
        _ => {}
    }
    for (prefix, offset) in [("bright", 8), ("regular", 0)] {
        if let Some(rest) = name.strip_prefix(prefix) {
            let digit = rest.parse::<usize>().ok().filter(|n| *n < 8);
            return digit
                .or_else(|| NAMES.iter().position(|n| *n == rest))
                .map(|i| Slot::Ansi(i + offset));
        }
    }
    if let Some(n) = name.strip_prefix("color").and_then(|n| n.parse::<usize>().ok()) {
        return (n < 16).then_some(Slot::Ansi(n));
    }
    match name {
        "foreground" | "fg" | "foregroundcolor" => Some(Slot::Foreground),
        "background" | "bg" | "backgroundcolor" => Some(Slot::Background),
        "cursor" | "cursorcolor" | "cursorbg" => Some(Slot::Cursor),
        "selectionbackground" | "selectionbg" | "selection" | "selectioncolor" => Some(Slot::Selection),
        _ => ansi(0),
    }
}

/// `#rrggbb`, `rrggbb`, `0xrrggbb`, `#rgb`, `#rrggbbaa` or X's `rgb:rr/gg/bb`, quoted or not.
fn color(value: &str) -> Option<Rgb> {
    let value = value
        .split(" #")
        .next()?
        .trim()
        .trim_end_matches(',')
        .trim_matches(['"', '\'']);
    if let Some(rgb) = value.strip_prefix("rgb:") {
        let parts: Vec<u8> = rgb
            .split('/')
            .filter_map(|p| {
                u16::from_str_radix(p, 16)
                    .ok()
                    .map(|v| if p.len() > 2 { (v >> 8) as u8 } else { v as u8 })
            })
            .collect();
        return (parts.len() == 3).then(|| Rgb {
            r: parts[0],
            g: parts[1],
            b: parts[2],
        });
    }
    let hex = value
        .strip_prefix('#')
        .or_else(|| value.strip_prefix("0x"))
        .unwrap_or(value);
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        3 => {
            let v = u32::from_str_radix(hex, 16).ok()?;
            let spread = |n: u32| ((n & 0xf) * 0x11) as u8;
            Some(Rgb {
                r: spread(v >> 8),
                g: spread(v >> 4),
                b: spread(v),
            })
        }
        6 | 8 => u32::from_str_radix(&hex[..6], 16).ok().map(Rgb::hex),
        _ => None,
    }
}

/// iTerm2's .itermcolors: a plist of named colors with components from 0 to 1.
fn plist(text: &str) -> Scheme {
    let mut scheme = Scheme::default();
    let mut rest = text;
    while let Some(at) = rest.find("<key>") {
        rest = &rest[at + 5..];
        let Some(end) = rest.find("</key>") else {
            break;
        };
        let name = key(&rest[..end]);
        rest = &rest[end..];
        let slot = match name.strip_prefix("ansi").and_then(|n| n.strip_suffix("color")) {
            Some(n) => n.parse::<usize>().ok().filter(|n| *n < 16).map(Slot::Ansi),
            None => match name.as_str() {
                "foregroundcolor" => Some(Slot::Foreground),
                "backgroundcolor" => Some(Slot::Background),
                "cursorcolor" => Some(Slot::Cursor),
                "selectioncolor" => Some(Slot::Selection),
                _ => None,
            },
        };
        let (Some(slot), Some(close)) = (slot, rest.find("</dict>")) else {
            continue;
        };
        let dict = &rest[..close];
        let part = |which: &str| -> Option<u8> {
            let at = dict.find(&format!("<key>{which} Component</key>"))?;
            let after = &dict[at..];
            let start = after.find("<real>")? + 6;
            let end = after.find("</real>")?;
            let v: f32 = after[start..end].trim().parse().ok()?;
            Some((v.clamp(0.0, 1.0) * 255.0).round() as u8)
        };
        if let (Some(r), Some(g), Some(b)) = (part("Red"), part("Green"), part("Blue")) {
            scheme.set(slot, Rgb { r, g, b });
        }
        rest = &rest[close..];
    }
    scheme
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(c: Option<Rgb>) -> String {
        c.map(Rgb::to_hex).unwrap_or_default()
    }

    const ANSI: &str = "color0 #000001\ncolor1 #000002\ncolor2 #000003\ncolor3 #000004\ncolor4 #000005\n\
        color5 #000006\ncolor6 #000007\ncolor7 #000008\ncolor8 #000009\ncolor15 #000010\n";

    #[test]
    fn kitty_and_xresources() {
        let s = read(&format!(
            "# Dracula\nforeground #f8f8f2\nbackground  #282a36\nselection_background #44475a\n{ANSI}"
        ))
        .unwrap();
        assert_eq!(hex(s.foreground), "#f8f8f2");
        assert_eq!(hex(s.selection), "#44475a");
        assert_eq!(hex(s.palette[15]), "#000010");
        let x = read("*.foreground: #ffffff\n*background: rgb:00/00/10\nURxvt*color0: #000001\n*.color1: #000002\n*.color2: #000003\n*.color3: #000004\n*.color4: #000005\n*.color5: #000006\n").unwrap();
        assert_eq!(hex(x.background), "#000010");
        assert_eq!(hex(x.palette[0]), "#000001");
    }

    #[test]
    fn ghostty_alacritty_and_base16() {
        let g = read("background = 282a36\nforeground = f8f8f2\ncursor-color = f8f8f2\nselection-background = 44475a\npalette = 0=#21222c\npalette = 1=#ff5555\npalette = 2=#50fa7b\npalette = 3=#f1fa8c\npalette = 4=#bd93f9\npalette = 12=#d6acff\n").unwrap();
        assert_eq!(hex(g.palette[12]), "#d6acff");
        assert_eq!(hex(g.selection), "#44475a");
        let a = read("[colors.primary]\nbackground = '#1e1e2e'\nforeground = '#cdd6f4'\n[colors.cursor]\ntext = '#1e1e2e'\ncursor = '#f5e0dc'\n[colors.selection]\nbackground = '#585b70'\n[colors.normal]\nblack = '#45475a'\nred = '#f38ba8'\ngreen = '#a6e3a1'\nyellow = '#f9e2af'\nblue = '#89b4fa'\n[colors.bright]\nblack = '#585b70'\nwhite = '#a6adc8'\n").unwrap();
        assert_eq!(hex(a.background), "#1e1e2e");
        assert_eq!(hex(a.cursor), "#f5e0dc");
        assert_eq!(hex(a.selection), "#585b70");
        assert_eq!(hex(a.palette[8]), "#585b70");
        assert_eq!(hex(a.palette[15]), "#a6adc8");
        let y = read("colors:\n  primary:\n    background: '0x002b36'\n    foreground: '0x839496'\n  normal:\n    black: '0x073642'\n    red: '0xdc322f'\n    green: '0x859900'\n    yellow: '0xb58900'\n    blue: '0x268bd2'\n    magenta: '0xd33682'\n").unwrap();
        assert_eq!(hex(y.palette[1]), "#dc322f");
        assert_eq!(hex(y.background), "#002b36");
        let b = read("scheme: \"Ocean\"\nbase00: \"2b303b\"\nbase02: \"4f5b66\"\nbase03: \"65737e\"\nbase05: \"c0c5ce\"\nbase07: \"eff1f5\"\nbase08: \"bf616a\"\nbase0A: \"ebcb8b\"\nbase0B: \"a3be8c\"\nbase0C: \"96b5b4\"\nbase0D: \"8fa1b3\"\nbase0E: \"b48ead\"\n").unwrap();
        assert_eq!(hex(b.palette[4]), "#8fa1b3");
        assert_eq!(hex(b.selection), "#4f5b66");
        assert_eq!(hex(b.palette[8]), "#65737e");
    }

    #[test]
    fn json_and_plist() {
        let wal = read(r##"{"wallpaper": "/x.jpg", "special": {"background": "#0e0f13", "foreground": "#c3c4c6", "cursor": "#c3c4c6"}, "colors": {"color0": "#0e0f13", "color1": "#6B5F63", "color2": "#7A6E70", "color3": "#8B7A73", "color4": "#5C6D80", "color5": "#7C7F88", "color6": "#8D8A8F", "color7": "#c3c4c6"}}"##).unwrap();
        assert_eq!(hex(wal.background), "#0e0f13");
        assert_eq!(hex(wal.palette[4]), "#5c6d80");
        let wt = read(r##"{"name": "One", "background": "#282C34", "foreground": "#DCDFE4", "black": "#282C34", "red": "#E06C75", "green": "#98C379", "yellow": "#E5C07B", "blue": "#61AFEF", "purple": "#C678DD", "brightBlack": "#5A6374", "cursorColor": "#FFFFFF", "selectionBackground": "#3E4451"}"##).unwrap();
        assert_eq!(hex(wt.palette[8]), "#5a6374");
        assert_eq!(hex(wt.cursor), "#ffffff");
        let mut plist = String::from("<?xml version=\"1.0\"?><plist version=\"1.0\"><dict>");
        for (i, name) in [
            "Ansi 0 Color",
            "Ansi 1 Color",
            "Ansi 2 Color",
            "Ansi 3 Color",
            "Ansi 4 Color",
            "Ansi 5 Color",
            "Background Color",
            "Foreground Color",
        ]
        .iter()
        .enumerate()
        {
            plist.push_str(&format!("<key>{name}</key><dict><key>Blue Component</key><real>{}</real><key>Green Component</key><real>0</real><key>Red Component</key><real>1</real></dict>", i as f32 / 10.0));
        }
        let p = read(&plist).unwrap();
        assert_eq!(hex(p.palette[0]), "#ff0000");
        assert_eq!(hex(p.foreground), "#ff00b3");
    }

    #[test]
    fn not_a_scheme() {
        assert!(read("[font]\nsize = 13\n").is_none());
    }
}
