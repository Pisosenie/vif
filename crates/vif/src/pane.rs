use std::borrow::Cow;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Grid, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::term::search::{Match, RegexSearch};
use alacritty_terminal::term::{self, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{CursorShape, CursorStyle};
use vif_config::{Badges, Config, GreetingShow, Osc52};
use vif_render::{Metrics, Scene};
use winit::event_loop::EventLoopProxy;
use winit::window::WindowId;

use crate::app::UserEvent;
use crate::colors::Scheme;
use crate::fold::Shown;
use crate::graphics;
use crate::hints::{self, Hints};
use crate::layout::{Area, PaneId};
use crate::marks::{self, Blocks, Kind};
use crate::rules::Place;
use crate::session::{self, Link};

mod draw;
mod search;

#[derive(Clone)]
pub struct Proxy {
    proxy: EventLoopProxy<UserEvent>,
    window: WindowId,
    pane: PaneId,
}

impl EventListener for Proxy {
    fn send_event(&self, event: Event) {
        let _ = self.proxy.send_event(UserEvent::Term {
            window: self.window,
            pane: self.pane,
            event,
        });
    }
}

#[derive(Clone, Copy)]
struct Size {
    cols: usize,
    lines: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.cols
    }
}

/// How the pane should look this frame.
pub struct Look<'a> {
    pub scheme: &'a Scheme,
    pub focused: bool,
    pub cursor_on: bool,
    pub link: Option<&'a str>,
    pub badges: &'a Badges,
}

pub struct Spawn<'a> {
    pub config: &'a Config,
    pub cwd: Option<PathBuf>,
    pub term: &'a (String, Option<PathBuf>),
    pub command: Option<&'a [String]>,
    pub proxy: EventLoopProxy<UserEvent>,
    pub window: WindowId,
    /// A session left by a closed window, to take over instead of starting a shell.
    pub attach: Option<String>,
    /// The first shell of its window, which plays the greeting if there is one.
    pub first: bool,
    /// What the shell this one replaces printed last, shown above it.
    pub before: Option<Vec<u8>>,
}

/// A built row: what it draws, where its image cells are, and the command typed on it.
#[derive(Clone)]
struct Row {
    scene: Scene,
    marks: Vec<Mark>,
    block: Option<u32>,
    /// Columns up to the last one with text in it.
    end: usize,
}

/// The first cell of an image on a row: which image, which of its cells, and where it sits.
#[derive(Clone, Copy)]
struct Mark {
    id: u32,
    row: u32,
    col: u32,
    x: f32,
    y: f32,
}

impl Mark {
    fn read(cell: &Cell, x: f32, y: f32) -> Option<Self> {
        let alacritty_terminal::vte::ansi::Color::Spec(c) = cell.fg else {
            return None;
        };
        let id = u32::from(c.r) << 16 | u32::from(c.g) << 8 | u32::from(c.b);
        let (row, col) = cell.zerowidth().and_then(graphics::decode)?;
        Some(Self { id, row, col, x, y })
    }
}

/// The shell's terminal: the PTY this pane owns, or a session's, opened only when asked.
enum Tty {
    Own(Option<std::fs::File>),
    Path(PathBuf),
}

impl Tty {
    fn open(&self) -> Option<std::fs::File> {
        match self {
            Tty::Own(file) => file.as_ref()?.try_clone().ok(),
            Tty::Path(path) => session::open_tty(path),
        }
    }
}

/// How far a row's edge backgrounds reach past the grid, so app colors fill the padding.
#[derive(Clone, Copy)]
struct Bleed {
    left: f32,
    right: f32,
    top: Option<f32>,
    bottom: Option<f32>,
}

/// Find in the scrollback, newest first.
pub struct Search {
    pub query: String,
    regex: Option<RegexSearch>,
    current: Option<Match>,
    origin: Point,
    pub missed: bool,
}

