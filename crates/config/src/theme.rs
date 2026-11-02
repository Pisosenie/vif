use crate::Colors;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn hex(v: u32) -> Self {
        Self {
            r: (v >> 16) as u8,
            g: (v >> 8) as u8,
            b: v as u8,
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.strip_prefix('#')?;
        if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        u32::from_str_radix(hex, 16).ok().map(Self::hex)
    }

    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// This color moved `t` of the way to `other`.
    pub fn toward(self, other: Rgb, t: f32) -> Rgb {
        let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
        Rgb {
            r: mix(self.r, other.r),
            g: mix(self.g, other.g),
            b: mix(self.b, other.b),
        }
    }

    pub fn luma(self) -> f32 {
        (0.2126 * f32::from(self.r) + 0.7152 * f32::from(self.g) + 0.0722 * f32::from(self.b)) / 255.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
    pub selection: Rgb,
    pub accent: Rgb,
    pub ansi: [Rgb; 16],
}

impl Palette {
    /// The colors when nothing sets them: the night theme.
    pub const NIGHT: Palette = Palette {
        foreground: Rgb::hex(0xe8e6e1),
        background: Rgb::hex(0x101115),
        cursor: Rgb::hex(0x86bdf0),
        selection: Rgb::hex(0x2c3a4a),
        accent: Rgb::hex(0x86bdf0),
        ansi: [
            Rgb::hex(0x1e2026),
            Rgb::hex(0xff7b6b),
            Rgb::hex(0x5dd39e),
            Rgb::hex(0xf2b35c),
            Rgb::hex(0x86bdf0),
            Rgb::hex(0xc084fc),
            Rgb::hex(0x7fd6e3),
            Rgb::hex(0xd6d5d1),
            Rgb::hex(0x686b75),
            Rgb::hex(0xff9e91),
            Rgb::hex(0x84e2b8),
            Rgb::hex(0xffcb80),
            Rgb::hex(0xa9d1f6),
            Rgb::hex(0xd6aafd),
            Rgb::hex(0xa6e6ef),
            Rgb::hex(0xf4f3f0),
        ],
    };
}

impl Colors {
    /// Makes the colors set so far the base for the ones that come later.
    pub(crate) fn settle(&mut self) {
        self.base = self.resolve(None);
        self.foreground = None;
        self.background = None;
        self.cursor = None;
        self.selection = None;
        self.accent = None;
        self.palette = [None; 16];
    }

    /// The theme's colors with this config's on top; `outside` stands in for colors left unset.
    pub fn resolve(&self, outside: Option<Rgb>) -> Palette {
        let mut palette = self.base;
        if let Some(accent) = outside {
            palette.accent = accent;
            palette.cursor = accent;
            palette.selection = accent.toward(palette.background, 0.72);
        }
        if let Some(c) = self.foreground {
            palette.foreground = c;
        }
        if let Some(c) = self.background {
            palette.background = c;
        }
        if let Some(c) = self.cursor {
            palette.cursor = c;
        }
        if let Some(c) = self.selection {
            palette.selection = c;
        }
        palette.accent = self
            .accent
            .or(self.cursor.filter(|_| outside.is_none()))
            .unwrap_or(palette.accent);
        for (slot, color) in palette.ansi.iter_mut().zip(self.palette) {
            if let Some(color) = color {
                *slot = color;
            }
        }
        palette
    }
}
