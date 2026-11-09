//! Getting bytes across in order over datagrams that may be lost, late or doubled: output from
//! the agent, keys from the client. Each side keeps what it sent until the other says it has
//! it, and sends again from there when nothing new was heard for a while. Nothing here touches a
//! socket, so it's tested with datagrams dropped on purpose.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::wire::{Clock, ROOM};

/// Least and most output in flight before any of it is acknowledged.
const LEAST_WINDOW: usize = 2 * ROOM;
const MOST_WINDOW: usize = 256 * 1024;
/// Most datagrams of keys in flight at once.
const PASTE_PIECES: usize = 16;
/// The longest a sender waits between tries at a peer that stopped answering.
const MOST_PATIENCE: Duration = Duration::from_secs(10);

/// Output for the client, numbered by its offset in an epoch. A new epoch starts with a fresh
/// picture of the screen, when the client fell too far behind for the bytes in between.
pub struct Outbox {
    pub epoch: u32,
    /// The offset of `bytes[0]`; everything before it is acknowledged.
    base: u64,
    bytes: VecDeque<u8>,
    /// Up to where everything has been sent at least once.
    sent: u64,
    window: usize,
    /// When the oldest byte in flight was last sent or acknowledged.
    since: Instant,
    /// Tries since the client last acknowledged anything, each waiting twice as long.
    misses: u32,
}

impl Outbox {
    pub fn new(now: Instant) -> Self {
        Self {
            epoch: 0,
            base: 0,
            bytes: VecDeque::new(),
            sent: 0,
            window: 16 * ROOM,
            since: now,
            misses: 0,
        }
    }

    pub fn push(&mut self, data: &[u8]) {
        self.bytes.extend(data);
    }

    /// Output not yet acknowledged.
    pub fn backlog(&self) -> usize {
        self.bytes.len()
    }

    /// Drops what wasn't acknowledged for a new epoch that starts with `picture`.
    pub fn restart(&mut self, picture: Vec<u8>, now: Instant) {
        self.epoch += 1;
        self.base = 0;
        self.bytes = picture.into();
        self.sent = 0;
        self.since = now;
    }

    pub fn acknowledged(&mut self, epoch: u32, have: u64, now: Instant) {
        if epoch != self.epoch || have <= self.base || have > self.base + self.bytes.len() as u64 {
            return;
        }
        self.bytes.drain(..(have - self.base) as usize);
        self.base = have;
        self.sent = self.sent.max(have);
        self.since = now;
        self.misses = 0;
        self.window = (self.window + ROOM).min(MOST_WINDOW);
    }

    /// The next piece to send, from its offset, when the window has room; output in flight with
    /// no word for longer than `wait`, doubled for each try that went unanswered, is sent again
    /// from the oldest.
    pub fn next(&mut self, now: Instant, wait: Duration) -> Option<(u64, Vec<u8>)> {
        let flying = self.sent > self.base;
        if flying && now.duration_since(self.since) >= self.patience(wait) {
            self.sent = self.base;
            self.since = now;
            self.misses += 1;
            self.window = (self.window / 2).max(LEAST_WINDOW);
        } else if !flying {
            self.since = now;
        }
        let from = (self.sent - self.base) as usize;
        if from >= self.bytes.len() || from >= self.window {
            return None;
        }
        let to = (from + ROOM).min(self.bytes.len());
        let at = self.sent;
        self.sent += (to - from) as u64;
        Some((at, self.bytes.range(from..to).copied().collect()))
    }

    /// When output in flight would be sent again.
    pub fn due(&self, wait: Duration) -> Option<Instant> {
        (self.sent > self.base).then(|| self.since + self.patience(wait))
    }

    fn patience(&self, wait: Duration) -> Duration {
        (wait * 2u32.pow(self.misses.min(8))).min(MOST_PATIENCE.max(wait))
    }
}