pub struct Pane {
    pub term: Arc<FairMutex<Term<Proxy>>>,
    pub search: Option<Search>,
    pub hints: Option<Hints>,
    pub blocks: Arc<Blocks>,
    /// A running command's time shows on screen and wants a redraw every second.
    pub ticking: bool,
    id: PaneId,
    graphics: Arc<graphics::Shared>,
    sender: EventLoopSender,
    tty: Tty,
    shell: u32,
    pub area: Area,
    pub outer: Area,
    pub cols: usize,
    pub lines: usize,
    pub title: Option<String>,
    rows: Vec<Option<Row>>,
    generation: u64,
    cursor_line: Option<usize>,
    pub scroll: f32,
    pub selecting: bool,
    place_seen: Option<(Instant, Place)>,
    /// The rule whose look the pane has now.
    pub rule: Option<usize>,
    /// The session host's control socket, when the shell lives in one.
    control: Option<std::os::unix::net::UnixStream>,
    /// Until when finished commands are a session's replay, not news.
    quiet: Option<Instant>,
    /// The session the shell lives in.
    pub session: Option<String>,
    /// Blocks whose output shows as one line.
    pub(crate) folds: std::collections::HashSet<u32>,
    /// What each screen line showed last frame, while a fold is on screen.
    pub(crate) view: Option<Vec<Shown>>,
    /// The name its session was given.
    pub name: Option<String>,
}

pub fn term_config(config: &Config) -> term::Config {
    let shape = match config.cursor.shape {
        vif_config::CursorShape::Block => CursorShape::Block,
        vif_config::CursorShape::Beam => CursorShape::Beam,
        vif_config::CursorShape::Underline => CursorShape::Underline,
    };
    term::Config {
        scrolling_history: config.terminal.scrollback as usize,
        default_cursor_style: CursorStyle {
            shape,
            blinking: config.cursor.blink,
        },
        kitty_keyboard: config.terminal.kitty_keyboard,
        osc52: match config.terminal.osc52 {
            Osc52::Off => term::Osc52::Disabled,
            Osc52::Copy => term::Osc52::OnlyCopy,
            Osc52::Paste => term::Osc52::OnlyPaste,
            Osc52::Both => term::Osc52::CopyPaste,
        },
        ..term::Config::default()
    }
}

