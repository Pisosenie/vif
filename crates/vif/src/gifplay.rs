//! `vif gif FILE`: a GIF (or a PNG or JPEG) drawn in text and played where you are, in any
//! terminal. Every frame is scaled and turned into lines once, up front; playing only writes the
//! lines that changed, in the GIF's own time, so it costs next to nothing while it runs.

use std::io::{self, Read, Write};
use std::path::Path;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const USAGE: &str = "       vif gif FILE [--blocks] [--mono] [--width N] [--once] [--text]
                                   play a GIF in text: --blocks draws ▀ halves for twice the
                                   detail, --mono without color, --text prints the frames";

/// Characters from empty to full.
const RAMP: &[u8] = b" .,:;-=+*#%@";

/// Pixels a GIF may have, and frames that play; past them it's a bomb, not a picture.
const LARGEST: u64 = 4096 * 4096;
const MOST_FRAMES: usize = 1000;

static STOP: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
struct Options {
    blocks: bool,
    mono: bool,
    once: bool,
    text: bool,
    width: Option<usize>,
}

pub(crate) struct Frame {
    pub lines: Vec<String>,
    pub delay: Duration,
}

pub(crate) struct Clip {
    pub frames: Vec<Frame>,
    pub cols: usize,
    pub rows: usize,
}

pub fn run(args: &[String]) -> ExitCode {
    let mut options = Options::default();
    let mut path = None;
    let mut words = args.iter();
    while let Some(arg) = words.next() {
        match arg.as_str() {
            "--blocks" => options.blocks = true,
            "--mono" => options.mono = true,
            "--once" => options.once = true,
            "--text" => options.text = true,
            "--width" => options.width = words.next().and_then(|w| w.parse().ok()).filter(|w| *w > 0),
            _ if path.is_none() => path = Some(arg.clone()),
            _ => return usage(),
        }
    }
    let Some(path) = path else {
        return usage();
    };
    match play(&path, &options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("vif: {path}: {err}");
            ExitCode::FAILURE
        }
    }
}

fn usage() -> ExitCode {
    eprintln!("usage:\n{USAGE}");
    ExitCode::from(2)
}

fn play(path: &str, options: &Options) -> Result<(), String> {
    let (cols, rows, cell) = screen();
    let text = options.text;
    let clip = load(
        Path::new(path),
        options.blocks && !text,
        options.mono || text,
        options.width.unwrap_or(cols),
        rows.saturating_sub(1).max(4),
        cell,
    )?;
    let frames = clip.frames;
    if text {
        let mut out = io::stdout().lock();
        for (i, frame) in frames.iter().enumerate() {
            if frames.len() > 1 {
                let _ = writeln!(
                    out,
                    "# frame {} of {}, {} ms",
                    i + 1,
                    frames.len(),
                    frame.delay.as_millis()
                );
            }
            for line in &frame.lines {
                let _ = writeln!(out, "{}", line.trim_end());
            }
        }
        return Ok(());
    }
    run_frames(&frames, clip.cols, clip.rows, options.once)
}

/// A picture drawn in text at most `cols` by `rows` cells, every frame ready to write.
pub(crate) fn load(path: &Path, blocks: bool, mono: bool, cols: usize, rows: usize, cell: f64) -> Result<Clip, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    decode(&data, blocks, mono, (cols, rows), cell)
}

/// [`load`] from the file's bytes.
pub(crate) fn decode(data: &[u8], blocks: bool, mono: bool, room: (usize, usize), cell: f64) -> Result<Clip, String> {
    let size = |w, h| fit(w, h, room.0, room.1, cell);
    let (shrunk, (cols, rows)) = if data.starts_with(b"GIF8") {
        frames(data, size)?
    } else {
        let (rgba, w, h) = crate::wallpaper::decode(data)?;
        let (cols, rows) = size(w, h);
        (
            vec![(shrink(&rgba, w, h, cols, rows * 2), Duration::ZERO)],
            (cols, rows),
        )
    };
    let samples: Vec<&[[u8; 4]]> = shrunk.iter().map(|(s, _)| s.as_slice()).collect();
    let range = levels(&samples);
    let frames = shrunk
        .iter()
        .map(|(samples, delay)| Frame {
            lines: draw(samples, cols, rows, blocks, mono, range),
            delay: *delay,
        })
        .collect();
    Ok(Clip { frames, cols, rows })
}

