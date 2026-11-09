//! A screen kept from the output that went by, where no window draws it: a session host keeps
//! one to paint for the window that comes back, a UDP agent to paint afresh for a client that
//! fell behind, and the UDP client one to know where its own terminal stands. A painting is
//! plain escape codes, so any terminal shows it, with the lines that scrolled off going into its
//! scrollback and shell integration's marks where they were, so commands stay blocks.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{self, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Processor};

use crate::marks::{self, Blocks, Kind, Scanner};

/// Most lines of what scrolled off that a painting brings back.
pub const MOST_HISTORY: usize = 1000;

#[derive(Clone, Default)]
struct Title(Arc<Mutex<Option<String>>>);

impl EventListener for Title {
    fn send_event(&self, event: Event) {
        let title = match event {
            Event::Title(t) => Some(t),
            Event::ResetTitle => None,
            // Answers to a program's questions come from the client's own terminal.
            _ => return,
        };
        if let Ok(mut held) = self.0.lock() {
            *held = title;
        }
    }
}

/// What a painting has put back of the marks so far.
#[derive(Default)]
struct Painted {
    /// The last command whose output's start was painted.
    output: Option<u32>,
}

fn size(cols: u16, lines: u16) -> TermSize {
    TermSize::new(usize::from(cols.max(2)), usize::from(lines.max(1)))
}

/// Modes a painting sets as the screen has them, with the codes that turn each on and off.
const MODES: [(TermMode, &str, &str); 13] = [
    (TermMode::APP_CURSOR, "\x1b[?1h", "\x1b[?1l"),
    (TermMode::LINE_WRAP, "\x1b[?7h", "\x1b[?7l"),
    (TermMode::SHOW_CURSOR, "\x1b[?25h", "\x1b[?25l"),
    (TermMode::MOUSE_REPORT_CLICK, "\x1b[?1000h", "\x1b[?1000l"),
    (TermMode::MOUSE_DRAG, "\x1b[?1002h", "\x1b[?1002l"),
    (TermMode::MOUSE_MOTION, "\x1b[?1003h", "\x1b[?1003l"),
    (TermMode::FOCUS_IN_OUT, "\x1b[?1004h", "\x1b[?1004l"),
    (TermMode::SGR_MOUSE, "\x1b[?1006h", "\x1b[?1006l"),
    (TermMode::ALTERNATE_SCROLL, "\x1b[?1007h", "\x1b[?1007l"),
    (TermMode::BRACKETED_PASTE, "\x1b[?2004h", "\x1b[?2004l"),
    (TermMode::INSERT, "\x1b[4h", "\x1b[4l"),
    (TermMode::LINE_FEED_NEW_LINE, "\x1b[20h", "\x1b[20l"),
    (TermMode::APP_KEYPAD, "\x1b=", "\x1b>"),
];

fn modes(out: &mut String, mode: TermMode) {
    for (flag, on, off) in MODES {
        out.push_str(if mode.contains(flag) { on } else { off });
    }
}

/// What a terminal should be left in after a session painted on it.
pub fn reset() -> Vec<u8> {
    let mut out = String::from("\x1b[0m\x1b[r\x1b[?1049l");
    modes(&mut out, TermMode::SHOW_CURSOR | TermMode::LINE_WRAP);
    out.push_str("\x1b[=0;1u\x1b[0 q");
    out.into_bytes()
}

/// Escape codes that move the cursor to a cell, counted from 0.
pub fn goto(col: usize, line: usize) -> String {
    format!("\x1b[{};{}H", line + 1, col + 1)
}

pub struct Screen {
    term: Term<Title>,
    parser: Processor,
    title: Title,
    marks: Scanner,
    /// What the commands marked here did, for the marks a painting puts back.
    blocks: Arc<Blocks>,
    /// Output on its way from the scanner to the grid.
    scanned: VecDeque<u8>,
}

