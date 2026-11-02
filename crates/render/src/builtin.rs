//! Box drawing, block elements, braille and powerline symbols drawn to fit the cell exactly,
//! so lines join across cells and rows whatever the font.

pub fn covers(c: char) -> bool {
    matches!(c as u32, 0x2500..=0x259F | 0x2800..=0x28FF | 0xE0B0..=0xE0B7)
}

/// A coverage mask of `w`×`h` pixels, or `None` when the character isn't drawn here.
pub fn draw(c: char, w: u32, h: u32, stroke: f32) -> Option<Vec<u8>> {
    let mut canvas = Canvas::new(w, h, stroke);
    let cp = c as u32;
    match cp {
        0x2500..=0x257F => canvas.box_drawing(cp)?,
        0x2580..=0x259F => canvas.block(cp)?,
        0x2800..=0x28FF => canvas.braille(cp - 0x2800),
        0xE0B0..=0xE0B7 => canvas.powerline(cp),
        _ => return None,
    }
    Some(canvas.pixels)
}

struct Canvas {
    w: u32,
    h: u32,
    light: f32,
    pixels: Vec<u8>,
}

// Arm weights for U+2500..=U+254B: up, right, down, left; 1 light, 2 heavy.
const ARMS: [[u8; 4]; 76] = [
    [0, 1, 0, 1],
    [0, 2, 0, 2],
    [1, 0, 1, 0],
    [2, 0, 2, 0],
    [0, 1, 0, 1],
    [0, 2, 0, 2],
    [1, 0, 1, 0],
    [2, 0, 2, 0],
    [0, 1, 0, 1],
    [0, 2, 0, 2],
    [1, 0, 1, 0],
    [2, 0, 2, 0],
    [0, 1, 1, 0],
    [0, 2, 1, 0],
    [0, 1, 2, 0],
    [0, 2, 2, 0],
    [0, 0, 1, 1],
    [0, 0, 1, 2],
    [0, 0, 2, 1],
    [0, 0, 2, 2],
    [1, 1, 0, 0],
    [1, 2, 0, 0],
    [2, 1, 0, 0],
    [2, 2, 0, 0],
    [1, 0, 0, 1],
    [1, 0, 0, 2],
    [2, 0, 0, 1],
    [2, 0, 0, 2],
    [1, 1, 1, 0],
    [1, 2, 1, 0],
    [2, 1, 1, 0],
    [1, 1, 2, 0],
    [2, 1, 2, 0],
    [2, 2, 1, 0],
    [1, 2, 2, 0],
    [2, 2, 2, 0],
    [1, 0, 1, 1],
    [1, 0, 1, 2],
    [2, 0, 1, 1],
    [1, 0, 2, 1],
    [2, 0, 2, 1],
    [2, 0, 1, 2],
    [1, 0, 2, 2],
    [2, 0, 2, 2],
    [0, 1, 1, 1],
    [0, 1, 1, 2],
    [0, 2, 1, 1],
    [0, 2, 1, 2],
    [0, 1, 2, 1],
    [0, 1, 2, 2],
    [0, 2, 2, 1],
    [0, 2, 2, 2],
    [1, 1, 0, 1],
    [1, 1, 0, 2],
    [1, 2, 0, 1],
    [1, 2, 0, 2],
    [2, 1, 0, 1],
    [2, 1, 0, 2],
    [2, 2, 0, 1],
    [2, 2, 0, 2],
    [1, 1, 1, 1],
    [1, 1, 1, 2],
    [1, 2, 1, 1],
    [1, 2, 1, 2],
    [2, 1, 1, 1],
    [1, 1, 2, 1],
    [2, 1, 2, 1],
    [2, 1, 1, 2],
    [2, 2, 1, 1],
    [1, 1, 2, 2],
    [1, 2, 2, 1],
    [2, 1, 2, 2],
    [2, 2, 2, 1],
    [1, 2, 2, 2],
    [2, 2, 1, 2],
    [2, 2, 2, 2],
];