/// The terminal's columns and lines, and how many times taller than wide a cell is.
pub(crate) fn screen() -> (usize, usize, f64) {
    // SAFETY: a C struct of plain numbers and arrays, for which all zeroes is a valid value.
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    // SAFETY: TIOCGWINSZ fills the winsize we pass.
    if unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) } != 0 || size.ws_col == 0 {
        return (80, 24, 2.0);
    }
    let (cols, rows) = (usize::from(size.ws_col), usize::from(size.ws_row));
    let cell = if size.ws_xpixel > 0 && size.ws_ypixel > 0 {
        (f64::from(size.ws_ypixel) / rows as f64) / (f64::from(size.ws_xpixel) / cols as f64)
    } else {
        2.0
    };
    (cols, rows, cell.clamp(1.0, 4.0))
}

/// Columns and lines for a picture of `w` by `h` pixels in cells `cell` times taller than wide.
fn fit(w: u32, h: u32, cols: usize, rows: usize, cell: f64) -> (usize, usize) {
    let aspect = f64::from(h) / f64::from(w.max(1)) / cell;
    let mut c = cols.max(1);
    let mut r = (c as f64 * aspect).round().max(1.0) as usize;
    if r > rows {
        r = rows;
        c = ((r as f64 / aspect).round() as usize).clamp(1, cols);
    }
    (c, r)
}

/// Every frame of a GIF laid on the one before it, as the GIF says to, scaled down to the
/// cells `size` gives the whole picture as soon as it's drawn, so only one full frame is ever
/// held.
#[allow(clippy::type_complexity)]
fn frames(
    data: &[u8],
    size: impl FnOnce(u32, u32) -> (usize, usize),
) -> Result<(Vec<(Vec<[u8; 4]>, Duration)>, (usize, usize)), String> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options.read_info(data).map_err(|e| e.to_string())?;
    let (w, h) = (u32::from(decoder.width()), u32::from(decoder.height()));
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > LARGEST {
        return Err(format!("{w}×{h} is more than vif plays"));
    }
    let (cols, rows) = size(w, h);
    let mut canvas = vec![0u8; (w * h * 4) as usize];
    let mut out = Vec::new();
    while let Some(frame) = decoder.read_next_frame().map_err(|e| e.to_string())? {
        if out.len() == MOST_FRAMES {
            break;
        }
        let before = (frame.dispose == gif::DisposalMethod::Previous).then(|| canvas.clone());
        let (fl, ft, fw, fh) = (
            u32::from(frame.left),
            u32::from(frame.top),
            u32::from(frame.width),
            u32::from(frame.height),
        );
        for y in 0..fh.min(h.saturating_sub(ft)) {
            for x in 0..fw.min(w.saturating_sub(fl)) {
                let from = ((y * fw + x) * 4) as usize;
                let px = &frame.buffer[from..from + 4];
                if px[3] > 0 {
                    let to = (((ft + y) * w + fl + x) * 4) as usize;
                    canvas[to..to + 4].copy_from_slice(px);
                }
            }
        }
        // Browsers play delays under 20 ms at 100 ms; so does this.
        let delay = match frame.delay {
            0 | 1 => Duration::from_millis(100),
            d => Duration::from_millis(u64::from(d) * 10),
        };
        out.push((shrink(&canvas, w, h, cols, rows * 2), delay));
        match frame.dispose {
            gif::DisposalMethod::Background => {
                for y in ft..(ft + fh).min(h) {
                    for x in fl..(fl + fw).min(w) {
                        let at = ((y * w + x) * 4) as usize;
                        canvas[at..at + 4].fill(0);
                    }
                }
            }
            gif::DisposalMethod::Previous => {
                if let Some(before) = before {
                    canvas = before;
                }
            }
            _ => {}
        }
    }
    if out.is_empty() {
        return Err("no frames in it".to_owned());
    }
    Ok((out, (cols, rows)))
}