impl Screen {
    /// A blank screen keeping `history` lines that scroll off.
    pub fn new(cols: u16, lines: u16, history: usize) -> Self {
        let title = Title::default();
        let config = term::Config {
            scrolling_history: history,
            ..term::Config::default()
        };
        let blocks = Arc::new(Blocks::default());
        Self {
            term: Term::new(config, &size(cols, lines), title.clone()),
            parser: Processor::new(),
            title,
            marks: Scanner::new(blocks.clone()),
            blocks,
            scanned: VecDeque::new(),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.marks.feed(bytes, &mut self.scanned);
        let (a, b) = self.scanned.as_slices();
        self.parser.advance(&mut self.term, a);
        self.parser.advance(&mut self.term, b);
        self.scanned.clear();
    }

    pub fn size(&self) -> (u16, u16) {
        (self.term.columns() as u16, self.term.screen_lines() as u16)
    }

    pub fn resize(&mut self, cols: u16, lines: u16) {
        if (cols, lines) != self.size() {
            self.term.resize(size(cols, lines));
        }
    }

    /// Lines that scrolled off and are still kept.
    pub fn history(&self) -> usize {
        self.term.grid().history_size()
    }

    /// Whether a full-screen program has the screen to itself.
    pub fn alternate(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    /// Where the cursor is: column and line on the screen.
    pub fn cursor(&self) -> (usize, usize) {
        let point = self.term.grid().cursor.point;
        (point.column.0, point.line.0.max(0) as usize)
    }

    /// The character in a cell on the screen, if one is drawn there.
    pub fn char_at(&self, col: usize, line: usize) -> Option<char> {
        let grid = self.term.grid();
        (col < grid.columns() && line < grid.screen_lines()).then(|| grid[Line(line as i32)][Column(col)].c)
    }

    /// Escape codes that paint this screen on a terminal of its size whatever it showed before,
    /// after the last `history` lines that scrolled off, and leave it in the same modes.
    pub fn paint(&self, history: usize) -> Vec<u8> {
        let mut out = String::from("\x1b[0m\x1b[r\x1b[?1049l\x1b[H\x1b[2J");
        let lines = self.term.screen_lines() as i32;
        let mut painted = Painted::default();
        if self.alternate() {
            out.push_str("\x1b[?1049h\x1b[H\x1b[2J");
            for line in 0..lines {
                out.push_str(&goto(0, line as usize));
                self.row(&mut out, Line(line), Some(&mut painted));
            }
        } else {
            let history = history.min(self.history()) as i32;
            // Written down from the top, the old lines scroll up into the terminal's scrollback
            // and the screen's own end up where they are here.
            for line in -history..lines {
                if line > -history {
                    out.push_str("\r\n");
                }
                self.row(&mut out, Line(line), Some(&mut painted));
            }
        }
        self.finish(&mut out);
        out.into_bytes()
    }

    /// Escape codes that paint one line of the screen in place, leaving the cursor and its
    /// colors as they were, and the marks the terminal already has alone.
    pub fn repaint_line(&self, line: usize) -> Vec<u8> {
        let mut out = format!("\x1b7{}\x1b[0m\x1b[2K", goto(0, line));
        self.row(&mut out, Line(line as i32), None);
        out.push_str("\x1b8");
        out.into_bytes()
    }

    /// Escape codes that put the cursor back where this screen has it, writing in its colors.
    pub fn back_to_cursor(&self) -> String {
        let (col, line) = self.cursor();
        let mut out = goto(col, line);
        pen(&mut out, &self.term.grid().cursor.template);
        out
    }

    /// A line's cells in their looks, with the marks its tags stand for when `marks` keeps
    /// track of a painting.
    fn row(&self, out: &mut String, line: Line, mut marks: Option<&mut Painted>) {
        let grid = self.term.grid();
        let row = &grid[line];
        let blank = Cell::default();
        let end = (0..grid.columns())
            .rev()
            .find(|&c| row[Column(c)] != blank)
            .map_or(0, |c| c + 1);
        let mut last = blank.clone();
        out.push_str("\x1b[0m");
        for col in 0..end {
            let cell = &row[Column(col)];
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            if !same_look(cell, &last) {
                pen(out, cell);
                last = cell.clone();
            }
            let zerowidth = cell.zerowidth().unwrap_or_default();
            if let Some(painted) = marks.as_deref_mut() {
                self.marks_before(out, zerowidth, painted);
            }
            out.push(if cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER) {
                ' '
            } else {
                cell.c
            });
            out.extend(zerowidth.iter().filter(|c| !marks::is_tag(**c)));
            // A command's mark goes on the character before it, as the shell left it.
            if marks.is_some() && marks::find(zerowidth, Kind::Command).is_some() {
                out.push_str("\x1b]133;B\x07");
            }
        }
        out.push_str("\x1b[0m");
    }

    /// The marks a cell's tags stand for that the shell sent before its character: where a
    /// prompt starts, after the end of the command before it, and where output starts.
    fn marks_before(&self, out: &mut String, zerowidth: &[char], painted: &mut Painted) {
        if let Some(prompt) = marks::find(zerowidth, Kind::Prompt) {
            let before = prompt.checked_sub(1);
            if let Some((id, block)) = before.and_then(|id| Some((id, self.blocks.get(id)?))) {
                // A command that printed nothing has no cell to carry where its output began.
                if painted.output != Some(id) {
                    out.push_str("\x1b]133;C\x07");
                }
                if let Some((_, exit)) = block.ended {
                    let _ = write!(out, "\x1b]133;D;{exit}\x07");
                }
            }
            out.push_str("\x1b]133;A\x07");
        }
        if let Some(id) = marks::find(zerowidth, Kind::Output) {
            out.push_str("\x1b]133;C\x07");
            painted.output = Some(id);
        }
    }

    fn finish(&self, out: &mut String) {
        let mode = *self.term.mode();
        modes(out, mode);
        let keyboard = (mode & TermMode::KITTY_KEYBOARD_PROTOCOL).bits()
            >> TermMode::DISAMBIGUATE_ESC_CODES.bits().trailing_zeros();
        let _ = write!(out, "\x1b[={keyboard};1u");
        let style = self.term.cursor_style();
        let shape = match style.shape {
            CursorShape::Underline => 4,
            CursorShape::Beam => 6,
            _ => 2,
        } - u8::from(style.blinking);
        let _ = write!(out, "\x1b[{shape} q");
        if let Ok(title) = self.title.0.lock()
            && let Some(title) = title.as_ref()
        {
            let clean: String = title.chars().filter(|c| !c.is_control()).collect();
            let _ = write!(out, "\x1b]2;{clean}\x07");
        }
        out.push_str(&self.back_to_cursor());
    }
}

/// SGR that writes in a cell's look, from plain.
fn pen(out: &mut String, cell: &Cell) {
    out.push_str("\x1b[0");
    sgr(out, cell);
    out.push('m');
}

fn same_look(a: &Cell, b: &Cell) -> bool {
    a.fg == b.fg
        && a.bg == b.bg
        && (a.flags - Flags::WRAPLINE) == (b.flags - Flags::WRAPLINE)
        && a.underline_color() == b.underline_color()
}

/// SGR parameters for a cell's look, each after a `;`.
fn sgr(out: &mut String, cell: &Cell) {
    let f = cell.flags;
    for (flag, n) in [
        (Flags::BOLD, "1"),
        (Flags::DIM, "2"),
        (Flags::ITALIC, "3"),
        (Flags::INVERSE, "7"),
        (Flags::HIDDEN, "8"),
        (Flags::STRIKEOUT, "9"),
        (Flags::UNDERLINE, "4"),
        (Flags::DOUBLE_UNDERLINE, "4:2"),
        (Flags::UNDERCURL, "4:3"),
        (Flags::DOTTED_UNDERLINE, "4:4"),
        (Flags::DASHED_UNDERLINE, "4:5"),
    ] {
        if f.contains(flag) {
            out.push(';');
            out.push_str(n);
        }
    }
    color(out, cell.fg, 30, 90, 38);
    color(out, cell.bg, 40, 100, 48);
    if let Some(under) = cell.underline_color() {
        color(out, under, 0, 0, 58);
    }
}

fn color(out: &mut String, color: Color, base: u8, bright: u8, extended: u8) {
    let index = match color {
        Color::Spec(rgb) => {
            let _ = write!(out, ";{extended};2;{};{};{}", rgb.r, rgb.g, rgb.b);
            return;
        }
        Color::Indexed(i) => i,
        Color::Named(named) => match named as usize {
            n @ 0..16 => n as u8,
            n if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize).contains(&n) => {
                (n - NamedColor::DimBlack as usize) as u8
            }
            _ => return,
        },
    };
    match index {
        0..8 if base > 0 => {
            let _ = write!(out, ";{}", base + index);
        }
        8..16 if bright > 0 => {
            let _ = write!(out, ";{}", bright + index - 8);
        }
        _ => {
            let _ = write!(out, ";{extended};5;{index}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(screen: &Screen) -> Vec<String> {
        let (cols, lines) = screen.size();
        (0..lines as usize)
            .map(|l| {
                (0..cols as usize)
                    .filter_map(|c| screen.char_at(c, l))
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect()
    }

    #[test]
    fn a_painting_brings_back_the_screen_its_colors_and_cursor() {
        let mut there = Screen::new(30, 5, 100);
        for i in 0..12 {
            there.feed(format!("line {i}\r\n").as_bytes());
        }
        there.feed(b"\x1b[1;31mred\x1b[0m \x1b[38;2;1;2;3mrgb\x1b[0m\x1b[?2004h\x1b]2;work\x07\x1b[3;7H");
        let mut here = Screen::new(30, 5, 100);
        here.feed(b"stale things\r\nmore");
        here.feed(&there.paint(3));
        assert_eq!(text(&here), text(&there));
        assert_eq!(here.cursor(), there.cursor());
        assert!(here.term.mode().contains(TermMode::BRACKETED_PASTE));
        assert_eq!(here.term.grid()[Line(4)][Column(0)].fg, Color::Named(NamedColor::Red));
        assert_eq!(here.history(), 3 + 2);
        assert_eq!(here.term.grid()[Line(-1)][Column(5)].c, '7');
    }

    #[test]
    fn a_full_screen_program_comes_back_on_the_alternate_screen() {
        let mut there = Screen::new(20, 4, 0);
        there.feed(b"shell\r\n\x1b[?1049h\x1b[2;3Hvim here\x1b[?1h");
        let mut here = Screen::new(20, 4, 0);
        here.feed(&there.paint(100));
        assert!(here.alternate());
        assert_eq!(text(&here), text(&there));
        assert_eq!(here.cursor(), there.cursor());
        assert!(here.term.mode().contains(TermMode::APP_CURSOR));
    }

    #[test]
    fn commands_stay_blocks_through_a_painting() {
        let mut there = Screen::new(30, 6, 100);
        there.feed(b"\x1b]133;A\x07$ \x1b]133;B\x07false\r\n\x1b]133;C\x07\x1b]133;D;1\x07\x1b]133;A\x07$ ");
        let painted = there.paint(100);
        let text = String::from_utf8_lossy(&painted);
        assert!(text.contains("\x1b]133;A\x07$ \x1b]133;B\x07false"), "{text:?}");
        assert!(text.contains("\x1b]133;D;1\x07\x1b]133;A\x07$"), "{text:?}");

        // A window taking the painting in finds the command that failed, as it ended.
        let blocks = Arc::new(Blocks::default());
        let (Ok(input), Ok(reply)) = (std::fs::File::open("/dev/null"), std::fs::File::create("/dev/null")) else {
            panic!("no /dev/null");
        };
        let shared = Arc::new(crate::graphics::Shared::default());
        let mut filter = crate::graphics::Filter::new(input, reply, shared, blocks.clone());
        filter.feed(&painted);
        let ended = blocks.last().and_then(|id| blocks.get(id)).and_then(|b| b.ended);
        assert_eq!(ended.map(|(_, exit)| exit), Some(1));
    }

    #[test]
    fn wide_characters_take_their_two_cells() {
        let mut there = Screen::new(10, 2, 0);
        there.feed("界x\u{301}".as_bytes());
        let mut here = Screen::new(10, 2, 0);
        here.feed(&there.paint(0));
        assert_eq!(text(&here), text(&there));
        assert_eq!(here.cursor(), (3, 0));
    }
}