impl Pane {
    pub fn spawn(id: PaneId, spawn: Spawn, area: Area, m: Metrics) -> std::io::Result<Self> {
        let (cols, lines) = grid_size(area, m);
        let proxy = Proxy {
            proxy: spawn.proxy,
            window: spawn.window,
            pane: id,
        };
        let mut term = Term::new(term_config(spawn.config), &Size { cols, lines }, proxy.clone());
        if let Some(before) = &spawn.before {
            let mut parser: alacritty_terminal::vte::ansi::Processor = alacritty_terminal::vte::ansi::Processor::new();
            parser.advance(&mut term, before);
            parser.advance(
                &mut term,
                b"\x1b[0m\r\n\x1b[2m-- vif started again here; the shell above is gone --\x1b[0m\r\n",
            );
        }
        let term = Arc::new(FairMutex::new(term));

        let mut env = crate::shell::env(spawn.term);
        let greeting = &spawn.config.greeting;
        let greet = match greeting.show {
            GreetingShow::Off => false,
            GreetingShow::Window => spawn.first,
            GreetingShow::Always => true,
        };
        if greet
            && spawn.attach.is_none()
            && spawn.before.is_none()
            && spawn.command.is_none()
            && (greeting.file.is_some() || greeting.info)
            && let Ok(me) = std::env::current_exe()
        {
            env.insert("VIF_GREET".to_owned(), me.display().to_string());
        }
        let program = match spawn.command {
            Some([program, args @ ..]) => Some((program.clone(), args.to_vec())),
            _ => crate::shell::prepare(spawn.config, &mut env),
        };
        let cwd = spawn.cwd.or_else(|| std::env::var_os("HOME").map(PathBuf::from));
        let size = window_size(cols, lines, m);
        let graphics = Arc::new(graphics::Shared::default());
        share_size(&graphics, cols, m);
        let blocks = Arc::new(Blocks::default());
        let link = match &spawn.attach {
            Some(session) => Some(Ok(session.clone())),
            None if spawn.config.terminal.sessions && spawn.command.is_none() => {
                let spec = session::Spec {
                    program: program.as_ref().map(|(p, _)| p.clone()),
                    args: program.as_ref().map(|(_, a)| a.clone()).unwrap_or_default(),
                    cwd: cwd.clone(),
                    env: env.clone(),
                    size: (size.num_cols, size.num_lines, size.cell_width, size.cell_height),
                };
                Some(session::start(&spec))
            }
            None => None,
        }
        .map(|session| session.and_then(|id| Link::attach(&id).map(|link| (id, link))));
        let (pty, shell, file, control, session) = match link {
            Some(Ok((session, link))) => {
                let (shell, file, control) = (link.pid, Tty::Path(link.tty.clone()), link.control().ok());
                let pty = graphics::GraphicsPty::session(link, graphics.clone(), blocks.clone())?;
                (pty, shell, file, control, Some(session))
            }
            other => {
                if let Some(Err(err)) = other {
                    log::warn!("no session, the shell runs in the window itself: {err}");
                }
                let options = tty::Options {
                    shell: program.map(|(p, a)| tty::Shell::new(p, a)),
                    working_directory: cwd,
                    drain_on_exit: true,
                    env,
                };
                let pty = tty::new(&options, size, id)?;
                let (shell, file) = (pty.child().id(), Tty::Own(pty.file().try_clone().ok()));
                let pty = graphics::GraphicsPty::new(pty, graphics.clone(), blocks.clone())?;
                (pty, shell, file, None, None)
            }
        };
        let event_loop = EventLoop::new(term.clone(), proxy, pty, true, false)?;
        let sender = event_loop.channel();
        event_loop.spawn();
        if control.is_some() {
            let _ = sender.send(Msg::Resize(size));
        }
        let name = session.as_deref().and_then(session::name);
        let quiet = spawn
            .attach
            .as_ref()
            .map(|_| Instant::now() + Duration::from_millis(1500));
        Ok(Self {
            term,
            search: None,
            hints: None,
            blocks,
            ticking: false,
            id,
            graphics,
            sender,
            tty: file,
            shell,
            area,
            outer: area,
            cols,
            lines,
            title: None,
            rows: Vec::new(),
            generation: u64::MAX,
            cursor_line: None,
            scroll: 0.0,
            selecting: false,
            place_seen: None,
            rule: None,
            control,
            quiet,
            session,
            folds: std::collections::HashSet::new(),
            view: None,
            name,
        })
    }

    pub fn toggle_fold(&mut self, id: u32) {
        if !self.folds.remove(&id) {
            self.folds.insert(id);
        }
    }

    pub fn folded(&self, id: u32) -> bool {
        self.folds.contains(&id)
    }

    pub fn cwd(&self) -> Option<PathBuf> {
        crate::process::cwd(&self.tty.open()?, self.shell)
    }

    /// Where it was when last asked, without asking again.
    pub fn last_location(&self) -> Option<Place> {
        self.place_seen.as_ref().map(|(_, p)| p.clone())
    }