/// The picture averaged down to `cols` by `rows` samples, each RGBA.
fn shrink(rgba: &[u8], w: u32, h: u32, cols: usize, rows: usize) -> Vec<[u8; 4]> {
    let (w, h) = (w as usize, h as usize);
    let mut out = Vec::with_capacity(cols * rows);
    for r in 0..rows {
        let (y0, y1) = (r * h / rows, ((r + 1) * h / rows).max(r * h / rows + 1).min(h));
        for c in 0..cols {
            let (x0, x1) = (c * w / cols, ((c + 1) * w / cols).max(c * w / cols + 1).min(w));
            let mut sum = [0u32; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    let at = (y * w + x) * 4;
                    let a = u32::from(rgba[at + 3]);
                    for (i, s) in sum.iter_mut().take(3).enumerate() {
                        *s += u32::from(rgba[at + i]) * a / 255;
                    }
                    sum[3] += a;
                }
            }
            let n = ((x1 - x0) * (y1 - y0)).max(1) as u32;
            let a = sum[3] / n;
            let color = |i: usize| (sum[i] * 255).checked_div(sum[3]).unwrap_or(0) as u8;
            out.push([color(0), color(1), color(2), a as u8]);
        }
    }
    out
}

fn luma(p: [u8; 4]) -> f32 {
    (0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2])) / 255.0 * f32::from(p[3]) / 255.0
}

/// The darkest and lightest the picture gets, leaving out the odd pixel, so the characters
/// use their whole range.
fn levels(frames: &[&[[u8; 4]]]) -> (f32, f32) {
    let mut all: Vec<f32> = frames
        .iter()
        .flat_map(|f| f.iter())
        .filter(|p| p[3] >= 128)
        .map(|p| luma(*p))
        .collect();
    if all.len() < 2 {
        return (0.0, 1.0);
    }
    all.sort_by(f32::total_cmp);
    let (low, high) = (all[all.len() / 50], all[all.len() - 1 - all.len() / 50]);
    if high - low < 0.05 { (0.0, 1.0) } else { (low, high) }
}

/// The color at full strength, so a dark picture still shows its colors.
fn vivid(p: [u8; 4]) -> [u8; 4] {
    let top = p[0].max(p[1]).max(p[2]).max(1);
    let lift = |c: u8| (u16::from(c) * 255 / u16::from(top)) as u8;
    [lift(p[0]), lift(p[1]), lift(p[2]), p[3]]
}

/// Colors rounded to 32 steps a channel, so the same color repeats and needs no new escape.
fn round(p: [u8; 4]) -> [u8; 3] {
    [p[0] | 7, p[1] | 7, p[2] | 7]
}

/// Lines of text for the samples: two samples a cell, one over the other.
fn draw(samples: &[[u8; 4]], cols: usize, rows: usize, blocks: bool, mono: bool, range: (f32, f32)) -> Vec<String> {
    let mut lines = Vec::with_capacity(rows);
    for r in 0..rows {
        let mut line = String::with_capacity(cols * 4);
        let mut last: Option<String> = None;
        for c in 0..cols {
            let top = samples[(2 * r) * cols + c];
            let bottom = samples[(2 * r + 1) * cols + c];
            let (glyph, sgr) = if blocks && !mono {
                let (t, b) = (top[3] >= 128, bottom[3] >= 128);
                let [tr, tg, tb] = round(top);
                let [br, bg, bb] = round(bottom);
                match (t, b) {
                    (false, false) => (' ', "0".to_owned()),
                    (true, false) => ('▀', format!("0;38;2;{tr};{tg};{tb}")),
                    (false, true) => ('▄', format!("0;38;2;{br};{bg};{bb}")),
                    (true, true) => ('▀', format!("38;2;{tr};{tg};{tb};48;2;{br};{bg};{bb}")),
                }
            } else if blocks {
                let glyph = match (luma(top) >= 0.5, luma(bottom) >= 0.5) {
                    (true, true) => '█',
                    (true, false) => '▀',
                    (false, true) => '▄',
                    (false, false) => ' ',
                };
                (glyph, String::new())
            } else {
                let mix = |i: usize| ((u16::from(top[i]) + u16::from(bottom[i])) / 2) as u8;
                let cell = [mix(0), mix(1), mix(2), mix(3)];
                let level = ((luma(cell) - range.0) / (range.1 - range.0)).clamp(0.0, 1.0);
                let glyph = RAMP[((level * (RAMP.len() - 1) as f32).round() as usize).min(RAMP.len() - 1)] as char;
                let [cr, cg, cb] = round(vivid(cell));
                let sgr = if mono || glyph == ' ' {
                    String::new()
                } else {
                    format!("38;2;{cr};{cg};{cb}")
                };
                (glyph, sgr)
            };
            if !mono && !sgr.is_empty() && last.as_deref() != Some(sgr.as_str()) {
                line.push_str("\x1b[");
                line.push_str(&sgr);
                line.push('m');
                last = Some(sgr);
            }
            line.push(glyph);
        }
        if !mono {
            line.push_str("\x1b[0m");
        }
        lines.push(line);
    }
    lines
}

