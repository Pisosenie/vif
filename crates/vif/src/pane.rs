use std::borrow::Cow;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Grid, Scroll};
use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::search::{Match, RegexSearch};
use alacritty_terminal::term::{self, Term, TermDamage, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{CursorShape, CursorStyle};
use vif_config::{BadgeShow, Badges, Config, GreetingShow, Osc52};
use vif_render::{LineCell, Metrics, RECT_CURL, RECT_DASHED, RECT_DOTTED, RECT_HOLLOW, Rect, Renderer, Scene, Style};
use winit::event_loop::EventLoopProxy;
use winit::window::WindowId;

use crate::app::UserEvent;
use crate::colors::{Rgba, Scheme, mix};
use crate::fold::{Shown, folded_row};
use crate::graphics;
use crate::hints::{self, Hints};
use crate::layout::{Area, PaneId};
use crate::marks::{self, Blocks, Kind};
use crate::rules::Place;
use crate::session::{self, Link};

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

    pub fn search_open(&mut self) {
        self.hints = None;
        if self.search.is_some() {
            return;
        }
        let term = self.term.lock();
        let bottom = term.screen_lines() as i32 - 1 - term.grid().display_offset() as i32;
        let origin = Point::new(Line(bottom), Column(term.columns() - 1));
        self.search = Some(Search {
            query: String::new(),
            regex: None,
            current: None,
            origin,
            missed: false,
        });
    }

    pub fn search_close(&mut self) {
        self.search = None;
        self.term.lock().selection = None;
    }

    pub fn search_edit(&mut self, edit: impl FnOnce(&mut String)) {
        let Some(search) = &mut self.search else {
            return;
        };
        edit(&mut search.query);
        search.regex = (!search.query.is_empty())
            .then(|| RegexSearch::new(&literal(&search.query)).ok())
            .flatten();
        search.current = None;
        let origin = search.origin;
        self.find(origin, Direction::Left, Side::Right);
    }

    /// The next match further back, or forward again with `older` false.
    pub fn search_step(&mut self, older: bool) {
        let Some(search) = &self.search else {
            return;
        };
        let term = self.term.lock();
        let (origin, direction, side) = match (&search.current, older) {
            (Some(m), true) => (m.start().sub(&*term, Boundary::None, 1), Direction::Left, Side::Right),
            (Some(m), false) => (m.end().add(&*term, Boundary::None, 1), Direction::Right, Side::Left),
            (None, _) => (search.origin, Direction::Left, Side::Right),
        };
        drop(term);
        self.find(origin, direction, side);
    }

    fn find(&mut self, origin: Point, direction: Direction, side: Side) {
        let Some(search) = &mut self.search else {
            return;
        };
        let mut term = self.term.lock();
        let Some(regex) = &mut search.regex else {
            search.missed = false;
            term.selection = None;
            return;
        };
        let hit = term.search_next(regex, origin, direction, side, None);
        search.missed = hit.is_none();
        match &hit {
            Some(m) => {
                let mut selection = Selection::new(SelectionType::Simple, *m.start(), Side::Left);
                selection.update(*m.end(), Side::Right);
                term.selection = Some(selection);
                term.scroll_to_point(*m.start());
            }
            None => term.selection = None,
        }
        search.current = hit.or(search.current.take());
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

    /// Draws each image whole, anchored at its topmost visible cell: text may reflow around
    /// it on resize, the picture stays in one piece.
    fn draw_images(&self, r: &mut Renderer, scene: &mut Scene) {
        let mut anchors: std::collections::HashMap<u32, Mark> = std::collections::HashMap::new();
        // Starts of image rows first: after a reflow the tail of a row sits at another column.
        let rank = |m: &Mark| (m.col != 0, m.row, m.col);
        for mark in self.rows.iter().flatten().flat_map(|row| &row.marks) {
            let better = anchors.get(&mark.id).is_none_or(|a| rank(mark) < rank(a));
            if better {
                anchors.insert(mark.id, *mark);
            }
        }
        if anchors.is_empty() {
            return;
        }
        let m = r.metrics();
        let a = self.area;
        let placements = self.graphics.placements.lock().expect("placements");
        for (id, mark) in anchors {
            let Some(p) = placements.get(&id) else {
                continue;
            };
            let (iw, ih) = (p.image.width as f32, p.image.height as f32);
            let (bw, bh) = (p.cols as f32 * m.cell_width, p.rows as f32 * m.cell_height);
            let scale = if p.scaled { (bw / iw).min(bh / ih) } else { 1.0 };
            let (dw, dh) = (iw * scale, ih * scale);
            let x = mark.x - mark.col as f32 * m.cell_width;
            let y = mark.y - mark.row as f32 * m.cell_height;
            let (left, top) = (x.max(a.x), y.max(a.y));
            let (right, bottom) = ((x + dw).min(a.x + a.w), (y + dh).min(a.y + a.h));
            if right <= left || bottom <= top {
                continue;
            }
            let key = (self.id << 32) | u64::from(id);
            r.image(key, &p.image.rgba, p.image.width, p.image.height);
            scene.images.push((
                key,
                vif_render::Sprite {
                    pos: [left, top],
                    size: [right - left, bottom - top],
                    uv: [
                        (left - x) / dw,
                        (top - y) / dh,
                        (right - left) / dw,
                        (bottom - top) / dh,
                    ],
                    color: [1.0, 1.0, 1.0, 1.0],
                },
            ));
        }
    }

    /// Exit status and time at the end of each command line.
    fn draw_badges(&mut self, r: &mut Renderer, look: &Look, scene: &mut Scene) {
        let m = r.metrics();
        let shown: Vec<(usize, usize, marks::Block)> = self
            .rows
            .iter()
            .enumerate()
            .filter_map(|(line, row)| {
                let row = row.as_ref()?;
                Some((line, row.end, self.blocks.get(row.block?)?))
            })
            .collect();
        self.ticking = shown.iter().any(|(_, _, b)| b.ended.is_none());
        for (line, end, block) in shown {
            let Some((text, color)) = badge(&block, look.scheme, look.badges) else {
                continue;
            };
            let len = text.chars().count();
            if end + len + 2 > self.cols {
                continue;
            }
            let chars: Vec<String> = text.chars().map(String::from).collect();
            let cells: Vec<LineCell> = chars
                .iter()
                .map(|t| LineCell {
                    text: t,
                    style: Style::Regular,
                    color,
                    wide: false,
                })
                .collect();
            let x = self.area.x + (self.cols - len - 1) as f32 * m.cell_width;
            let y = self.area.y + line as f32 * m.cell_height;
            r.draw_line(&cells, x, y, scene);
        }
    }

    pub fn draw(&mut self, r: &mut Renderer, look: &Look, scene: &mut Scene) {
        let m = r.metrics();
        let mut term = self.term.lock();
        if self.generation != r.generation() || self.rows.len() != self.lines {
            self.rows = vec![None; self.lines];
            self.generation = r.generation();
        }
        match term.damage() {
            TermDamage::Full => self.rows.iter_mut().for_each(|row| *row = None),
            TermDamage::Partial(lines) => {
                for damage in lines {
                    if let Some(row) = self.rows.get_mut(damage.line) {
                        *row = None;
                    }
                }
            }
        }
        term.reset_damage();
        // Folds move lines around the screen, so every row is drawn again while one shows.
        let view = self.fold_view(&term);
        if view.is_some() || self.view.is_some() {
            self.rows.iter_mut().for_each(|row| *row = None);
        }
        self.view = view;

        let content = term.renderable_content();
        let offset = content.display_offset as i32;
        let cursor = self
            .screen_of(offset, content.cursor.point.line.0)
            .map(|line| (content.cursor.point.column.0, line))
            .filter(|_| content.cursor.shape != CursorShape::Hidden);
        let shape = if look.focused {
            content.cursor.shape
        } else {
            CursorShape::HollowBlock
        };
        let solid = cursor.filter(|_| look.focused && look.cursor_on && shape == CursorShape::Block);
        let selection = content.selection;
        let colors = *content.colors;

        for line in [self.cursor_line, cursor.map(|c| c.1)].into_iter().flatten() {
            if let Some(row) = self.rows.get_mut(line) {
                *row = None;
            }
        }
        self.cursor_line = cursor.map(|c| c.1);

        // Copy what changed and let the PTY thread go before shaping and rasterizing.
        let cols = self.cols;
        let dirty: Vec<(usize, Shown, Vec<Cell>)> = (0..self.lines)
            .filter(|&line| self.rows[line].is_none())
            .map(|line| {
                let shown = self.shown(offset, line);
                let cells = match shown {
                    Shown::Line(grid_line) => {
                        let row = &term.grid()[Line(grid_line)];
                        (0..cols.min(row.len())).map(|c| row[Column(c)].clone()).collect()
                    }
                    _ => Vec::new(),
                };
                (line, shown, cells)
            })
            .collect();
        let wide_cursor = cursor.is_some_and(|(col, _)| {
            term.grid()[content.cursor.point.line][Column(col)]
                .flags
                .contains(Flags::WIDE_CHAR)
        });
        if let Some(hints) = &mut self.hints
            && !dirty.is_empty()
        {
            let (rows, links) = screen_text(&term, self.lines, self.cols);
            hints.refresh(&rows, &links);
        }
        drop(term);

        let (x0, y0) = (self.area.x, self.area.y);
        let o = self.outer;
        for (line, shown, cells) in dirty {
            let invert = solid.filter(|c| c.1 == line).map(|c| c.0);
            let y = y0 + line as f32 * m.cell_height;
            if let Shown::Folded { lines, .. } = shown {
                self.rows[line] = Some(Row {
                    scene: folded_row(r, look, lines, x0, y),
                    marks: Vec::new(),
                    block: None,
                    end: 0,
                });
                continue;
            }
            let bleed = Bleed {
                left: o.x,
                right: o.x + o.w,
                top: (line == 0).then_some(o.y),
                bottom: (line + 1 == self.lines).then_some(o.y + o.h),
            };
            self.rows[line] = Some(build_row(r, &cells, &colors, look, invert, x0, y, bleed));
        }
        for row in self.rows.iter().flatten() {
            scene.extend(&row.scene);
        }
        self.draw_images(r, scene);
        self.draw_badges(r, look, scene);

        if let Some(range) = selection {
            let color = rgba_alpha(look.scheme.palette.selection, 1.0);
            let last = self.cols.saturating_sub(1);
            for line in 0..self.lines {
                let Shown::Line(grid_line) = self.shown(offset, line) else {
                    continue;
                };
                if grid_line < range.start.line.0 || grid_line > range.end.line.0 {
                    continue;
                }
                let (from, to) = if range.is_block {
                    (range.start.column.0, range.end.column.0)
                } else {
                    let from = if grid_line == range.start.line.0 {
                        range.start.column.0
                    } else {
                        0
                    };
                    let to = if grid_line == range.end.line.0 {
                        range.end.column.0
                    } else {
                        last
                    };
                    (from, to)
                };
                if from > to {
                    continue;
                }
                scene.back.push(Rect::fill(
                    x0 + from as f32 * m.cell_width,
                    y0 + line as f32 * m.cell_height,
                    (to - from + 1) as f32 * m.cell_width,
                    m.cell_height,
                    color,
                ));
            }
        }

        if let Some((col, line)) = cursor.filter(|_| look.cursor_on || !look.focused) {
            let w = m.cell_width * if wide_cursor { 2.0 } else { 1.0 };
            let (x, y) = (x0 + col as f32 * m.cell_width, y0 + line as f32 * m.cell_height);
            let color = colors[alacritty_terminal::vte::ansi::NamedColor::Cursor]
                .map(|c| {
                    [
                        f32::from(c.r) / 255.0,
                        f32::from(c.g) / 255.0,
                        f32::from(c.b) / 255.0,
                        1.0,
                    ]
                })
                .unwrap_or_else(|| look.scheme.cursor());
            let beam = (m.cell_width / 8.0).round().max(2.0);
            match shape {
                CursorShape::Block => scene.back.push(Rect::fill(x, y, w, m.cell_height, color)),
                CursorShape::Beam => scene.front.push(Rect::fill(x, y, beam, m.cell_height, color)),
                CursorShape::Underline => scene
                    .front
                    .push(Rect::fill(x, y + m.cell_height - beam, w, beam, color)),
                CursorShape::HollowBlock => scene
                    .front
                    .push(Rect::fill(x, y, w, m.cell_height, color).kind(RECT_HOLLOW, m.stroke.max(1.0))),
                CursorShape::Hidden => {}
            }
        }
    }
}

fn rgba_alpha(c: vif_config::Rgb, a: f32) -> Rgba {
    crate::colors::rgba(c, a)
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

/// `✓ 41.2s · 16:39`, `✗ exit 101 · 3.1s · 16:40`, `● running · 1:12`; with `show = "failures"`
/// only the failures.
fn badge(block: &marks::Block, scheme: &Scheme, style: &Badges) -> Option<(String, Rgba)> {
    let took = block.running();
    let all = style.show == BadgeShow::All;
    let mark = |glyph: &str, rest: String| {
        if glyph.is_empty() {
            rest
        } else {
            format!("{glyph} {rest}")
        }
    };
    let ended = |text: String| {
        if style.clock {
            format!("{text} · {}", marks::time_of_day(block.finished_at()))
        } else {
            text
        }
    };
    match block.ended {
        _ if style.show == BadgeShow::None => None,
        None => all.then(|| {
            let text = format!("running · {}", marks::clock(took));
            (mark(&style.running, text), scheme.accent())
        }),
        Some((_, 0)) => all.then(|| {
            let text = ended(marks::clock(took));
            (mark(&style.ok, text), mix(scheme.fg(), scheme.bg(), 0.5))
        }),
        Some((_, code)) => {
            let text = ended(format!("exit {code} · {}", marks::clock(took)));
            Some((mark(&style.failed, text), rgba_alpha(scheme.palette.ansi[1], 1.0)))
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_row(
    r: &mut Renderer,
    row: &[Cell],
    colors: &term::color::Colors,
    look: &Look,
    invert: Option<usize>,
    x0: f32,
    y: f32,
    bleed: Bleed,
) -> Row {
    let m = r.metrics();
    let scheme = look.scheme;
    let mut scene = Scene::default();
    let cols = row.len();
    let mut text = String::with_capacity(cols);
    let mut spans = Vec::with_capacity(cols);
    let mut meta: Vec<(Style, Rgba, bool)> = Vec::with_capacity(cols);
    let mut run: Option<(usize, Rgba)> = None;
    let mut marks = Vec::new();
    let (mut block, mut end) = (None, 0);
    let flush = |run: &mut Option<(usize, Rgba)>, end: usize, scene: &mut Scene| {
        if let Some((start, color)) = run.take() {
            let left = if start == 0 {
                bleed.left
            } else {
                x0 + start as f32 * m.cell_width
            };
            let right = if end == cols {
                bleed.right
            } else {
                x0 + end as f32 * m.cell_width
            };
            let top = bleed.top.unwrap_or(y);
            let bottom = bleed.bottom.unwrap_or(y + m.cell_height);
            scene
                .back
                .push(Rect::fill(left, top, right - left, bottom - top, color));
        }
    };
    for (col, cell) in row.iter().enumerate() {
        let flags = cell.flags;
        let (mut fg, bg, plain_bg) = scheme.cell(colors, cell.fg, cell.bg, flags);
        match run {
            Some((_, color)) if !plain_bg && color == bg => {}
            _ => {
                flush(&mut run, col, &mut scene);
                if !plain_bg {
                    run = Some((col, bg));
                }
            }
        }
        if invert == Some(col) {
            fg = if plain_bg { scheme.bg() } else { bg };
        }

        let start = text.len();
        if cell.c == graphics::PLACEHOLDER {
            if let Some(mark) = Mark::read(cell, x0 + col as f32 * m.cell_width, y)
                && !marks.iter().any(|m: &Mark| m.id == mark.id)
            {
                marks.push(mark);
            }
        } else if !flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            text.push(cell.c);
            if let Some(extra) = cell.zerowidth() {
                text.extend(extra.iter().filter(|c| !crate::marks::is_tag(**c)));
                block = block.or_else(|| crate::marks::find(extra, Kind::Command));
            }
            if cell.c != ' ' {
                end = col + 1;
            }
        }
        spans.push(start..text.len());
        let style = Style::new(flags.contains(Flags::BOLD), flags.contains(Flags::ITALIC));
        meta.push((style, fg, flags.contains(Flags::WIDE_CHAR)));

        let x = x0 + col as f32 * m.cell_width;
        let w = m.cell_width * if flags.contains(Flags::WIDE_CHAR) { 2.0 } else { 1.0 };
        let underline = cell.underline_color().map(|c| scheme.resolve(colors, c)).unwrap_or(fg);
        let s = m.stroke;
        let linked = look
            .link
            .is_some_and(|id| cell.hyperlink().is_some_and(|h| h.id() == id));
        if flags.contains(Flags::UNDERCURL) {
            scene
                .front
                .push(Rect::fill(x, y + m.underline - s, w, s * 3.0, underline).kind(RECT_CURL, s));
        } else if flags.contains(Flags::DOUBLE_UNDERLINE) {
            scene.front.push(Rect::fill(x, y + m.underline - s, w, s, underline));
            scene.front.push(Rect::fill(x, y + m.underline + s, w, s, underline));
        } else if flags.contains(Flags::DOTTED_UNDERLINE) {
            scene
                .front
                .push(Rect::fill(x, y + m.underline, w, s, underline).kind(RECT_DOTTED, s));
        } else if flags.contains(Flags::DASHED_UNDERLINE) {
            scene
                .front
                .push(Rect::fill(x, y + m.underline, w, s, underline).kind(RECT_DASHED, s));
        } else if flags.contains(Flags::UNDERLINE) || linked {
            scene.front.push(Rect::fill(x, y + m.underline, w, s, underline));
        } else if cell.hyperlink().is_some() {
            let faint = mix(underline, scheme.bg(), 0.6);
            scene
                .front
                .push(Rect::fill(x, y + m.underline, w, s, faint).kind(RECT_DOTTED, s));
        }
        if flags.contains(Flags::STRIKEOUT) {
            scene.front.push(Rect::fill(x, y + m.strikeout, w, s, fg));
        }
    }
    flush(&mut run, cols, &mut scene);

    let cells: Vec<LineCell> = spans
        .iter()
        .zip(&meta)
        .map(|(span, (style, color, wide))| LineCell {
            text: &text[span.clone()],
            style: *style,
            color: *color,
            wide: *wide,
        })
        .collect();
    r.draw_line(&cells, x0, y, &mut scene);
    Row {
        scene,
        marks,
        block,
        end,
    }
}

/// The visible rows as text, and the OSC 8 links on them.
fn screen_text(term: &Term<Proxy>, lines: usize, cols: usize) -> (Vec<hints::Row>, Vec<hints::Link>) {
    let offset = term.grid().display_offset() as i32;
    let mut rows = Vec::with_capacity(lines);
    let mut links = Vec::new();
    for line in 0..lines {
        let row = &term.grid()[Line(line as i32 - offset)];
        let n = cols.min(row.len());
        let (mut text, mut at) = (Vec::with_capacity(n), Vec::with_capacity(n));
        let mut link: Option<(String, hints::Link)> = None;
        for col in 0..n {
            let cell = &row[Column(col)];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            text.push(if cell.c == graphics::PLACEHOLDER { ' ' } else { cell.c });
            at.push(col);
            let h = cell.hyperlink();
            match (&h, &mut link) {
                (Some(h), Some((id, l))) if id == h.id() => l.end = col,
                _ => {
                    links.extend(link.take().map(|(_, l)| l));
                    link = h.map(|h| {
                        let l = hints::Link {
                            line,
                            start: col,
                            end: col,
                            uri: h.uri().to_owned(),
                        };
                        (h.id().to_owned(), l)
                    });
                }
            }
        }
        links.extend(link.map(|(_, l)| l));
        let wraps = n > 0 && row[Column(n - 1)].flags.contains(Flags::WRAPLINE);
        rows.push(hints::Row { text, cols: at, wraps });
    }
    (rows, links)
}

fn literal(query: &str) -> String {
    let mut out = String::with_capacity(query.len() * 2);
    for c in query.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
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
