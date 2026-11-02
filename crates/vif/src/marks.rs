//! Shell integration marks (OSC 133): where prompts, commands and their output are.
//!
//! The PTY filter takes `ESC ] 133 ; X … ST` out of the stream and leaves a tag in its place.
//! Tag characters are zero-width, so the grid keeps them on a cell through scrolling and
//! reflow. A prompt or output tag waits for the next printed character: a cell written after
//! the tag landed would drop it.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

const TAG: u32 = 0xE0000;
const KEEP: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Prompt,
    Command,
    Output,
}

impl Kind {
    fn letter(self) -> u32 {
        match self {
            Kind::Prompt => 'a' as u32,
            Kind::Command => 'b' as u32,
            Kind::Output => 'c' as u32,
        }
    }
}

pub fn is_tag(c: char) -> bool {
    (TAG..=TAG + 0x7F).contains(&(c as u32))
}

fn encode(kind: Kind, id: u32) -> String {
    let mut out = String::new();
    out.push(char::from_u32(TAG + kind.letter()).expect("tag"));
    for digit in id.to_string().bytes() {
        out.push(char::from_u32(TAG + u32::from(digit)).expect("tag"));
    }
    out
}

/// The id of a `kind` mark among a cell's zero-width characters.
pub fn find(zerowidth: &[char], kind: Kind) -> Option<u32> {
    let start = zerowidth.iter().position(|&c| c as u32 == TAG + kind.letter())?;
    let digits: String = zerowidth[start + 1..]
        .iter()
        .map_while(|&c| char::from_u32((c as u32).checked_sub(TAG)?).filter(char::is_ascii_digit))
        .collect();
    digits.parse().ok()
}

#[derive(Clone, Copy, Debug)]
pub struct Block {
    pub started: Instant,
    /// When it started by the wall clock.
    pub at: SystemTime,
    pub ended: Option<(Duration, i32)>,
}

impl Block {
    pub fn running(&self) -> Duration {
        self.ended.map_or_else(|| self.started.elapsed(), |(took, _)| took)
    }

    /// When it ended, or now while it runs.
    pub fn finished_at(&self) -> SystemTime {
        self.at + self.running()
    }
}

/// What the shell ran, shared between the PTY thread and the window.
#[derive(Default)]
pub struct Blocks {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    map: HashMap<u32, Block>,
    done: Vec<u32>,
    last: Option<u32>,
}

impl Blocks {
    pub fn get(&self, id: u32) -> Option<Block> {
        self.inner.lock().expect("blocks").map.get(&id).copied()
    }

    /// The newest command that started.
    pub fn last(&self) -> Option<u32> {
        self.inner.lock().expect("blocks").last
    }

    /// Blocks that finished since the last call.
    pub fn take_done(&self) -> Vec<u32> {
        std::mem::take(&mut self.inner.lock().expect("blocks").done)
    }

    fn start(&self, id: u32) {
        let mut inner = self.inner.lock().expect("blocks");
        inner.last = Some(id);
        inner.map.insert(
            id,
            Block {
                started: Instant::now(),
                at: SystemTime::now(),
                ended: None,
            },
        );
        if inner.map.len() > KEEP {
            let oldest = inner.map.keys().min().copied();
            inner.map.remove(&oldest.unwrap_or(id));
        }
    }

    fn end(&self, id: u32, exit: i32) {
        let mut inner = self.inner.lock().expect("blocks");
        if let Some(block) = inner.map.get_mut(&id)
            && block.ended.is_none()
        {
            block.ended = Some((block.started.elapsed(), exit));
            inner.done.push(id);
        }
    }
}

/// A duration the way a badge shows it: 0.02s, 41.2s, 2:41, 1:02:41.
pub fn clock(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0 => format!("{:.2}s", d.as_secs_f32()),
        1..60 => format!("{:.1}s", d.as_secs_f32()),
        60..3600 => format!("{}:{:02}", s / 60, s % 60),
        _ => format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60),
    }
}

/// A duration in words: 41s, 2m 41s, 1h 2m.
pub fn spoken(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m {}s", s / 60, s % 60),
        _ => format!("{}h {}m", s / 3600, s / 60 % 60),
    }
}

/// Hours and minutes on this machine's clock: 16:39.
pub fn time_of_day(at: SystemTime) -> String {
    let secs = at.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&secs, &mut tm) }.is_null() {
        return String::new();
    }
    format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
}

/// Where a byte stream stands, enough to tell when a character gets printed.
#[derive(Clone, Copy, Default)]
enum Vt {
    #[default]
    Ground,
    Utf8(u8),
    Esc,
    EscInter,
    Csi,
    Str,
    StrEsc,
}

impl Vt {
    /// Takes one byte; true when it finished a printed character.
    fn step(&mut self, b: u8) -> bool {
        let (next, printed) = match (*self, b) {
            (Vt::Ground, 0x1b) => (Vt::Esc, false),
            (Vt::Ground, 0x20..=0x7e) => (Vt::Ground, true),
            (Vt::Ground, 0xc0..=0xdf) => (Vt::Utf8(1), false),
            (Vt::Ground, 0xe0..=0xef) => (Vt::Utf8(2), false),
            (Vt::Ground, 0xf0..=0xf7) => (Vt::Utf8(3), false),
            (Vt::Ground, _) => (Vt::Ground, false),
            (Vt::Utf8(n), 0x80..=0xbf) => (if n > 1 { Vt::Utf8(n - 1) } else { Vt::Ground }, n == 1),
            (Vt::Utf8(_), _) => (Vt::Ground, false),
            (Vt::Esc, b'[') => (Vt::Csi, false),
            (Vt::Esc, b']' | b'P' | b'_' | b'^' | b'X') => (Vt::Str, false),
            (Vt::Esc, b' '..=b'/') => (Vt::EscInter, false),
            (Vt::Esc | Vt::EscInter, _) => (Vt::Ground, false),
            (Vt::Csi, 0x40..=0x7e) => (Vt::Ground, false),
            (Vt::Csi, _) => (Vt::Csi, false),
            (Vt::Str, 0x07) => (Vt::Ground, false),
            (Vt::Str, 0x1b) => (Vt::StrEsc, false),
            (Vt::Str, _) => (Vt::Str, false),
            (Vt::StrEsc, b'\\') => (Vt::Ground, false),
            (Vt::StrEsc, _) => (Vt::Str, false),
        };
        *self = next;
        printed
    }
}

