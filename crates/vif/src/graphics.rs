//! The kitty graphics protocol: https://sw.kovidgoyal.net/kitty/graphics-protocol/
//!
//! Commands are taken out of the PTY stream before the VT parser sees it. A shown image
//! becomes a block of placeholder characters written at the cursor, so the grid itself
//! scrolls, reflows and clears images the way it does text; the renderer draws each
//! placeholder cell as its slice of the picture.
//!
//! The same filter takes shell integration marks (OSC 133) out; see [`crate::marks`].

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::tty::{ChildEvent, EventedPty, EventedReadWrite, Pty};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use crate::marks::{Blocks, Marker};
use crate::session::Link;

/// The character every image cell holds; the foreground color names the placement.
pub const PLACEHOLDER: char = '\u{10EEEE}';
const DIGITS: u32 = 112;
const LIMIT: usize = 320 << 20;
const OSC_MARK: &[u8] = b"133;";

pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// An image shown somewhere, `cols` × `rows` cells big.
#[derive(Clone)]
pub struct Placement {
    pub image: Arc<Image>,
    pub cols: u32,
    pub rows: u32,
    /// Fit to the cells rather than shown at its own size.
    pub scaled: bool,
}

#[derive(Default)]
pub struct Shared {
    pub placements: Mutex<HashMap<u32, Placement>>,
    pub cell_width: AtomicU32,
    pub cell_height: AtomicU32,
    pub columns: AtomicU32,
}

/// Row and column of a placeholder cell from its combining marks.
pub fn decode(marks: &[char]) -> Option<(u32, u32)> {
    let digit = |c: &char| (*c as u32).checked_sub(0x300).filter(|d| *d < DIGITS);
    // A shell mark may sit on the cell too, ahead of the image's own marks.
    let marks: Vec<char> = marks.iter().copied().filter(|c| !crate::marks::is_tag(*c)).collect();
    match marks.as_slice() {
        [a, b, c, d, ..] => Some((digit(a)? * DIGITS + digit(b)?, digit(c)? * DIGITS + digit(d)?)),
        _ => None,
    }
}

fn encode(n: u32, out: &mut String) {
    out.push(char::from_u32(0x300 + n / DIGITS).unwrap_or('\u{300}'));
    out.push(char::from_u32(0x300 + n % DIGITS).unwrap_or('\u{300}'));
}

enum Scan {
    Text,
    Escape,
    Command(Vec<u8>),
    CommandEscape(Vec<u8>),
    OscHead(Vec<u8>),
    Mark(Vec<u8>),
    MarkEscape(Vec<u8>),
}

struct Transfer {
    keys: HashMap<u8, String>,
    data: Vec<u8>,
}

/// Reads the PTY and keeps graphics commands away from the VT parser.
pub struct Filter {
    file: File,
    reply: File,
    shared: Arc<Shared>,
    queue: VecDeque<u8>,
    scan: Scan,
    images: HashMap<u32, Arc<Image>>,
    transfer: Option<Transfer>,
    next: u32,
    order: VecDeque<u32>,
    marker: Marker,
}

impl Filter {
    pub fn new(file: File, reply: File, shared: Arc<Shared>, blocks: Arc<Blocks>) -> Self {
        Self {
            file,
            reply,
            shared,
            queue: VecDeque::new(),
            scan: Scan::Text,
            images: HashMap::new(),
            transfer: None,
            next: 1,
            order: VecDeque::new(),
            marker: Marker::new(blocks),
        }
    }

    fn out(&mut self, bytes: &[u8]) {
        self.marker.emit(bytes, &mut self.queue);
    }

