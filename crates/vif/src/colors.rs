use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors as TermColors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb as TermRgb};
use vif_config::{Palette, Rgb};

/// The configured palette turned into what the renderer draws with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scheme {
    pub palette: Palette,
    pub opacity: f32,
}

pub type Rgba = [f32; 4];

pub fn rgba(c: Rgb, a: f32) -> Rgba {
    [
        f32::from(c.r) / 255.0,
        f32::from(c.g) / 255.0,
        f32::from(c.b) / 255.0,
        a,
    ]
}

pub fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    ]
}

impl Scheme {
    pub fn fg(&self) -> Rgba {
        rgba(self.palette.foreground, 1.0)
    }

    pub fn bg(&self) -> Rgba {
        rgba(self.palette.background, 1.0)
    }

    pub fn accent(&self) -> Rgba {
        rgba(self.palette.accent, 1.0)
    }

    pub fn cursor(&self) -> Rgba {
        rgba(self.palette.cursor, 1.0)
    }

    /// Foreground and background of a cell after inverse, dim and hidden.
    pub fn cell(&self, overrides: &TermColors, fg: Color, bg: Color, flags: Flags) -> (Rgba, Rgba, bool) {
        let mut front = self.resolve(overrides, fg);
        let mut back = self.resolve(overrides, bg);
        let default_bg = matches!(bg, Color::Named(NamedColor::Background));
        let mut plain_bg = default_bg;
        if flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut front, &mut back);
            plain_bg = false;
        }
        if flags.contains(Flags::DIM) {
            front = mix(front, back, 0.38);
        }
        if flags.contains(Flags::HIDDEN) {
            front = back;
        }
        (front, back, plain_bg)
    }

    pub fn resolve(&self, overrides: &TermColors, color: Color) -> Rgba {
        match color {
            Color::Spec(c) => term_rgba(c),
            Color::Indexed(i) => match overrides[i as usize] {
                Some(c) => term_rgba(c),
                None => rgba(self.indexed(i), 1.0),
            },
            Color::Named(name) => match overrides[name] {
                Some(c) => term_rgba(c),
                None => self.named(name),
            },
        }
    }

    fn named(&self, name: NamedColor) -> Rgba {
        let p = &self.palette;
        match name as usize {
            n @ 0..=15 => rgba(p.ansi[n], 1.0),
            _ => match name {
                NamedColor::Foreground | NamedColor::BrightForeground => rgba(p.foreground, 1.0),
                NamedColor::Background => rgba(p.background, 1.0),
                NamedColor::Cursor => rgba(p.cursor, 1.0),
                NamedColor::DimForeground => mix(self.fg(), self.bg(), 0.38),
                other => {
                    let base = other as usize - NamedColor::DimBlack as usize;
                    mix(rgba(p.ansi[base.min(7)], 1.0), self.bg(), 0.38)
                }
            },
        }
    }

    fn indexed(&self, i: u8) -> Rgb {
        match i {
            0..=15 => self.palette.ansi[i as usize],
            16..=231 => {
                let i = i - 16;
                let step = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                Rgb {
                    r: step(i / 36),
                    g: step((i / 6) % 6),
                    b: step(i % 6),
                }
            }
            _ => {
                let v = 8 + (i - 232) * 10;
                Rgb { r: v, g: v, b: v }
            }
        }
    }
}

fn term_rgba(c: TermRgb) -> Rgba {
    rgba(Rgb { r: c.r, g: c.g, b: c.b }, 1.0)
}