    /// Its folder and the host it is ssh'd into, asked of the system at most once a second.
    pub fn location(&mut self) -> Place {
        let fresh = self
            .place_seen
            .as_ref()
            .is_some_and(|(at, _)| at.elapsed() < Duration::from_millis(900));
        if !fresh {
            let front = self
                .tty
                .open()
                .and_then(|tty| crate::process::command(&tty, self.shell));
            self.place_seen = Some((Instant::now(), Place::read(self.cwd(), front)));
        }
        self.place_seen.as_ref().map(|(_, p)| p.clone()).unwrap_or_default()
    }

    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        let _ = self.sender.send(Msg::Input(bytes.into()));
    }

    /// Writes typed input, jumping back to the bottom of the scrollback first.
    pub fn input(&self, bytes: Vec<u8>) {
        {
            let mut term = self.term.lock();
            if term.grid().display_offset() != 0 {
                term.scroll_display(Scroll::Bottom);
            }
        }
        self.write(bytes);
    }

    /// Puts the grid in `area`; its edge colors may spread out to `outer`.
    pub fn place(&mut self, area: Area, outer: Area, m: Metrics) {
        let (cols, lines) = grid_size(area, m);
        let moved = area != self.area || outer != self.outer;
        self.area = area;
        self.outer = outer;
        if moved {
            self.invalidate();
        }
        share_size(&self.graphics, cols, m);
        if cols == self.cols && lines == self.lines {
            return;
        }
        self.cols = cols;
        self.lines = lines;
        self.term.lock().resize(Size { cols, lines });
        let _ = self.sender.send(Msg::Resize(window_size(cols, lines, m)));
        self.invalidate();
    }

    pub fn invalidate(&mut self) {
        self.rows.clear();
    }

    /// Lets go of the shell; one in a session lives on for the next window.
    pub fn shutdown(&self) {
        let _ = self.sender.send(Msg::Shutdown);
    }

    /// Ends the shell for good, session or not.
    pub fn end(&self) {
        if let Some(control) = &self.control {
            let _ = (&*control).write_all(b"kill\n");
        }
        self.shutdown();
    }

    /// Commands that finish now are a session being replayed.
    pub fn replaying(&self) -> bool {
        self.quiet.is_some_and(|until| Instant::now() < until)
    }

    pub fn mode(&self) -> TermMode {
        *self.term.lock().mode()
    }

    pub fn scroll(&mut self, scroll: Scroll) {
        self.term.lock().scroll_display(scroll);
    }

    /// The grid cell under a window position, and which half of it.
    pub fn cell_at(&self, x: f32, y: f32, m: Metrics) -> (usize, usize, Side) {
        let fx = ((x - self.area.x) / m.cell_width).max(0.0);
        let fy = ((y - self.area.y) / m.cell_height).max(0.0);
        let col = (fx as usize).min(self.cols.saturating_sub(1));
        let line = (fy as usize).min(self.lines.saturating_sub(1));
        let side = if fx.fract() < 0.5 { Side::Left } else { Side::Right };
        (col, line, side)
    }

    fn point(&self, term: &Term<Proxy>, col: usize, line: usize) -> Point {
        let offset = term.grid().display_offset() as i32;
        let grid_line = match self.shown(offset, line) {
            Shown::Line(l) => l,
            _ => line as i32 - offset,
        };
        Point::new(Line(grid_line), Column(col))
    }

    pub fn select(&mut self, col: usize, line: usize, side: Side, clicks: u8, block: bool) {
        let mut term = self.term.lock();
        let ty = match clicks {
            2 => SelectionType::Semantic,
            3.. => SelectionType::Lines,
            _ if block => SelectionType::Block,
            _ => SelectionType::Simple,
        };
        let point = self.point(&term, col, line);
        term.selection = Some(Selection::new(ty, point, side));
        self.selecting = true;
    }

    pub fn extend_selection(&mut self, col: usize, line: usize, side: Side) {
        let mut term = self.term.lock();
        let point = self.point(&term, col, line);
        if let Some(selection) = term.selection.as_mut() {
            selection.update(point, side);
        }
    }

    pub fn selection_text(&self) -> Option<String> {
        let text = self.term.lock().selection_to_string()?;
        let text: String = text.chars().filter(|c| !marks::is_tag(*c)).collect();
        (!text.is_empty()).then_some(text)
    }

    /// What the newest command printed, selected so it shows what was taken.
    pub fn last_output(&mut self) -> Option<String> {
        let id = self.blocks.last()?;
        let mut term = self.term.lock();
        let (start, end) = output_lines(&term, id)?;
        let cols = term.columns();
        let mut selection = Selection::new(SelectionType::Lines, Point::new(Line(start), Column(0)), Side::Left);
        selection.update(Point::new(Line(end), Column(cols - 1)), Side::Right);
        term.selection = Some(selection);
        clean(&term.selection_to_string()?)
    }

    /// What a block printed, if it is still in the scrollback.
    pub fn output(&self, id: u32) -> Option<String> {
        let term = self.term.lock();
        let (start, end) = output_lines(&term, id)?;
        let text = term.bounds_to_string(
            Point::new(Line(start), Column(0)),
            Point::new(Line(end), Column(term.columns() - 1)),
        );
        clean(&text)
    }

    /// The command line a block ran, as typed after the prompt, leading spaces and all.
    pub fn command(&self, id: u32) -> Option<String> {
        self.after_prompt(|block| block == id).filter(|t| !t.trim().is_empty())
    }

    /// What is typed at the prompt now, when no command runs.
    pub fn typed(&self) -> Option<String> {
        if self.busy() {
            return None;
        }
        self.after_prompt(|_| true).map(|t| t.trim().to_owned())
    }

    /// The text after the lowest prompt whose block `which` picks.
    fn after_prompt(&self, which: impl Fn(u32) -> bool) -> Option<String> {
        let term = self.term.lock();
        let grid = term.grid();
        let (first, last) = (-(grid.history_size() as i32), grid.screen_lines() as i32 - 1);
        (first..=last).rev().find_map(|l| {
            let row = &grid[Line(l)];
            let at = (0..row.len()).find(|&c| {
                row[Column(c)]
                    .zerowidth()
                    .and_then(|z| marks::find(z, Kind::Command))
                    .is_some_and(&which)
            })?;
            let text: String = (at + 1..row.len()).map(|c| row[Column(c)].c).collect();
            Some(text.trim_end().to_owned())
        })
    }

    pub fn link_at(&self, col: usize, line: usize) -> Option<(String, String)> {
        let term = self.term.lock();
        let point = self.point(&term, col, line);
        let link = term.grid()[point].hyperlink()?;
        Some((link.id().to_owned(), link.uri().to_owned()))
    }

    pub fn hints_open(&mut self, alphabet: &str) {
        if self.search.is_some() {
            self.search_close();
        }
        self.hints = Some(Hints::new(self.cwd(), alphabet));
        self.invalidate();
    }

    /// A plain URL printed across `col`: its first and last column and the address.
    pub fn url_at(&self, col: usize, line: usize) -> Option<(usize, usize, String)> {
        let term = self.term.lock();
        let offset = term.grid().display_offset() as i32;
        let Shown::Line(grid_line) = self.shown(offset, line) else {
            return None;
        };
        let row = &term.grid()[Line(grid_line)];
        let chars: Vec<char> = (0..self.cols.min(row.len())).map(|c| row[Column(c)].c).collect();
        drop(term);
        find_url(&chars, col)
    }

    /// A path like `src/main.rs:12` under a cell, if it names something that exists: its first and
    /// last column and what it is.
    pub fn path_at(&self, col: usize, line: usize) -> Option<(usize, usize, hints::Target)> {
        let term = self.term.lock();
        let offset = term.grid().display_offset() as i32;
        let Shown::Line(grid_line) = self.shown(offset, line) else {
            return None;
        };
        let row = &term.grid()[Line(grid_line)];
        let text: Vec<char> = (0..self.cols.min(row.len())).map(|c| row[Column(c)].c).collect();
        drop(term);
        let cols = (0..text.len()).collect();
        let rows = [hints::Row {
            text,
            cols,
            wraps: false,
        }];
        let cwd = self.last_location().and_then(|p| p.cwd);
        hints::scan(&rows, &[], cwd.as_deref()).into_iter().find_map(|t| {
            let cells = t.at.first()?;
            let (start, end) = (cells.first()?.1, cells.last()?.1);
            let path = matches!(t.kind, hints::Kind::Path { .. });
            (path && (start..=end).contains(&col)).then_some((start, end, t))
        })
    }

    /// The cursor's cell inside the pane, for the IME popup.
    pub fn cursor_cell(&self) -> Option<(usize, usize)> {
        let term = self.term.lock();
        let content = term.renderable_content();
        let line = self.screen_of(content.display_offset as i32, content.cursor.point.line.0)?;
        Some((content.cursor.point.column.0, line))
    }
}