/// What of `data`, which starts at offset `at`, comes right after the `have` bytes already
/// taken; `have` moves past it.
fn fresh<'a>(have: &mut u64, at: u64, data: &'a [u8]) -> Option<&'a [u8]> {
    let end = at + data.len() as u64;
    if at > *have || end <= *have {
        return None;
    }
    let new = &data[(*have - at) as usize..];
    *have = end;
    Some(new)
}

/// The client's side of the output: how much of which epoch it has written.
#[derive(Default)]
pub struct Inbox {
    pub epoch: u32,
    pub have: u64,
}

impl Inbox {
    /// What of a piece is new and next in order, to be written; None for anything else.
    pub fn take<'a>(&mut self, epoch: u32, at: u64, data: &'a [u8]) -> Option<&'a [u8]> {
        if epoch > self.epoch {
            if at != 0 {
                return None;
            }
            self.epoch = epoch;
            self.have = 0;
        }
        if epoch < self.epoch {
            return None;
        }
        fresh(&mut self.have, at, data)
    }
}

/// Keys the client sent, kept until the agent has them.
#[derive(Default)]
pub struct Typed {
    base: u64,
    bytes: Vec<u8>,
}

impl Typed {
    pub fn push(&mut self, keys: &[u8]) {
        self.bytes.extend_from_slice(keys);
    }

    pub fn acknowledged(&mut self, have: u64) {
        if have > self.base && have <= self.base + self.bytes.len() as u64 {
            self.bytes.drain(..(have - self.base) as usize);
            self.base = have;
        }
    }

    /// The offset of the oldest key not yet acknowledged.
    pub fn base(&self) -> u64 {
        self.base
    }

    /// Numbers the keys still waiting from 0, for an agent that never had any.
    pub fn rebase(&mut self) {
        self.base = 0;
    }

    /// Keys not yet acknowledged in pieces that each fit a datagram, so a long paste goes in
    /// one round trip rather than one per piece.
    pub fn pieces(&self) -> impl Iterator<Item = (u64, &[u8])> {
        self.bytes
            .chunks(ROOM)
            .take(PASTE_PIECES)
            .enumerate()
            .map(|(i, piece)| (self.base + (i * ROOM) as u64, piece))
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// The agent's side of the keys: how many it has passed on.
#[derive(Default)]
pub struct Keys {
    pub have: u64,
}

impl Keys {
    pub fn take<'a>(&mut self, from: u64, keys: &'a [u8]) -> &'a [u8] {
        fresh(&mut self.have, from, keys).unwrap_or_default()
    }
}

/// The round trip as it's been lately, and the clock both sides stamp datagrams with.
pub struct Timing {
    start: Instant,
    smooth: Option<f64>,
    swing: f64,
    heard: Option<(u16, Instant)>,
}

impl Timing {
    pub fn new(now: Instant) -> Self {
        Self {
            start: now,
            smooth: None,
            swing: 0.0,
            heard: None,
        }
    }

    fn ms(&self, at: Instant) -> u16 {
        (at.duration_since(self.start).as_millis() & 0xffff) as u16
    }

    /// The stamp for a datagram sent now.
    pub fn stamp(&self, now: Instant) -> Clock {
        let (heard, held) = match self.heard {
            Some((t, at)) => (Some(t), now.duration_since(at).as_millis().min(u16::MAX as u128) as u16),
            None => (None, 0),
        };
        Clock {
            now: self.ms(now),
            heard,
            held,
        }
    }

    /// Learns from a datagram's stamp.
    pub fn heard(&mut self, clock: Clock, now: Instant) {
        self.heard = Some((clock.now, now));
        let Some(sent) = clock.heard else {
            return;
        };
        let trip = self.ms(now).wrapping_sub(sent).wrapping_sub(clock.held);
        if trip > 10_000 {
            return;
        }
        let trip = f64::from(trip);
        match self.smooth {
            None => {
                self.smooth = Some(trip);
                self.swing = trip / 2.0;
            }
            Some(s) => {
                self.swing = 0.75 * self.swing + 0.25 * (s - trip).abs();
                self.smooth = Some(0.875 * s + 0.125 * trip);
            }
        }
    }