    /// Passes `input` on, taking out `ESC _ G … ESC \` and putting placeholders where images show.
    fn feed(&mut self, input: &[u8]) {
        let mut i = 0;
        while i < input.len() {
            match &mut self.scan {
                Scan::Text => match memchr::memchr(0x1b, &input[i..]) {
                    Some(at) => {
                        self.out(&input[i..i + at]);
                        i += at + 1;
                        self.scan = Scan::Escape;
                    }
                    None => {
                        self.out(&input[i..]);
                        i = input.len();
                    }
                },
                Scan::Escape => {
                    match input[i] {
                        b'_' => self.scan = Scan::Command(Vec::new()),
                        b']' => self.scan = Scan::OscHead(Vec::new()),
                        b => {
                            self.scan = Scan::Text;
                            self.out(&[0x1b, b]);
                        }
                    }
                    i += 1;
                }
                Scan::OscHead(head) => {
                    head.push(input[i]);
                    i += 1;
                    if head.as_slice() == OSC_MARK {
                        self.scan = Scan::Mark(Vec::new());
                    } else if !OSC_MARK.starts_with(head) {
                        let mut bytes = b"\x1b]".to_vec();
                        bytes.append(head);
                        self.scan = Scan::Text;
                        self.out(&bytes);
                    }
                }
                Scan::Mark(body) => match memchr::memchr2(0x07, 0x1b, &input[i..]) {
                    Some(at) => {
                        body.extend_from_slice(&input[i..i + at]);
                        let body = std::mem::take(body);
                        let bell = input[i + at] == 0x07;
                        i += at + 1;
                        if bell {
                            self.scan = Scan::Text;
                            self.marker.mark(&body, &mut self.queue);
                        } else {
                            self.scan = Scan::MarkEscape(body);
                        }
                    }
                    None => {
                        if body.len() < 1024 {
                            body.extend_from_slice(&input[i..]);
                        }
                        i = input.len();
                    }
                },
                Scan::MarkEscape(body) => {
                    let body = std::mem::take(body);
                    self.marker.mark(&body, &mut self.queue);
                    if input[i] == b'\\' {
                        self.scan = Scan::Text;
                        i += 1;
                    } else {
                        self.scan = Scan::Escape;
                    }
                }
                Scan::Command(body) => match memchr::memchr(0x1b, &input[i..]) {
                    Some(at) => {
                        body.extend_from_slice(&input[i..i + at]);
                        i += at + 1;
                        let body = std::mem::take(body);
                        self.scan = Scan::CommandEscape(body);
                    }
                    None => {
                        body.extend_from_slice(&input[i..]);
                        i = input.len();
                    }
                },
                Scan::CommandEscape(body) => {
                    let mut body = std::mem::take(body);
                    if input[i] == b'\\' {
                        self.scan = Scan::Text;
                        self.command(&body);
                    } else {
                        body.push(0x1b);
                        body.push(input[i]);
                        self.scan = Scan::Command(body);
                    }
                    i += 1;
                }
            }
        }
    }

    fn command(&mut self, body: &[u8]) {
        let Some(rest) = body.strip_prefix(b"G") else {
            // Some other APC: hand it on untouched.
            let mut apc = b"\x1b_".to_vec();
            apc.extend_from_slice(body);
            apc.extend_from_slice(b"\x1b\\");
            self.out(&apc);
            return;
        };
        let (head, payload) = match rest.iter().position(|b| *b == b';') {
            Some(at) => (&rest[..at], &rest[at + 1..]),
            None => (rest, &rest[rest.len()..]),
        };
        let mut keys: HashMap<u8, String> = HashMap::new();
        for pair in head.split(|b| *b == b',') {
            if let [key, b'=', value @ ..] = pair {
                keys.insert(*key, String::from_utf8_lossy(value).into_owned());
            }
        }
        let more = keys.get(&b'm').is_some_and(|m| m == "1");
        let data = STANDARD.decode(
            payload
                .iter()
                .copied()
                .filter(|b| !b.is_ascii_whitespace())
                .collect::<Vec<_>>(),
        );
        let data = data.unwrap_or_default();
        // Later chunks carry only m=; the first chunk's keys rule.
        let transfer = match self.transfer.take() {
            Some(mut transfer) => {
                transfer.data.extend(data);
                transfer
            }
            None => Transfer { keys, data },
        };
        if more {
            self.transfer = Some(transfer);
            return;
        }
        let keys = transfer.keys;
        let result = self.run(&keys, transfer.data);
        self.answer(&keys, result);
    }

    fn run(&mut self, keys: &HashMap<u8, String>, data: Vec<u8>) -> Result<(), String> {
        let action = keys.get(&b'a').map_or("t", String::as_str);
        let number = |k: u8| keys.get(&k).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
        let id = number(b'i');
        match action {
            "t" | "T" | "q" => {
                let image = Arc::new(self.load(keys, data)?);
                if action == "q" {
                    return Ok(());
                }
                if id != 0 {
                    self.images.insert(id, image.clone());
                }
                if action == "T" {
                    self.show(image, keys);
                }
                Ok(())
            }
            "p" => {
                let image = self.images.get(&id).cloned().ok_or("ENOENT:no such image")?;
                self.show(image, keys);
                Ok(())
            }
            "d" => {
                match keys.get(&b'd').map_or("a", String::as_str) {
                    "i" | "I" if id != 0 => {
                        self.images.remove(&id);
                    }
                    "a" | "A" => {
                        self.shared.placements.lock().expect("placements").clear();
                        self.order.clear();
                    }
                    _ => {}
                }
                Ok(())
            }
            _ => Err("EINVAL:unsupported action".into()),
        }
    }