/// From a block's output mark to the line before the next prompt, or to the cursor.
pub(crate) fn output_lines(term: &Term<Proxy>, id: u32) -> Option<(i32, i32)> {
    let grid = term.grid();
    let (first, last) = (-(grid.history_size() as i32), grid.screen_lines() as i32 - 1);
    let start = (first..=last)
        .rev()
        .find(|&l| row_has(grid, l, |z| marks::find(z, Kind::Output) == Some(id)))?;
    let end = (start + 1..=last)
        .find(|&l| row_has(grid, l, |z| marks::find(z, Kind::Prompt).is_some()))
        .map_or(grid.cursor.point.line.0, |l| l - 1);
    Some((start, end.max(start)))
}

pub(crate) fn clean(text: &str) -> Option<String> {
    let text: String = text.chars().filter(|c| !marks::is_tag(*c)).collect();
    let text = text.trim_end();
    (!text.is_empty()).then(|| text.to_owned())
}

pub(crate) fn row_has(grid: &Grid<Cell>, line: i32, test: impl Fn(&[char]) -> bool) -> bool {
    let row = &grid[Line(line)];
    (0..row.len()).any(|c| row[Column(c)].zerowidth().is_some_and(&test))
}

const SCHEMES: [&str; 7] = [
    "https://", "http://", "file://", "ftp://", "ssh://", "git://", "mailto:",
];