    pub fn round_trip(&self) -> Option<Duration> {
        self.smooth.map(|s| Duration::from_secs_f64(s / 1000.0))
    }

    /// How long to wait for word before sending again.
    pub fn wait(&self) -> Duration {
        let ms = self.smooth.map_or(1000.0, |s| s + 4.0 * self.swing);
        Duration::from_secs_f64(ms.clamp(50.0, 1000.0) / 1000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Output across a link that loses every third datagram, acknowledged as it arrives.
    #[test]
    fn output_arrives_whole_and_in_order_through_loss() {
        let mut now = Instant::now();
        let mut out = Outbox::new(now);
        let mut inbox = Inbox::default();
        let sent: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        out.push(&sent);
        let mut got = Vec::new();
        let mut count = 0;
        while got.len() < sent.len() {
            now += Duration::from_millis(10);
            while let Some((at, data)) = out.next(now, Duration::from_millis(100)) {
                count += 1;
                if count % 3 == 0 {
                    continue;
                }
                if let Some(fresh) = inbox.take(out.epoch, at, &data) {
                    got.extend_from_slice(fresh);
                }
            }
            out.acknowledged(inbox.epoch, inbox.have, now);
            assert!(count < 10_000, "stuck at {}", got.len());
        }
        assert_eq!(got, sent);
        assert_eq!(out.backlog(), 0);
    }

    #[test]
    fn a_new_epoch_starts_only_at_its_beginning() {
        let now = Instant::now();
        let mut out = Outbox::new(now);
        let mut inbox = Inbox::default();
        out.push(b"old output");
        out.restart(b"picture".to_vec(), now);
        assert_eq!(inbox.take(1, 3, b"ture"), None);
        assert_eq!(inbox.take(1, 0, b"pic"), Some(&b"pic"[..]));
        assert_eq!(inbox.take(0, 0, b"old"), None);
        assert_eq!(inbox.take(1, 0, b"pictu"), Some(&b"tu"[..]));
        let (at, data) = out.next(now, Duration::from_secs(1)).unwrap();
        assert_eq!((at, &data[..]), (0, &b"picture"[..]));
    }

    #[test]
    fn keys_go_once_however_often_they_are_sent() {
        let mut typed = Typed::default();
        let mut keys = Keys::default();
        typed.push(b"ls");
        let (from, k) = typed.pieces().next().unwrap();
        assert_eq!(keys.take(from, k), b"ls");
        typed.push(b" -l");
        let (from, k) = typed.pieces().next().unwrap();
        assert_eq!(keys.take(from, k), b" -l");
        assert_eq!(keys.have, 5);
        assert_eq!(keys.take(from, k), b"");
        typed.acknowledged(keys.have);
        assert!(typed.is_empty());

        let paste = vec![b'x'; ROOM * 3 + 5];
        typed.push(&paste);
        let mut got = Vec::new();
        for (from, piece) in typed.pieces().collect::<Vec<_>>().into_iter().rev() {
            got.extend_from_slice(keys.take(from, piece));
        }
        // Out of order, only the first piece fits; the rest go again.
        assert_eq!(got.len(), ROOM);
        for (from, piece) in typed.pieces() {
            got.extend_from_slice(keys.take(from, piece));
        }
        assert_eq!(got, paste);
    }

    #[test]
    fn the_round_trip_is_measured_from_stamps() {
        let start = Instant::now();
        let mut client = Timing::new(start);
        let mut agent = Timing::new(start);
        let up = client.stamp(start);
        agent.heard(up, start + Duration::from_millis(40));
        let down = agent.stamp(start + Duration::from_millis(45));
        client.heard(down, start + Duration::from_millis(85));
        let trip = client.round_trip().unwrap().as_millis();
        assert!((79..=81).contains(&trip), "{trip}");
    }
}