    fn load(&self, keys: &HashMap<u8, String>, data: Vec<u8>) -> Result<Image, String> {
        let number = |k: u8| keys.get(&k).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
        let data = match keys.get(&b't').map_or("d", String::as_str) {
            "d" => data,
            medium @ ("f" | "t") => {
                let path = String::from_utf8(data).map_err(|_| "EINVAL:bad path")?;
                if medium == "t" && !path.contains("tty-graphics-protocol") {
                    return Err("EPERM:not a temporary file".into());
                }
                let bytes = std::fs::read(&path).map_err(|e| format!("EBADF:{e}"))?;
                if medium == "t" {
                    let _ = std::fs::remove_file(&path);
                }
                bytes
            }
            _ => return Err("EINVAL:unsupported medium".into()),
        };
        let data = if keys.get(&b'o').is_some_and(|o| o == "z") {
            let mut out = Vec::new();
            flate2::read::ZlibDecoder::new(&data[..])
                .read_to_end(&mut out)
                .map_err(|_| "EINVAL:bad zlib data")?;
            out
        } else {
            data
        };
        match number(b'f') {
            100 => png(&data),
            24 | 32 | 0 => {
                let (width, height) = (number(b's'), number(b'v'));
                let channels = if number(b'f') == 24 { 3 } else { 4 };
                let size = (width as usize) * (height as usize) * channels;
                if width == 0 || height == 0 || data.len() < size {
                    return Err("EINVAL:wrong size".into());
                }
                let rgba = if channels == 4 {
                    data[..size].to_vec()
                } else {
                    data[..size].chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect()
                };
                Ok(Image { width, height, rgba })
            }
            _ => Err("EINVAL:unsupported format".into()),
        }
    }

    fn show(&mut self, image: Arc<Image>, keys: &HashMap<u8, String>) {
        let number = |k: u8| keys.get(&k).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
        let cw = self.shared.cell_width.load(Ordering::Relaxed).max(1);
        let ch = self.shared.cell_height.load(Ordering::Relaxed).max(1);
        let columns = self.shared.columns.load(Ordering::Relaxed).max(1);
        let (mut cols, mut rows) = (number(b'c'), number(b'r'));
        let mut scaled = cols != 0 || rows != 0;
        match (cols, rows) {
            (0, 0) => {
                cols = image.width.div_ceil(cw);
                rows = image.height.div_ceil(ch);
            }
            (0, r) => cols = (image.width * r * ch).div_ceil(image.height.max(1) * cw),
            (c, 0) => rows = (image.height * c * cw).div_ceil(image.width.max(1) * ch),
            _ => {}
        }
        if cols > columns {
            rows = (rows * columns).div_ceil(cols).max(1);
            cols = columns;
            scaled = true;
        }
        let (cols, rows) = (cols.clamp(1, DIGITS * DIGITS - 1), rows.clamp(1, DIGITS * DIGITS - 1));
        let id = self.next;
        self.next = if self.next >= 0xff_ffff { 1 } else { self.next + 1 };
        self.remember(
            id,
            Placement {
                image,
                cols,
                rows,
                scaled,
            },
        );

        let still = keys.get(&b'C').is_some_and(|c| c == "1");
        let mut out = String::new();
        if still {
            out.push_str("\x1b7");
        }
        out.push_str(&format!("\x1b[38;2;{};{};{}m", id >> 16, (id >> 8) & 0xff, id & 0xff));
        for row in 0..rows {
            for col in 0..cols {
                out.push(PLACEHOLDER);
                encode(row, &mut out);
                encode(col, &mut out);
            }
            if row + 1 < rows {
                out.push_str(&format!("\x1b[{cols}D\n"));
            }
        }
        out.push_str("\x1b[39m");
        if still {
            out.push_str("\x1b8");
        }
        self.out(out.as_bytes());
    }

    /// Keeps placements under a memory budget, dropping the oldest first.
    fn remember(&mut self, id: u32, placement: Placement) {
        let mut placements = self.shared.placements.lock().expect("placements");
        placements.insert(id, placement);
        self.order.push_back(id);
        let mut total: usize = placements.values().map(|p| p.image.rgba.len()).sum();
        while total > LIMIT {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            if let Some(p) = placements.remove(&old) {
                total -= p.image.rgba.len();
            }
        }
    }