// Arm weights for U+2550..=U+256C; 3 is a double line.
const DOUBLE: [[u8; 4]; 29] = [
    [0, 3, 0, 3],
    [3, 0, 3, 0],
    [0, 3, 1, 0],
    [0, 1, 3, 0],
    [0, 3, 3, 0],
    [0, 0, 1, 3],
    [0, 0, 3, 1],
    [0, 0, 3, 3],
    [1, 3, 0, 0],
    [3, 1, 0, 0],
    [3, 3, 0, 0],
    [1, 0, 0, 3],
    [3, 0, 0, 1],
    [3, 0, 0, 3],
    [1, 3, 1, 0],
    [3, 1, 3, 0],
    [3, 3, 3, 0],
    [1, 0, 1, 3],
    [3, 0, 3, 1],
    [3, 0, 3, 3],
    [0, 3, 1, 3],
    [0, 1, 3, 1],
    [0, 3, 3, 3],
    [1, 3, 0, 3],
    [3, 1, 0, 1],
    [3, 3, 0, 3],
    [1, 3, 1, 3],
    [3, 1, 3, 1],
    [3, 3, 3, 3],
];

impl Canvas {
    fn new(w: u32, h: u32, stroke: f32) -> Self {
        Self {
            w,
            h,
            light: stroke.max(1.0).round(),
            pixels: vec![0; (w * h) as usize],
        }
    }

    fn fill(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, alpha: u8) {
        let (x0, x1) = (x0.round().max(0.0) as u32, x1.round().min(self.w as f32) as u32);
        let (y0, y1) = (y0.round().max(0.0) as u32, y1.round().min(self.h as f32) as u32);
        for y in y0..y1 {
            for x in x0..x1 {
                let p = &mut self.pixels[(y * self.w + x) as usize];
                *p = (*p).max(alpha);
            }
        }
    }