extern "C" fn stop(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

/// Plays the frames on the alternate screen until q, Esc or ctrl-c, or once through.
fn run_frames(frames: &[Frame], cols: usize, rows: usize, once: bool) -> Result<(), String> {
    let (screen_cols, screen_rows, _) = screen();
    let left = screen_cols.saturating_sub(cols) / 2 + 1;
    let top = screen_rows.saturating_sub(rows) / 2 + 1;
    // SAFETY: the handler only stores to an atomic, which a signal handler may do.
    unsafe {
        libc::signal(libc::SIGINT, stop as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, stop as *const () as libc::sighandler_t);
    }
    // Keys go to this, not onto the screen, and don't wait for Enter.
    // SAFETY: a C struct of plain numbers and arrays, for which all zeroes is a valid value.
    let mut saved: libc::termios = unsafe { std::mem::zeroed() };
    // SAFETY: tcgetattr fills the termios we pass; failure is checked.
    let quiet = unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut saved) } == 0;
    if quiet {
        let mut raw = saved;
        raw.c_lflag &= !(libc::ICANON | libc::ECHO);
        raw.c_cc[libc::VMIN] = 0;
        raw.c_cc[libc::VTIME] = 0;
        // SAFETY: the termios tcgetattr filled, with only its flags changed.
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) };
    }
    let mut out = io::stdout().lock();
    let _ = out.write_all(b"\x1b[?1049h\x1b[?25l\x1b[2J");
    let mut shown: Vec<&str> = vec![""; rows];
    let mut next = Instant::now();
    let mut buf = String::new();
    let mut stdin = io::stdin();
    'play: loop {
        for frame in frames {
            buf.clear();
            buf.push_str("\x1b[?2026h");
            for (i, line) in frame.lines.iter().enumerate() {
                if shown[i] != line.as_str() {
                    buf.push_str(&format!("\x1b[{};{left}H", top + i));
                    buf.push_str(line);
                    shown[i] = line.as_str();
                }
            }
            buf.push_str("\x1b[?2026l");
            if out.write_all(buf.as_bytes()).and_then(|()| out.flush()).is_err() {
                break 'play;
            }
            next += frame.delay;
            // Sleeps in short steps so q and ctrl-c answer at once.
            loop {
                if STOP.load(Ordering::Relaxed) {
                    break 'play;
                }
                let mut key = [0u8; 8];
                if quiet
                    && let Ok(n) = stdin.read(&mut key)
                    && key[..n].iter().any(|k| matches!(k, b'q' | 0x1b | 3))
                {
                    break 'play;
                }
                let now = Instant::now();
                if now >= next {
                    break;
                }
                std::thread::sleep((next - now).min(Duration::from_millis(50)));
            }
            if frames.len() == 1 {
                // A still picture stays until a key.
                next = Instant::now() + Duration::from_secs(3600);
            }
        }
        if once && frames.len() > 1 {
            break;
        }
        if Instant::now() > next + Duration::from_secs(1) {
            next = Instant::now();
        }
    }
    let _ = out.write_all(b"\x1b[0m\x1b[?25h\x1b[?1049l");
    let _ = out.flush();
    if quiet {
        // SAFETY: the termios as tcgetattr gave it.
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &saved) };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_the_screen_keeping_the_shape() {
        assert_eq!(fit(200, 100, 80, 40, 2.0), (80, 20));
        assert_eq!(fit(100, 400, 80, 40, 2.0), (20, 40));
    }

    #[test]
    fn draws_light_as_dense_characters() {
        let white = [255, 255, 255, 255];
        let clear = [0, 0, 0, 0];
        let lines = draw(&[white, clear, white, clear], 2, 1, false, true, (0.0, 1.0));
        assert_eq!(lines, vec!["@ ".to_owned()]);
        let blocks = draw(&[white, clear, clear, white], 2, 1, true, true, (0.0, 1.0));
        assert_eq!(blocks, vec!["▀▄".to_owned()]);
    }
}
