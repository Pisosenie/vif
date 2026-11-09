//! Typed characters shown before the other machine echoes them, the way mosh does, for links
//! slow enough to feel. A guess is drawn underlined at the cursor and left to the echo to
//! overwrite; one the echo doesn't confirm in time is wiped. Guesses are only drawn once one in
//! the same line was confirmed, so after Enter, at a password prompt say, nothing typed shows.

use std::io::Write;
use std::time::{Duration, Instant};

use crate::screen::{Screen, goto};

/// Round trips shorter than this feel instant already.
const SLOW: Duration = Duration::from_millis(30);
/// The least time a guess on screen waits for its echo before it's wiped.
const PATIENCE: Duration = Duration::from_millis(500);
/// How long a guess not shown waits, only to tell whether guessing works on this line; nothing
/// needs wiping, so a slow echo still counts.
const UNSEEN: Duration = Duration::from_secs(5);

struct Guess {
    col: usize,
    line: usize,
    c: char,
    at: Instant,
    drawn: bool,
}

impl Guess {
    fn draw(&self, out: &mut impl Write) {
        let _ = write!(out, "{}\x1b[0;4m{}\x1b[0m", goto(self.col, self.line), self.c);
    }
}

#[derive(Default)]
pub struct Guesses {
    guesses: Vec<Guess>,
    /// Whether a guess in this line came true, so new ones may be drawn.
    trusted: bool,
    /// Where the next guess goes, while typing goes on in a line.
    next: Option<(usize, usize)>,
    patience: Option<Duration>,
}

impl Guesses {
    /// Guesses at what `keys` will show, drawing them when the link is slow and guesses have
    /// been right.
    pub fn typed(&mut self, keys: &[u8], screen: &Screen, trip: Option<Duration>, now: Instant, out: &mut impl Write) {
        self.patience = trip.map(|t| (t * 3).max(PATIENCE));
        if screen.alternate() {
            self.guesses.clear();
            self.next = None;
            return;
        }
        let slow = trip.is_some_and(|t| t >= SLOW);
        let (cols, _) = screen.size();
        let mut drew = false;
        for &key in keys {
            if !(0x20..0x7f).contains(&key) {
                // Enter, editing keys and escapes move things in ways not worth guessing, and
                // what was guessed of the line before tells nothing of the next.
                self.next = None;
                self.trusted = false;
                self.guesses.retain(|g| g.drawn);
                continue;
            }
            let (col, line) = *self.next.get_or_insert_with(|| screen.cursor());
            if col + 1 >= usize::from(cols) {
                continue;
            }
            let guess = Guess {
                col,
                line,
                c: key as char,
                at: now,
                drawn: self.trusted && slow && screen.char_at(col, line).is_some(),
            };
            if guess.drawn {
                guess.draw(out);
                drew = true;
            }
            self.guesses.push(guess);
            self.next = Some((col + 1, line));
        }
        if drew {
            self.leave_cursor(out);
        }
    }

    /// Puts the cursor back where the other machine has it before its output is written.
    pub fn before_output(&self, screen: &Screen, out: &mut impl Write) {
        if self.guesses.iter().any(|g| g.drawn) {
            let _ = out.write_all(screen.back_to_cursor().as_bytes());
        }
    }

    /// Settles the guesses the output confirmed, and draws again those still waiting.
    pub fn after_output(&mut self, screen: &Screen, out: &mut impl Write) {
        let before = self.guesses.len();
        self.guesses.retain(|g| screen.char_at(g.col, g.line) != Some(g.c));
        if self.guesses.len() < before {
            self.trusted = true;
        }
        if self.guesses.is_empty() {
            self.next = None;
        }
        let mut drew = false;
        for guess in self.guesses.iter().filter(|g| g.drawn) {
            guess.draw(out);
            drew = true;
        }
        if drew {
            self.leave_cursor(out);
        }
    }

    /// Wipes guesses that waited too long for their echo, which were wrong then.
    pub fn expire(&mut self, screen: &Screen, now: Instant, out: &mut impl Write) {
        let Some(patience) = self.patience else {
            return;
        };
        if !self
            .guesses
            .iter()
            .any(|g| g.drawn && now.duration_since(g.at) >= patience)
        {
            self.guesses.retain(|g| now.duration_since(g.at) < UNSEEN);
            if self.guesses.is_empty() {
                self.next = None;
            }
            return;
        }
        let mut lines: Vec<usize> = self.guesses.iter().filter(|g| g.drawn).map(|g| g.line).collect();
        lines.dedup();
        for line in lines {
            let _ = out.write_all(&screen.repaint_line(line));
        }
        if self.guesses.iter().any(|g| g.drawn) {
            let _ = out.write_all(screen.back_to_cursor().as_bytes());
        }
        self.guesses.clear();
        self.next = None;
        self.trusted = false;
    }