    fn answer(&mut self, keys: &HashMap<u8, String>, result: Result<(), String>) {
        let quiet = keys.get(&b'q').and_then(|q| q.parse::<u8>().ok()).unwrap_or(0);
        let id = keys.get(&b'i').filter(|i| *i != "0");
        if id.is_none() && keys.get(&b'a').is_none_or(|a| a != "q") {
            return;
        }
        let message = match &result {
            Ok(()) if quiet >= 1 => return,
            Err(_) if quiet >= 2 => return,
            Ok(()) => "OK".to_owned(),
            Err(e) => e.clone(),
        };
        let mut reply = String::from("\x1b_G");
        if let Some(id) = id {
            reply.push_str(&format!("i={id}"));
        }
        if let Some(p) = keys.get(&b'p') {
            reply.push_str(&format!(",p={p}"));
        }
        reply.push(';');
        reply.push_str(&message);
        reply.push_str("\x1b\\");
        let _ = self.reply.write_all(reply.as_bytes());
    }
}

fn png(data: &[u8]) -> Result<Image, String> {
    let mut decoder = png::Decoder::new(io::Cursor::new(data));
    decoder.set_transformations(
        png::Transformations::EXPAND | png::Transformations::STRIP_16 | png::Transformations::ALPHA,
    );
    let mut reader = decoder.read_info().map_err(|e| format!("EBADPNG:{e}"))?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("EBADPNG:too big")?];
    let info = reader.next_frame(&mut buf).map_err(|e| format!("EBADPNG:{e}"))?;
    let pixels = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::Rgb => pixels.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => pixels.chunks(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => pixels.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
        png::ColorType::Indexed => return Err("EBADPNG:palette not expanded".into()),
    };
    Ok(Image {
        width: info.width,
        height: info.height,
        rgba,
    })
}

impl Read for Filter {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if !self.queue.is_empty() {
                return self.queue.read(buf);
            }
            let n = self.file.read(buf)?;
            if n == 0 {
                return Ok(0);
            }
            // Output with no APC or mark in it, the usual case, goes straight through.
            let plain = matches!(self.scan, Scan::Text)
                && self.marker.idle()
                && !open_tail(&buf[..n])
                && memchr::memmem::find(&buf[..n], b"\x1b_").is_none()
                && memchr::memmem::find(&buf[..n], b"\x1b]133;").is_none();
            if plain {
                return Ok(n);
            }
            let chunk = buf[..n].to_vec();
            self.feed(&chunk);
        }
    }
}

/// Whether the chunk ends partway into something the filter has to see whole.
fn open_tail(buf: &[u8]) -> bool {
    let from = buf.len().saturating_sub(5);
    buf[from..]
        .iter()
        .rposition(|&b| b == 0x1b)
        .is_some_and(|at| b"\x1b]133;".starts_with(&buf[from + at..]))
}

/// The real PTY with its output read through a [`Filter`].
pub struct GraphicsPty {
    backend: Backend,
    filter: Filter,
}

/// A PTY of this window's own, or a session host's.
enum Backend {
    Local(Pty),
    Session(Link),
}

impl GraphicsPty {
    pub fn new(pty: Pty, shared: Arc<Shared>, blocks: Arc<Blocks>) -> io::Result<Self> {
        let filter = Filter::new(pty.file().try_clone()?, pty.file().try_clone()?, shared, blocks);
        Ok(Self {
            backend: Backend::Local(pty),
            filter,
        })
    }

    pub fn session(link: Link, shared: Arc<Shared>, blocks: Arc<Blocks>) -> io::Result<Self> {
        let filter = Filter::new(link.bytes()?, link.bytes()?, shared, blocks);
        Ok(Self {
            backend: Backend::Session(link),
            filter,
        })
    }
}

impl EventedReadWrite for GraphicsPty {
    type Reader = Filter;
    type Writer = File;

    unsafe fn register(
        &mut self,
        poll: &Arc<polling::Poller>,
        event: polling::Event,
        mode: polling::PollMode,
    ) -> io::Result<()> {
        match &mut self.backend {
            Backend::Local(pty) => unsafe { pty.register(poll, event, mode) },
            Backend::Session(link) => unsafe { link.register(poll, event, mode) },
        }
    }

    fn reregister(
        &mut self,
        poll: &Arc<polling::Poller>,
        event: polling::Event,
        mode: polling::PollMode,
    ) -> io::Result<()> {
        match &mut self.backend {
            Backend::Local(pty) => pty.reregister(poll, event, mode),
            Backend::Session(link) => link.reregister(poll, event, mode),
        }
    }