    /// Fills where `inside` holds, 4×4 supersampled.
    fn shape(&mut self, inside: impl Fn(f32, f32) -> bool) {
        for y in 0..self.h {
            for x in 0..self.w {
                let mut hits = 0u32;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let px = x as f32 + (sx as f32 + 0.5) / 4.0;
                        let py = y as f32 + (sy as f32 + 0.5) / 4.0;
                        hits += u32::from(inside(px, py));
                    }
                }
                let p = &mut self.pixels[(y * self.w + x) as usize];
                *p = (*p).max((hits * 255 / 16) as u8);
            }
        }
    }

    fn thickness(&self, weight: u8) -> f32 {
        if weight == 2 { self.light * 2.0 } else { self.light }
    }

    fn arms(&mut self, arms: [u8; 4]) {
        let (w, h) = (self.w as f32, self.h as f32);
        let cx = ((w - self.light) / 2.0).floor();
        let cy = ((h - self.light) / 2.0).floor();
        let vertical = arms[0].max(arms[2]);
        let horizontal = arms[1].max(arms[3]);
        let vt = self.thickness(vertical);
        let ht = self.thickness(horizontal);
        let (vx, hy) = (
            cx - ((vt - self.light) / 2.0).floor(),
            cy - ((ht - self.light) / 2.0).floor(),
        );
        let gap = self.light;
        for (i, &weight) in arms.iter().enumerate() {
            if weight == 0 {
                continue;
            }
            let t = self.thickness(weight);
            let ox = cx - ((t - self.light) / 2.0).floor();
            let oy = cy - ((t - self.light) / 2.0).floor();
            let double = weight == 3;
            let offsets: &[f32] = if double { &[-gap, gap] } else { &[0.0] };
            for &d in offsets {
                match i {
                    0 => {
                        let end = if double { hy + ht + gap } else { hy + ht };
                        self.fill(ox + d, 0.0, ox + d + t, end, 255)
                    }
                    2 => {
                        let start = if double { hy - gap } else { hy };
                        self.fill(ox + d, start, ox + d + t, h, 255)
                    }
                    1 => {
                        let start = if double { vx - gap } else { vx };
                        self.fill(start, oy + d, w, oy + d + t, 255)
                    }
                    _ => {
                        let end = if double { vx + vt + gap } else { vx + vt };
                        self.fill(0.0, oy + d, end, oy + d + t, 255)
                    }
                }
            }
        }
    }

    fn box_drawing(&mut self, cp: u32) -> Option<()> {
        let (w, h) = (self.w as f32, self.h as f32);
        match cp {
            0x2500..=0x254B => self.arms(ARMS[(cp - 0x2500) as usize]),
            0x254C | 0x254D => self.arms([0, (cp - 0x254B) as u8, 0, (cp - 0x254B) as u8]),
            0x254E | 0x254F => self.arms([(cp - 0x254D) as u8, 0, (cp - 0x254D) as u8, 0]),
            0x2550..=0x256C => self.arms(DOUBLE[(cp - 0x2550) as usize]),
            0x256D..=0x2570 => {
                let t = self.light;
                let cx = ((w - t) / 2.0).floor() + t / 2.0;
                let cy = ((h - t) / 2.0).floor() + t / 2.0;
                let r = cx.min(cy);
                let (sx, sy) = match cp {
                    0x256D => (1.0, 1.0),
                    0x256E => (-1.0, 1.0),
                    0x256F => (-1.0, -1.0),
                    _ => (1.0, -1.0),
                };
                let (ax, ay) = (cx + sx * r, cy + sy * r);
                self.shape(|x, y| {
                    let (dx, dy) = (x - ax, y - ay);
                    let in_corner = dx * sx <= 0.0 && dy * sy <= 0.0;
                    let ring = in_corner && ((dx * dx + dy * dy).sqrt() - r).abs() <= t / 2.0;
                    let arm_h = (y - cy).abs() <= t / 2.0 && (x - ax) * sx >= 0.0;
                    let arm_v = (x - cx).abs() <= t / 2.0 && (y - ay) * sy >= 0.0;
                    ring || arm_h || arm_v
                });
            }
            0x2571..=0x2573 => {
                let t = self.light * 0.75;
                let len = (w * w + h * h).sqrt();
                let up = cp != 0x2572;
                let down = cp != 0x2571;
                self.shape(|x, y| {
                    let a = up && ((h * x + w * y - w * h).abs() / len) <= t;
                    let b = down && ((h * x - w * y).abs() / len) <= t;
                    a || b
                });
            }
            0x2574..=0x257F => {
                let arms: [u8; 4] = match cp {
                    0x2574 => [0, 0, 0, 1],
                    0x2575 => [1, 0, 0, 0],
                    0x2576 => [0, 1, 0, 0],
                    0x2577 => [0, 0, 1, 0],
                    0x2578 => [0, 0, 0, 2],
                    0x2579 => [2, 0, 0, 0],
                    0x257A => [0, 2, 0, 0],
                    0x257B => [0, 0, 2, 0],
                    0x257C => [0, 2, 0, 1],
                    0x257D => [1, 0, 2, 0],
                    0x257E => [0, 1, 0, 2],
                    _ => [2, 0, 1, 0],
                };
                self.arms(arms);
            }
            _ => return None,
        }
        Some(())
    }

    fn block(&mut self, cp: u32) -> Option<()> {
        let (w, h) = (self.w as f32, self.h as f32);
        match cp {
            0x2580 => self.fill(0.0, 0.0, w, h / 2.0, 255),
            0x2581..=0x2588 => {
                let eighths = (cp - 0x2580) as f32;
                self.fill(0.0, h - h * eighths / 8.0, w, h, 255);
            }
            0x2589..=0x258F => {
                let eighths = (0x2590 - cp) as f32;
                self.fill(0.0, 0.0, w * eighths / 8.0, h, 255);
            }
            0x2590 => self.fill(w / 2.0, 0.0, w, h, 255),
            0x2591..=0x2593 => {
                let alpha = [64, 128, 192][(cp - 0x2591) as usize];
                self.fill(0.0, 0.0, w, h, alpha);
            }
            0x2594 => self.fill(0.0, 0.0, w, h / 8.0, 255),
            0x2595 => self.fill(w - w / 8.0, 0.0, w, h, 255),
            0x2596..=0x259F => {
                // quadrants: upper-left, upper-right, lower-left, lower-right
                let quads: [u8; 10] = [
                    0b0010, 0b0001, 0b1000, 0b1011, 0b1001, 0b1110, 0b1101, 0b0100, 0b0110, 0b0111,
                ];
                let q = quads[(cp - 0x2596) as usize];
                let (hw, hh) = (w / 2.0, h / 2.0);
                if q & 0b1000 != 0 {
                    self.fill(0.0, 0.0, hw, hh, 255);
                }
                if q & 0b0100 != 0 {
                    self.fill(hw, 0.0, w, hh, 255);
                }
                if q & 0b0010 != 0 {
                    self.fill(0.0, hh, hw, h, 255);
                }
                if q & 0b0001 != 0 {
                    self.fill(hw, hh, w, h, 255);
                }
            }
            _ => return None,
        }
        Some(())
    }

    fn braille(&mut self, bits: u32) {
        let (w, h) = (self.w as f32, self.h as f32);
        let dot = (w / 4.0).min(h / 8.0).max(1.0).round();
        let positions = [(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2), (0, 3), (1, 3)];
        for (i, (col, row)) in positions.iter().enumerate() {
            if bits & (1 << i) == 0 {
                continue;
            }
            let x = (w / 4.0 * (1.0 + 2.0 * *col as f32) - dot / 2.0).round();
            let y = (h / 8.0 * (1.0 + 2.0 * *row as f32) - dot / 2.0).round();
            self.fill(x, y, x + dot, y + dot, 255);
        }
    }

    fn powerline(&mut self, cp: u32) {
        let (w, h) = (self.w as f32, self.h as f32);
        let t = self.light;
        match cp {
            0xE0B0 => self.shape(|x, y| x <= w * (1.0 - (2.0 * y / h - 1.0).abs())),
            0xE0B2 => self.shape(|x, y| w - x <= w * (1.0 - (2.0 * y / h - 1.0).abs())),
            0xE0B1 | 0xE0B3 => {
                let len = (w * w + h * h / 4.0).sqrt();
                self.shape(|x, y| {
                    let x = if cp == 0xE0B1 { x } else { w - x };
                    let y = if y > h / 2.0 { h - y } else { y };
                    ((h / 2.0) * x - w * y).abs() / len <= t / 2.0 + 0.25
                });
            }
            0xE0B4 | 0xE0B6 => self.shape(|x, y| {
                let x = if cp == 0xE0B4 { x } else { w - x };
                let r = h / 2.0;
                let sx = w / r;
                (x / sx).powi(2) + (y - r).powi(2) <= r * r
            }),
            _ => self.shape(|x, y| {
                let x = if cp == 0xE0B5 { x } else { w - x };
                let r = h / 2.0;
                let sx = w / r;
                let d = ((x / sx).powi(2) + (y - r).powi(2)).sqrt();
                (d - r + t / 2.0).abs() <= t / 2.0
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_reach_the_edges() {
        let px = draw('┼', 16, 32, 2.0).unwrap();
        let at = |x: u32, y: u32| px[(y * 16 + x) as usize];
        assert_eq!(at(0, 15), 255);
        assert_eq!(at(15, 15), 255);
        assert_eq!(at(7, 0), 255);
        assert_eq!(at(7, 31), 255);
        assert_eq!(at(0, 0), 0);
    }

    #[test]
    fn everything_in_range_draws() {
        for cp in (0x2500..=0x259F).chain(0x2800..=0x28FF).chain(0xE0B0..=0xE0B7) {
            let c = char::from_u32(cp).unwrap();
            assert!(draw(c, 16, 32, 2.0).is_some(), "{c:?}");
        }
    }
}