    /// When the next guess runs out of patience.
    pub fn due(&self) -> Option<Instant> {
        let patience = self.patience?;
        self.guesses
            .iter()
            .map(|g| g.at + if g.drawn { patience } else { UNSEEN })
            .min()
    }

    fn leave_cursor(&self, out: &mut impl Write) {
        if let Some((col, line)) = self.next {
            let _ = out.write_all(goto(col, line).as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLOW_TRIP: Option<Duration> = Some(Duration::from_millis(200));

    fn prompt() -> Screen {
        let mut screen = Screen::new(40, 5, 0);
        screen.feed(b"$ ");
        screen
    }

    /// Guesses whose first one, `c`, came true on `screen` over a link with round trip `trip`.
    fn trusted(screen: &mut Screen, c: u8, trip: Option<Duration>, now: Instant) -> Guesses {
        let mut guesses = Guesses::default();
        guesses.typed(&[c], screen, trip, now, &mut Vec::new());
        screen.feed(&[c]);
        guesses.after_output(screen, &mut Vec::new());
        guesses
    }

    #[test]
    fn guesses_show_once_one_came_true() {
        let mut screen = prompt();
        let mut guesses = Guesses::default();
        let now = Instant::now();
        let mut drawn = Vec::new();
        guesses.typed(b"l", &screen, SLOW_TRIP, now, &mut drawn);
        assert!(drawn.is_empty(), "nothing is trusted yet");
        screen.feed(b"l");
        guesses.after_output(&screen, &mut Vec::new());
        guesses.typed(b"s", &screen, SLOW_TRIP, now, &mut drawn);
        let drawn = String::from_utf8(drawn).unwrap();
        assert!(drawn.contains("\x1b[1;4H\x1b[0;4ms"), "{drawn:?}");
    }

    #[test]
    fn a_wrong_guess_is_wiped_and_trust_goes() {
        let mut screen = prompt();
        let now = Instant::now();
        let mut guesses = trusted(&mut screen, b'a', SLOW_TRIP, now);
        let mut drawn = Vec::new();
        guesses.typed(b"b", &screen, SLOW_TRIP, now, &mut drawn);
        assert!(!drawn.is_empty());
        let mut wiped = Vec::new();
        guesses.expire(&screen, now + Duration::from_secs(2), &mut wiped);
        assert!(!wiped.is_empty());
        assert!(guesses.due().is_none());
        let mut drawn = Vec::new();
        guesses.typed(b"c", &screen, SLOW_TRIP, now, &mut drawn);
        assert!(drawn.is_empty());
    }

    #[test]
    fn a_slow_echo_still_earns_trust() {
        let mut screen = prompt();
        let mut guesses = Guesses::default();
        let now = Instant::now();
        guesses.typed(b"e", &screen, SLOW_TRIP, now, &mut Vec::new());
        // Past the patience a guess on screen gets, but this one was never shown.
        guesses.expire(&screen, now + Duration::from_secs(2), &mut Vec::new());
        screen.feed(b"e");
        guesses.after_output(&screen, &mut Vec::new());
        let mut drawn = Vec::new();
        guesses.typed(b"c", &screen, SLOW_TRIP, now, &mut drawn);
        assert!(!drawn.is_empty());
    }

    #[test]
    fn nothing_typed_after_enter_shows_until_it_echoes() {
        let mut screen = prompt();
        let now = Instant::now();
        let mut guesses = trusted(&mut screen, b'x', SLOW_TRIP, now);
        let mut drawn = Vec::new();
        guesses.typed(b"\rsecret", &screen, SLOW_TRIP, now, &mut drawn);
        assert!(drawn.is_empty());
    }

    #[test]
    fn a_fast_link_needs_no_guessing() {
        let mut screen = prompt();
        let now = Instant::now();
        let fast = Some(Duration::from_millis(5));
        let mut guesses = trusted(&mut screen, b'a', fast, now);
        let mut drawn = Vec::new();
        guesses.typed(b"b", &screen, fast, now, &mut drawn);
        assert!(drawn.is_empty());
    }
}