/// The PTY thread's half: turns marks into tags and times what runs.
pub struct Marker {
    blocks: std::sync::Arc<Blocks>,
    next: u32,
    current: u32,
    running: bool,
    pending: Option<(Kind, String)>,
    vt: Vt,
}

impl Marker {
    pub fn new(blocks: std::sync::Arc<Blocks>) -> Self {
        Self {
            blocks,
            next: 1,
            current: 0,
            running: false,
            pending: None,
            vt: Vt::Ground,
        }
    }

    /// Nothing waits for a printed character, so output may go straight through.
    pub fn idle(&self) -> bool {
        self.pending.is_none()
    }

    /// Takes the part of an OSC 133 after `133;`, putting a tag into `out` where one goes now.
    pub fn mark(&mut self, body: &[u8], out: &mut VecDeque<u8>) {
        let mut parts = body.split(|&b| b == b';');
        match parts.next() {
            Some(b"A") => {
                self.current = self.next;
                self.next += 1;
                self.wait(Kind::Prompt);
            }
            Some(b"B") => {
                if self.current == 0 {
                    self.current = self.next;
                    self.next += 1;
                }
                out.extend(encode(Kind::Command, self.current).as_bytes());
            }
            Some(b"C") if self.current != 0 => {
                self.blocks.start(self.current);
                self.running = true;
                self.wait(Kind::Output);
            }
            Some(b"D") => {
                if self.pending.as_ref().is_some_and(|(kind, _)| *kind == Kind::Output) {
                    self.pending = None;
                }
                if self.running {
                    let exit = parts
                        .next()
                        .and_then(|code| std::str::from_utf8(code).ok()?.parse().ok())
                        .unwrap_or(0);
                    self.blocks.end(self.current, exit);
                    self.running = false;
                }
            }
            _ => {}
        }
    }

    fn wait(&mut self, kind: Kind) {
        self.pending = Some((kind, encode(kind, self.current)));
        self.vt = Vt::Ground;
    }

    /// Copies output on, putting a waiting tag right after the first printed character.
    pub fn emit(&mut self, bytes: &[u8], out: &mut VecDeque<u8>) {
        let Some((_, tag)) = &self.pending else {
            out.extend(bytes);
            return;
        };
        for (i, &b) in bytes.iter().enumerate() {
            if self.vt.step(b) {
                out.extend(&bytes[..=i]);
                out.extend(tag.as_bytes());
                out.extend(&bytes[i + 1..]);
                self.pending = None;
                return;
            }
        }
        out.extend(bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn run(marker: &mut Marker, steps: &[(&str, &str)]) -> String {
        let mut out = VecDeque::new();
        for (mark, text) in steps {
            if !mark.is_empty() {
                marker.mark(mark.as_bytes(), &mut out);
            }
            marker.emit(text.as_bytes(), &mut out);
        }
        let bytes: Vec<u8> = out.into_iter().collect();
        String::from_utf8(bytes).unwrap()
    }

    fn shown(s: &str) -> String {
        s.chars()
            .map(|c| {
                if is_tag(c) {
                    char::from_u32(c as u32 - TAG).unwrap().to_ascii_uppercase()
                } else {
                    c
                }
            })
            .collect()
    }

    #[test]
    fn tags_follow_the_first_printed_character() {
        let blocks = Arc::new(Blocks::default());
        let mut marker = Marker::new(blocks.clone());
        let out = run(
            &mut marker,
            &[
                ("A", "\x1b[1;34m~\x1b[0m $ "),
                ("B", "ls\r\n"),
                ("C", "\x1b[0mfile\r\n"),
                ("D;2", ""),
                ("A", "~ $ "),
            ],
        );
        assert_eq!(shown(&out), "\x1b[1;34m~A1\x1b[0m $ B1ls\r\n\x1b[0mfC1ile\r\n~A2 $ ");
        let block = blocks.get(1).unwrap();
        assert_eq!(block.ended.map(|e| e.1), Some(2));
        assert_eq!(blocks.take_done(), vec![1]);
    }

    #[test]
    fn a_command_with_no_output_leaves_no_output_tag() {
        let mut marker = Marker::new(Arc::new(Blocks::default()));
        let out = run(
            &mut marker,
            &[("A", "$ "), ("B", "cd /\r\n"), ("C", ""), ("D;0", ""), ("A", "$ ")],
        );
        assert_eq!(shown(&out), "$A1 B1cd /\r\n$A2 ");
    }

    #[test]
    fn finds_marks_among_other_marks() {
        let mut zw: Vec<char> = "\u{301}".chars().collect();
        zw.extend(encode(Kind::Command, 42).chars());
        zw.extend(encode(Kind::Prompt, 7).chars());
        assert_eq!(find(&zw, Kind::Command), Some(42));
        assert_eq!(find(&zw, Kind::Prompt), Some(7));
        assert_eq!(find(&zw, Kind::Output), None);
    }
}