fn find_url(chars: &[char], col: usize) -> Option<(usize, usize, String)> {
    let stop = |c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '`' | '\0');
    if col >= chars.len() || stop(chars[col]) {
        return None;
    }
    let mut start = col;
    while start > 0 && !stop(chars[start - 1]) {
        start -= 1;
    }
    let mut end = col;
    while end + 1 < chars.len() && !stop(chars[end + 1]) {
        end += 1;
    }
    let token: String = chars[start..=end].iter().collect();
    let (at, _) = SCHEMES
        .iter()
        .filter_map(|scheme| token.find(scheme).map(|i| (i, scheme)))
        .min_by_key(|(i, _)| *i)?;
    let begin = start + token[..at].chars().count();
    let text = hints::trim_url(&token[at..]).to_owned();
    let last = (begin + text.chars().count()).checked_sub(1)?;
    let bare = SCHEMES.iter().any(|s| text == *s);
    (col >= begin && col <= last && !bare).then_some((begin, last, text))
}

fn share_size(shared: &graphics::Shared, cols: usize, m: Metrics) {
    use std::sync::atomic::Ordering;
    shared.cell_width.store(m.cell_width as u32, Ordering::Relaxed);
    shared.cell_height.store(m.cell_height as u32, Ordering::Relaxed);
    shared.columns.store(cols as u32, Ordering::Relaxed);
}

fn grid_size(area: Area, m: Metrics) -> (usize, usize) {
    let cols = (area.w / m.cell_width).floor().max(2.0) as usize;
    let lines = (area.h / m.cell_height).floor().max(1.0) as usize;
    (cols, lines)
}

fn window_size(cols: usize, lines: usize, m: Metrics) -> WindowSize {
    WindowSize {
        num_lines: lines as u16,
        num_cols: cols as u16,
        cell_width: m.cell_width as u16,
        cell_height: m.cell_height as u16,
    }
}

#[cfg(test)]
mod tests {
    use super::find_url;

    fn at(line: &str, col: usize) -> Option<String> {
        let chars: Vec<char> = line.chars().collect();
        find_url(&chars, col).map(|(_, _, url)| url)
    }

    #[test]
    fn finds_urls_and_trims_punctuation() {
        let line = "see https://github.com/alacritty/vtebench. or (https://en.wikipedia.org/wiki/Rust_(language)) now";
        assert_eq!(at(line, 10).as_deref(), Some("https://github.com/alacritty/vtebench"));
        assert_eq!(
            at(line, 60).as_deref(),
            Some("https://en.wikipedia.org/wiki/Rust_(language)")
        );
        assert_eq!(at(line, 1), None);
        assert_eq!(at("open file:///tmp/a.txt", 8).as_deref(), Some("file:///tmp/a.txt"));
        assert_eq!(at("x https:// y", 4), None);
    }
}
