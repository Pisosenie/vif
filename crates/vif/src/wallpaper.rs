//! The background picture: read once, made no bigger than a screen needs, blurred on the CPU.

use std::hash::{Hash, Hasher};
use std::path::Path;

use vif_config::{Background, Fit};
use vif_render::Sprite;

/// Longer than any side a window gets; bigger pictures are scaled down to it.
const LONGEST: u32 = 3840;

pub struct Picture {
    pub key: u64,
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl std::fmt::Debug for Picture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Picture({}x{})", self.width, self.height)
    }
}

pub fn load(background: &Background) -> Option<Result<Picture, String>> {
    let name = background.image.as_deref()?;
    let path = vif_config::file(name, "");
    Some(read(&path, background.blur).map_err(|e| format!("background {}: {e}", crate::tilde(&path))))
}

fn read(path: &Path, blur: u8) -> Result<Picture, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (path, data.len(), &data[..data.len().min(4096)], blur).hash(&mut hasher);
    let key = hasher.finish() | 1 << 63;
    let (rgba, width, height) = decode(&data)?;
    let (rgba, width, height) = shrink(rgba, width, height);
    let rgba = if blur > 0 {
        blurred(rgba, width, height, u32::from(blur))
    } else {
        rgba
    };
    Ok(Picture {
        key,
        rgba,
        width,
        height,
    })
}

/// A PNG or JPEG as RGBA, with its width and height.
pub fn decode(data: &[u8]) -> Result<(Vec<u8>, u32, u32), String> {
    if data.starts_with(b"\x89PNG") {
        let image = crate::graphics::png(data).map_err(|e| e.trim_start_matches("EBADPNG:").to_owned())?;
        Ok((image.rgba, image.width, image.height))
    } else if data.starts_with(&[0xff, 0xd8]) {
        jpeg(data)
    } else {
        Err("only PNG and JPEG pictures".to_owned())
    }
}

fn jpeg(data: &[u8]) -> Result<(Vec<u8>, u32, u32), String> {
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(std::io::Cursor::new(data), options);
    let rgba = decoder.decode().map_err(|e| e.to_string())?;
    let info = decoder.info().ok_or("no picture in it")?;
    Ok((rgba, u32::from(info.width), u32::from(info.height)))
}

/// Halves the picture, averaging pixels, until it fits in [`LONGEST`].
fn shrink(mut rgba: Vec<u8>, mut w: u32, mut h: u32) -> (Vec<u8>, u32, u32) {
    while w.max(h) > LONGEST && w > 1 && h > 1 {
        let (nw, nh) = (w / 2, h / 2);
        let mut out = vec![0; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let at = |dx: u32, dy: u32| u32::from(rgba[(((2 * y + dy) * w + 2 * x + dx) * 4 + c) as usize]);
                    out[((y * nw + x) * 4 + c) as usize] = ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1)) / 4) as u8;
                }
            }
        }
        (rgba, w, h) = (out, nw, nh);
    }
    (rgba, w, h)
}

/// Three box blurs one way and the other, close enough to a Gaussian.
fn blurred(mut rgba: Vec<u8>, w: u32, h: u32, radius: u32) -> Vec<u8> {
    let r = (radius / 2).max(1) as i64;
    let (w, h) = (w as i64, h as i64);
    let mut line = Vec::new();
    for _ in 0..3 {
        for horizontal in [true, false] {
            let (count, length) = if horizontal { (h, w) } else { (w, h) };
            for i in 0..count {
                let index = |j: i64| -> usize {
                    let j = j.clamp(0, length - 1);
                    (if horizontal { i * w + j } else { j * w + i }) as usize * 4
                };
                line.clear();
                line.extend((0..length).map(|j| {
                    let at = index(j);
                    [rgba[at], rgba[at + 1], rgba[at + 2], rgba[at + 3]]
                }));
                let mut sum = [0i64; 4];
                for j in -r..=r {
                    let p = line[j.clamp(0, length - 1) as usize];
                    for c in 0..4 {
                        sum[c] += i64::from(p[c]);
                    }
                }
                for j in 0..length {
                    let at = index(j);
                    for c in 0..4 {
                        rgba[at + c] = (sum[c] / (2 * r + 1)) as u8;
                    }
                    let (out, inn) = (
                        line[(j - r).clamp(0, length - 1) as usize],
                        line[(j + r + 1).clamp(0, length - 1) as usize],
                    );
                    for c in 0..4 {
                        sum[c] += i64::from(inn[c]) - i64::from(out[c]);
                    }
                }
            }
        }
    }
    rgba
}

/// Where the picture goes in a `width` by `height` window, as sprites over uv 0–1.
pub fn place(fit: Fit, image: (u32, u32), width: f32, height: f32, alpha: f32) -> Vec<Sprite> {
    let (iw, ih) = (image.0 as f32, image.1 as f32);
    let color = [1.0, 1.0, 1.0, alpha];
    let sprite = |pos: [f32; 2], size: [f32; 2], uv: [f32; 4]| Sprite { pos, size, uv, color };
    match fit {
        Fit::Stretch => vec![sprite([0.0, 0.0], [width, height], [0.0, 0.0, 1.0, 1.0])],
        Fit::Cover => {
            let scale = (width / iw).max(height / ih);
            let (vw, vh) = (width / scale / iw, height / scale / ih);
            vec![sprite(
                [0.0, 0.0],
                [width, height],
                [(1.0 - vw) / 2.0, (1.0 - vh) / 2.0, vw, vh],
            )]
        }
        Fit::Contain | Fit::Center => {
            let scale = if fit == Fit::Contain {
                (width / iw).min(height / ih)
            } else {
                1.0
            };
            let (w, h) = (iw * scale, ih * scale);
            let pos = [((width - w) / 2.0).round(), ((height - h) / 2.0).round()];
            vec![sprite(pos, [w, h], [0.0, 0.0, 1.0, 1.0])]
        }
        Fit::Tile => {
            let mut out = Vec::new();
            let mut y = 0.0;
            while y < height {
                let mut x = 0.0;
                while x < width {
                    out.push(sprite([x, y], [iw, ih], [0.0, 0.0, 1.0, 1.0]));
                    x += iw;
                }
                y += ih;
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_crops_the_middle() {
        let s = place(Fit::Cover, (2000, 1000), 1000.0, 1000.0, 1.0);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].uv, [0.25, 0.0, 0.5, 1.0]);
        let c = place(Fit::Contain, (2000, 1000), 1000.0, 1000.0, 1.0);
        assert_eq!(c[0].size, [1000.0, 500.0]);
        assert_eq!(c[0].pos, [0.0, 250.0]);
        assert_eq!(place(Fit::Tile, (300, 300), 1000.0, 500.0, 1.0).len(), 8);
    }

    #[test]
    fn shrinks_and_blurs() {
        let (rgba, w, h) = shrink(vec![255; 8000 * 2 * 4], 8000, 2);
        assert_eq!((w, h, rgba.len()), (4000, 1, 16_000));
        let mut dot = vec![0u8; 9 * 9 * 4];
        dot[(4 * 9 + 4) * 4] = 255;
        let soft = blurred(dot, 9, 9, 2);
        assert!(soft[(4 * 9 + 4) * 4] < 255 && soft[(4 * 9 + 5) * 4] > 0);
    }
}
