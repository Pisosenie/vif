//! Datagrams between `vif ssh` and its agent on the other machine: what each says, sealed with
//! ChaCha20-Poly1305 under a key that came over ssh. A datagram's number is its nonce and only
//! ever grows, so one seen before, or forged, is dropped without a reply.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key as Secret, Nonce};

pub const KEY_LEN: usize = 32;
/// Most a datagram may carry, so it crosses any path without being split.
pub const MOST: usize = 1200;
/// Room for data in one datagram after the header, the tag and the rest of a message.
pub const ROOM: usize = MOST - HEADER - TAG - 48;

const VERSION: u8 = 1;
const HEADER: usize = 10;
const TAG: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Client = 0,
    Agent = 1,
}

/// Seals what one side sends.
pub struct Sealer {
    cipher: ChaCha20Poly1305,
    side: Side,
    next: u64,
}

impl Sealer {
    pub fn new(key: &[u8; KEY_LEN], side: Side) -> Self {
        Self {
            cipher: ChaCha20Poly1305::new(Secret::from_slice(key)),
            side,
            next: 0,
        }
    }

    pub fn seal(&mut self, message: &[u8]) -> Vec<u8> {
        let seq = self.next;
        self.next += 1;
        let mut out = Vec::with_capacity(HEADER + message.len() + TAG);
        out.push(VERSION);
        out.push(self.side as u8);
        out.extend_from_slice(&seq.to_le_bytes());
        let sealed = self
            .cipher
            .encrypt(
                &nonce(self.side, seq),
                Payload {
                    msg: message,
                    aad: &out,
                },
            )
            .unwrap_or_default();
        out.extend_from_slice(&sealed);
        out
    }
}

/// Opens what the other side sent.
pub struct Opener {
    cipher: ChaCha20Poly1305,
    from: Side,
    last: Option<u64>,
}

impl Opener {
    pub fn new(key: &[u8; KEY_LEN], from: Side) -> Self {
        Self {
            cipher: ChaCha20Poly1305::new(Secret::from_slice(key)),
            from,
            last: None,
        }
    }

    /// The message in `datagram` when it is genuine and newer than any before it.
    pub fn open(&mut self, datagram: &[u8]) -> Option<Vec<u8>> {
        let (header, sealed) = datagram.split_at_checked(HEADER)?;
        if header[0] != VERSION || header[1] != self.from as u8 {
            return None;
        }
        let seq = u64::from_le_bytes(header[2..].try_into().ok()?);
        if self.last.is_some_and(|last| seq <= last) {
            return None;
        }
        let message = self
            .cipher
            .decrypt(
                &nonce(self.from, seq),
                Payload {
                    msg: sealed,
                    aad: header,
                },
            )
            .ok()?;
        self.last = Some(seq);
        Some(message)
    }
}

fn nonce(side: Side, seq: u64) -> Nonce {
    let mut n = [0u8; 12];
    n[0] = side as u8;
    n[4..].copy_from_slice(&seq.to_le_bytes());
    *Nonce::from_slice(&n)
}

/// When a message was sent and what the sender last heard, for timing the round trip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Clock {
    /// The sender's clock in milliseconds, wrapping.
    pub now: u16,
    /// The other side's `now` the sender heard last, if any.
    pub heard: Option<u16>,
    /// Milliseconds between hearing that and sending this.
    pub held: u16,
}

/// What the client says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Up {
    pub clock: Clock,
    /// The stream it has, all bytes before `have` of `epoch`.
    pub epoch: u32,
    pub have: u64,
    /// Its terminal: columns, lines, and pixels across and down.
    pub size: [u16; 4],
    /// Keys from offset `from` of everything typed.
    pub from: u64,
    pub keys: Vec<u8>,
    /// This end is going.
    pub bye: bool,
    /// Wants a reply even with nothing to acknowledge: a hello, or news of a move or a resize.
    pub answer: bool,
}

/// How the session ended, when it has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum End {
    #[default]
    No,
    /// The shell exited.
    Exited,
    /// Another connection took the session.
    Taken,
}

/// What the agent says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Down {
    pub clock: Clock,
    /// How much of what was typed it has.
    pub keys: u64,
    /// Output from offset `at` of `epoch`.
    pub epoch: u32,
    pub at: u64,
    pub data: Vec<u8>,
    pub end: End,
}

impl Up {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.clock(self.clock);
        w.u32(self.epoch);
        w.u64(self.have);
        for n in self.size {
            w.u16(n);
        }
        w.u64(self.from);
        w.bytes(&self.keys);
        w.u8(u8::from(self.bye) | u8::from(self.answer) << 1);
        w.0
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let mut r = Reader(bytes);
        let (clock, epoch, have) = (r.clock()?, r.u32()?, r.u64()?);
        let size = [r.u16()?, r.u16()?, r.u16()?, r.u16()?];
        let (from, keys, flags) = (r.u64()?, r.bytes()?, r.u8()?);
        let up = Self {
            clock,
            epoch,
            have,
            size,
            from,
            keys,
            bye: flags & 1 != 0,
            answer: flags & 2 != 0,
        };
        r.0.is_empty().then_some(up)
    }
}