    fn deregister(&mut self, poll: &Arc<polling::Poller>) -> io::Result<()> {
        match &mut self.backend {
            Backend::Local(pty) => pty.deregister(poll),
            Backend::Session(link) => link.deregister(poll),
        }
    }

    fn reader(&mut self) -> &mut Filter {
        &mut self.filter
    }

    fn writer(&mut self) -> &mut File {
        match &mut self.backend {
            Backend::Local(pty) => pty.writer(),
            Backend::Session(link) => link.writer(),
        }
    }
}

impl EventedPty for GraphicsPty {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        match &mut self.backend {
            Backend::Local(pty) => pty.next_child_event(),
            Backend::Session(link) => link.next_child_event(),
        }
    }
}

impl OnResize for GraphicsPty {
    fn on_resize(&mut self, size: WindowSize) {
        match &mut self.backend {
            Backend::Local(pty) => pty.on_resize(size),
            Backend::Session(link) => link.on_resize(size),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(name: &str) -> (Filter, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("vif-graphics-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        let file = File::open("/dev/null").unwrap();
        let reply = File::create(&path).unwrap();
        let shared = Arc::new(Shared::default());
        shared.cell_width.store(10, Ordering::Relaxed);
        shared.cell_height.store(20, Ordering::Relaxed);
        shared.columns.store(80, Ordering::Relaxed);
        (Filter::new(file, reply, shared, Arc::new(Blocks::default())), path)
    }

    fn drained(f: &mut Filter) -> String {
        String::from_utf8(f.queue.drain(..).collect()).unwrap()
    }

    #[test]
    fn text_passes_and_images_become_placeholders() {
        let (mut f, reply) = filter("placeholders");
        let pixels = STANDARD.encode([255u8, 0, 0, 255].repeat(25 * 30));
        let cmd = format!("ab\x1b[1mc\x1b_Ga=T,f=32,s=25,v=30,i=7;{pixels}\x1b\\de");
        // Split anywhere: the scanner keeps its state between reads.
        let (one, two) = cmd.as_bytes().split_at(13);
        f.feed(one);
        f.feed(two);
        let out = drained(&mut f);
        assert!(out.starts_with("ab\x1b[1mc\x1b[38;2;0;0;1m"));
        assert!(out.ends_with("\x1b[39mde"));
        assert_eq!(out.matches(PLACEHOLDER).count(), 3 * 2);
        let placements = f.shared.placements.lock().unwrap();
        let p = placements.get(&1).unwrap();
        assert_eq!((p.cols, p.rows, p.image.width), (3, 2, 25));
        assert_eq!(std::fs::read_to_string(reply).unwrap(), "\x1b_Gi=7;OK\x1b\\");
    }

    #[test]
    fn chunks_queries_and_other_apcs() {
        let (mut f, reply) = filter("chunks");
        let pixels = STANDARD.encode([0u8, 255, 0].repeat(4));
        let (a, b) = pixels.split_at(8);
        f.feed(format!("\x1b_Ga=t,f=24,s=2,v=2,i=3,m=1;{a}\x1b\\\x1b_Gm=0;{b}\x1b\\").as_bytes());
        assert!(f.images.contains_key(&3));
        f.feed(b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\");
        f.feed(b"\x1b_Xother\x1b\\");
        assert_eq!(drained(&mut f), "\x1b_Xother\x1b\\");
        let replies = std::fs::read_to_string(reply).unwrap();
        assert_eq!(replies, "\x1b_Gi=3;OK\x1b\\\x1b_Gi=31;OK\x1b\\");
    }

    #[test]
    fn marks_become_tags_and_other_oscs_pass() {
        let (mut f, _) = filter("marks");
        f.feed(b"\x1b]0;title\x07\x1b]13");
        f.feed(b"3;A\x07$ \x1b]133;B\x1b\\ls");
        let out: String = drained(&mut f)
            .chars()
            .map(|c| if crate::marks::is_tag(c) { '#' } else { c })
            .collect();
        assert_eq!(out, "\x1b]0;title\x07$## ##ls");
        assert!(open_tail(b"text\x1b]13"));
        assert!(!open_tail(b"text\x1b[0m"));
    }

    #[test]
    fn placeholders_round_trip() {
        let mut s = String::new();
        encode(130, &mut s);
        encode(5, &mut s);
        let marks: Vec<char> = s.chars().collect();
        assert_eq!(decode(&marks), Some((130, 5)));
    }
}
