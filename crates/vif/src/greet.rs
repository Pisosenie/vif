//! `vif greet`: a picture played in text right where the shell is, with who and where beside
//! it, the way people run neofetch. The shell integration runs it before the first prompt when
//! `[greeting]` asks for it; the picture stays, and the prompt comes up under it.

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use vif_config::Config;

use crate::facts;
use crate::gifplay::{self, Clip};

pub const USAGE: &str =
    "       vif greet [FILE]            the greeting from [greeting], or FILE in its picture's place";

/// Cells between the picture and the lines beside it.
const GAP: usize = 3;
/// However many loops the config asks for, the prompt waits no longer than this.
const LONGEST: Duration = Duration::from_secs(15);

static STOP: AtomicBool = AtomicBool::new(false);

pub fn run(args: &[String]) -> ExitCode {
    if args.len() > 1 || args.first().is_some_and(|a| a.starts_with('-')) {
        let help = args.first().is_some_and(|a| a == "--help" || a == "-h");
        eprintln!("usage:\n{USAGE}");
        return if help { ExitCode::SUCCESS } else { ExitCode::from(2) };
    }
    let (config, _) = Config::load();
    let greeting = &config.greeting;
    let file = match args.first() {
        Some(path) => Some(PathBuf::from(path)),
        None => greeting.file.as_deref().map(|f| vif_config::file(f, "")),
    };
    let info = if greeting.info { lines() } else { Vec::new() };
    let (cols, _, cell) = gifplay::screen();
    let widest = info.iter().map(|(_, w)| *w).max().unwrap_or(0);
    let room = if info.is_empty() {
        cols.saturating_sub(1)
    } else {
        cols.saturating_sub(widest + GAP + 1)
    };
    let mut failed = false;
    let clip = file.and_then(|path| {
        let rows = usize::from(greeting.height);
        gifplay::load(&path, greeting.blocks, false, room.max(4), rows, cell)
            .inspect_err(|err| {
                eprintln!("vif greet: {}: {err}", path.display());
                failed = true;
            })
            .ok()
    });
    if clip.is_none() && info.is_empty() {
        return if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS };
    }
    show(clip.as_ref(), &info, greeting.loops);
    ExitCode::SUCCESS
}

/// The lines beside the picture, each with how many cells it takes.
fn lines() -> Vec<(String, usize)> {
    let who = facts::who();
    let facts = facts::gather();
    let label = facts.iter().map(|(l, _)| l.len()).max().unwrap_or(0);
    let mut out = vec![
        (format!("\x1b[1;34m{who}\x1b[0m"), who.chars().count()),
        ("─".repeat(who.chars().count()), who.chars().count()),
    ];
    for (name, value) in facts {
        let width = label + 2 + value.chars().count();
        out.push((format!("\x1b[1;34m{name:<label$}\x1b[0m  {value}"), width));
    }
    out.push((String::new(), 0));
    for range in [0..8, 8..16] {
        let swatches: String = range.map(|i| format!("\x1b[48;5;{i}m   ")).collect();
        out.push((format!("{swatches}\x1b[0m"), 24));
    }
    out
}

extern "C" fn stop(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

/// Makes room under the cursor and plays the picture there, then leaves the cursor under it.
fn show(clip: Option<&Clip>, info: &[(String, usize)], loops: u8) {
    let picture = clip.map_or((0, 0), |c| (c.cols, c.rows));
    let rows = picture.1.max(info.len());
    let left = 2;
    let beside = if picture.0 == 0 { left } else { left + picture.0 + GAP };
    // SAFETY: the handler only stores to an atomic, which a signal handler may do.
    unsafe {
        libc::signal(libc::SIGINT, stop as *const () as libc::sighandler_t);
    }
    // Keys typed meanwhile wait for the prompt instead of landing on the picture.
    let modes = crate::tty::Modes::change(|t| t.c_lflag &= !libc::ECHO);
    let mut out = io::stdout().lock();
    let at = |line: usize, col: usize| {
        let down = if line > 0 {
            format!("\x1b[{line}B")
        } else {
            String::new()
        };
        format!("\x1b8{down}\x1b[{col}G")
    };
    let mut buf = format!("\r{}\x1b[{rows}A\x1b7\x1b[?25l", "\n".repeat(rows));
    let top = rows.saturating_sub(info.len()) / 2;
    for (i, (line, _)) in info.iter().enumerate() {
        buf.push_str(&at(top + i, beside));
        buf.push_str(line);
    }
    let _ = out.write_all(buf.as_bytes()).and_then(|()| out.flush());
    if let Some(clip) = clip {
        play(&mut out, clip, (rows.saturating_sub(clip.rows) / 2, left), loops, &at);
    }
    let _ = write!(out, "\x1b[0m\x1b8\x1b[{rows}B\r\n\x1b[?25h");
    let _ = out.flush();
    drop(modes);
}

/// Writes the frames that differ from what's there, in the GIF's own time.
fn play(
    out: &mut impl Write,
    clip: &Clip,
    (top, left): (usize, usize),
    loops: u8,
    at: &dyn Fn(usize, usize) -> String,
) {
    let mut shown: Vec<&str> = vec![""; clip.rows];
    let started = Instant::now();
    let mut next = started;
    let still = clip.frames.len() == 1;
    for _ in 0..if still { 1 } else { loops } {
        for frame in &clip.frames {
            let mut buf = String::from("\x1b[?2026h");
            for (i, line) in frame.lines.iter().enumerate() {
                if shown[i] != line.as_str() {
                    buf.push_str(&at(top + i, left));
                    buf.push_str(line);
                    shown[i] = line.as_str();
                }
            }
            buf.push_str("\x1b[?2026l");
            if out.write_all(buf.as_bytes()).and_then(|()| out.flush()).is_err() || still {
                return;
            }
            next += frame.delay;
            while Instant::now() < next {
                if STOP.load(Ordering::Relaxed) || started.elapsed() > LONGEST {
                    return;
                }
                std::thread::sleep((next - Instant::now()).min(Duration::from_millis(50)));
            }
        }
    }
}