impl Down {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.clock(self.clock);
        w.u64(self.keys);
        w.u32(self.epoch);
        w.u64(self.at);
        w.bytes(&self.data);
        w.u8(match self.end {
            End::No => 0,
            End::Exited => 1,
            End::Taken => 2,
        });
        w.0
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let mut r = Reader(bytes);
        let down = Self {
            clock: r.clock()?,
            keys: r.u64()?,
            epoch: r.u32()?,
            at: r.u64()?,
            data: r.bytes()?,
            end: match r.u8()? {
                0 => End::No,
                1 => End::Exited,
                2 => End::Taken,
                _ => return None,
            },
        };
        r.0.is_empty().then_some(down)
    }
}

#[derive(Default)]
struct Writer(Vec<u8>);

impl Writer {
    fn u8(&mut self, n: u8) {
        self.0.push(n);
    }
    fn u16(&mut self, n: u16) {
        self.0.extend_from_slice(&n.to_le_bytes());
    }
    fn u32(&mut self, n: u32) {
        self.0.extend_from_slice(&n.to_le_bytes());
    }
    fn u64(&mut self, n: u64) {
        self.0.extend_from_slice(&n.to_le_bytes());
    }
    fn bytes(&mut self, b: &[u8]) {
        let len = b.len().min(u16::MAX as usize);
        self.u16(len as u16);
        self.0.extend_from_slice(&b[..len]);
    }
    fn clock(&mut self, c: Clock) {
        self.u16(c.now);
        self.u8(u8::from(c.heard.is_some()));
        self.u16(c.heard.unwrap_or(0));
        self.u16(c.held);
    }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let (head, rest) = self.0.split_first_chunk::<N>()?;
        self.0 = rest;
        Some(*head)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take::<1>().map(|b| b[0])
    }
    fn u16(&mut self) -> Option<u16> {
        self.take().map(u16::from_le_bytes)
    }
    fn u32(&mut self) -> Option<u32> {
        self.take().map(u32::from_le_bytes)
    }
    fn u64(&mut self) -> Option<u64> {
        self.take().map(u64::from_le_bytes)
    }
    fn bytes(&mut self) -> Option<Vec<u8>> {
        let len = self.u16()? as usize;
        let (head, rest) = self.0.split_at_checked(len)?;
        self.0 = rest;
        Some(head.to_vec())
    }
    fn clock(&mut self) -> Option<Clock> {
        let now = self.u16()?;
        let known = self.u8()? != 0;
        let heard = self.u16()?;
        Some(Clock {
            now,
            heard: known.then_some(heard),
            held: self.u16()?,
        })
    }
}

/// Fresh random bytes from the system.
pub fn random<const N: usize>() -> std::io::Result<[u8; N]> {
    let mut out = [0u8; N];
    for chunk in out.chunks_mut(256) {
        // SAFETY: getentropy fills at most 256 bytes of the buffer it's given.
        if unsafe { libc::getentropy(chunk.as_mut_ptr().cast(), chunk.len()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_survive_the_trip() {
        let up = Up {
            clock: Clock {
                now: 7,
                heard: Some(65000),
                held: 3,
            },
            epoch: 2,
            have: 1 << 40,
            size: [120, 40, 1440, 960],
            from: 9,
            keys: b"ls\r".to_vec(),
            bye: false,
            answer: true,
        };
        assert_eq!(Up::decode(&up.encode()), Some(up));
        let down = Down {
            keys: 12,
            epoch: 3,
            at: 5,
            data: vec![0x1b; 900],
            end: End::Taken,
            ..Down::default()
        };
        assert_eq!(Down::decode(&down.encode()), Some(down.clone()));
        let mut bytes = down.encode();
        bytes.push(0);
        assert_eq!(Down::decode(&bytes), None);
        assert_eq!(Down::decode(&bytes[..10]), None);
    }

    #[test]
    fn only_genuine_new_datagrams_open() {
        let key = random::<KEY_LEN>().unwrap();
        let mut sealer = Sealer::new(&key, Side::Client);
        let mut opener = Opener::new(&key, Side::Client);
        let first = sealer.seal(b"one");
        let second = sealer.seal(b"two");
        assert_eq!(opener.open(&second).as_deref(), Some(&b"two"[..]));
        // Older than one already opened, so a replay or a straggler.
        assert_eq!(opener.open(&first), None);
        assert_eq!(opener.open(&second), None);

        let mut forged = sealer.seal(b"three");
        let last = forged.len() - 1;
        forged[last] ^= 1;
        assert_eq!(opener.open(&forged), None);
        // The agent's own words can't be played back to it as the client's.
        let mut agent = Sealer::new(&key, Side::Agent);
        assert_eq!(opener.open(&agent.seal(b"x")), None);
        let other = Sealer::new(&random::<KEY_LEN>().unwrap(), Side::Client).seal(b"y");
        assert_eq!(opener.open(&other), None);
        assert_eq!(opener.open(&[1, 0]), None);
    }
}
