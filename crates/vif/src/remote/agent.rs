//! The other machine's end of a session over UDP. `vif udp-agent NAME` runs under ssh: it starts
//! an agent of its own session, so it outlives ssh, passes on the agent's port and key, and
//! ends. The agent joins the session host the way a window would, keeps a screen from what the
//! shell prints, and talks to whichever address its client writes from, so the client may move
//! between networks. Nobody heard from for ten minutes, it goes; the shell stays.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, UdpSocket};
use std::os::unix::net::UnixStream;
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use super::flow::{Keys, Outbox, Timing};
use super::wire::{self, Down, End, KEY_LEN, Opener, Sealer, Side, Up};
use super::{bridge, clean, pump};
use crate::screen::{MOST_HISTORY, Screen};
use crate::session;

/// The ports mosh uses, so firewall rules made for it let these through.
const PORTS: (u16, u16) = (60000, 61000);
const IDLE: Duration = Duration::from_secs(600);
const NEVER_CAME: Duration = Duration::from_secs(60);
const HEARTBEAT: Duration = Duration::from_secs(3);
/// Output a client may fall behind by before it gets a fresh picture of the screen instead.
const MOST_BEHIND: usize = 256 * 1024;
/// How long the output played back on joining a session keeps coming before it's painted.
const SETTLE: Duration = Duration::from_millis(100);
/// How long an ended session keeps telling its client so.
const LINGER: Duration = Duration::from_secs(5);
/// Lines a screen here keeps after they scroll off.
const HISTORY: usize = 2 * MOST_HISTORY;

/// The line `vif udp-agent` answers with: protocol, port and key.
pub(super) const GREETING: &str = "vif-udp 1";

/// `vif udp-agent NAME`, or `--serve NAME` for the agent itself.
pub fn main(args: &[String]) -> ExitCode {
    let serve = args.first().is_some_and(|a| a == "--serve");
    let Some(id) = args.get(usize::from(serve)).and_then(|n| clean(n)) else {
        eprintln!("usage: vif udp-agent NAME");
        return ExitCode::from(2);
    };
    if serve {
        return match agent(&id) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("vif: {err}");
                ExitCode::FAILURE
            }
        };
    }
    if let Err(err) = bridge::ensure(&id) {
        eprintln!("vif: can't start the session: {err}");
        return ExitCode::FAILURE;
    }
    match start(&id) {
        Ok(line) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("vif: can't start the UDP agent: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Starts the agent in a session of its own, so ssh's going doesn't take it along, and returns
/// what it says first.
fn start(id: &str) -> io::Result<String> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(["udp-agent", "--serve", id])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    session::detach(&mut command);
    let mut child = command.spawn()?;
    let mut line = String::new();
    if let Some(out) = child.stdout.take() {
        BufReader::new(out).read_line(&mut line)?;
    }
    let line = line.trim();
    if line.starts_with(GREETING) {
        // The agent goes on; reaping it is init's now.
        std::mem::forget(child);
        Ok(line.to_owned())
    } else {
        let _ = child.kill();
        Err(io::Error::other("it didn't say where it listens"))
    }
}

fn agent(id: &str) -> io::Result<()> {
    let socket = bind()?;
    let key = wire::random::<KEY_LEN>()?;
    let port = socket.local_addr()?.port();
    let mut out = io::stdout();
    writeln!(out, "{GREETING} {port} {}", STANDARD.encode(key))?;
    out.flush()?;
    quiet_stdout();
    Agent::new(id, socket, &key).run();
    Ok(())
}

fn bind() -> io::Result<UdpSocket> {
    let span = PORTS.1 - PORTS.0 + 1;
    let start = u16::from_le_bytes(wire::random::<2>()?) % span;
    let mut last = io::Error::other("no port was free");
    for i in 0..span {
        let port = PORTS.0 + (start + i) % span;
        for any in ["[::]", "0.0.0.0"] {
            match UdpSocket::bind(format!("{any}:{port}")) {
                Ok(socket) => return Ok(socket),
                Err(err) => last = err,
            }
        }
    }
    Err(last)
}

/// Points stdout at /dev/null, now that ssh is no longer reading it.
fn quiet_stdout() {
    if let Ok(null) = std::fs::OpenOptions::new().write(true).open("/dev/null") {
        use std::os::fd::AsRawFd;
        // SAFETY: duplicates an open descriptor onto stdout's number.
        unsafe { libc::dup2(null.as_raw_fd(), libc::STDOUT_FILENO) };
    }
}

enum Heard {
    Datagram(Vec<u8>, SocketAddr),
    Output(Vec<u8>),
    Closed,
}

struct Joined {
    data: UnixStream,
    ctl: UnixStream,
    at: Instant,
}

struct Agent {
    id: String,
    socket: UdpSocket,
    sealer: Sealer,
    opener: Opener,
    peer: Option<SocketAddr>,
    heard: Instant,
    sent: Instant,
    owed: bool,
    bye: bool,
    timing: Timing,
    out: Outbox,
    keys: Keys,
    screen: Screen,
    size: [u16; 4],
    joined: Option<Joined>,
    /// Lines scrolled off when the client last had everything.
    seen: usize,
    ended: Option<(End, Instant)>,
    tell: Sender<Heard>,
    heard_rx: mpsc::Receiver<Heard>,
}

impl Agent {
    fn new(id: &str, socket: UdpSocket, key: &[u8; KEY_LEN]) -> Self {
        let now = Instant::now();
        let (tell, heard_rx) = mpsc::channel();
        Self {
            id: id.to_owned(),
            socket,
            sealer: Sealer::new(key, Side::Agent),
            opener: Opener::new(key, Side::Client),
            peer: None,
            heard: now,
            sent: now,
            owed: false,
            bye: false,
            timing: Timing::new(now),
            out: Outbox::new(now),
            keys: Keys::default(),
            screen: Screen::new(80, 24, HISTORY),
            size: [80, 24, 0, 0],
            joined: None,
            seen: 0,
            ended: None,
            tell,
            heard_rx,
        }
    }

    fn run(&mut self) {
        let Ok(socket) = self.socket.try_clone() else {
            return;
        };
        pump(self.tell.clone(), None, move |buf| {
            let (n, from) = socket.recv_from(buf).ok()?;
            Some(Heard::Datagram(buf[..n].to_vec(), from))
        });
        loop {
            let now = Instant::now();
            match self
                .heard_rx
                .recv_timeout(self.wake(now).saturating_duration_since(now))
            {
                Ok(heard) => self.on(heard),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            while let Ok(heard) = self.heard_rx.try_recv() {
                self.on(heard);
            }
            let now = Instant::now();
            self.paint_when_due(now);
            self.send(now);
            if self.done(now) {
                return;
            }
        }
    }

    /// Whether the client got its first picture of the screen, which starts the first epoch.
    fn painted(&self) -> bool {
        self.out.epoch > 0
    }

    fn wake(&self, now: Instant) -> Instant {
        if self.peer.is_none() {
            return self.heard + NEVER_CAME;
        }
        let mut wake = self.sent + HEARTBEAT;
        if let Some(due) = self.out.due(self.timing.wait()) {
            wake = wake.min(due);
        }
        if let Some(joined) = self.joined.as_ref().filter(|_| !self.painted()) {
            wake = wake.min(joined.at + SETTLE);
        }
        if self.ended.is_some() {
            wake = wake.min(now + Duration::from_millis(200));
        }
        wake.max(now + Duration::from_millis(1))
    }

    fn done(&self, now: Instant) -> bool {
        let quiet = now.duration_since(self.heard);
        // A client that never came found UDP blocked and went on over ssh.
        self.bye
            || quiet >= IDLE
            || (self.peer.is_none() && quiet >= NEVER_CAME)
            || self.ended.is_some_and(|(_, at)| now.duration_since(at) >= LINGER)
    }

    fn on(&mut self, heard: Heard) {
        let now = Instant::now();
        match heard {
            Heard::Datagram(bytes, from) => {
                let Some(up) = self.opener.open(&bytes).and_then(|m| Up::decode(&m)) else {
                    return;
                };
                // Keys want acknowledging, and a new address or a client that asks hearing back;
                // acknowledgments themselves don't, or every one would be answered.
                self.owed |= up.answer || !up.keys.is_empty() || self.peer != Some(from);
                self.peer = Some(from);
                self.heard = now;
                self.timing.heard(up.clock, now);
                self.bye |= up.bye;
                if self.joined.is_none() && self.ended.is_none() {
                    self.join(up.size, now);
                } else if up.size != self.size {
                    self.resize(up.size);
                }
                let keys = self.keys.take(up.from, &up.keys);
                if !keys.is_empty()
                    && let Some(joined) = &mut self.joined
                {
                    let _ = joined.data.write_all(keys);
                }
                self.out.acknowledged(up.epoch, up.have, now);
                if self.out.backlog() == 0 {
                    self.seen = self.screen.history();
                }
            }
            Heard::Output(bytes) => {
                self.screen.feed(&bytes);
                if self.painted() {
                    self.out.push(&bytes);
                }
            }
            Heard::Closed => {
                let end = match self.joined.take() {
                    Some(joined) if !session::ended(&joined.ctl) => End::Taken,
                    _ => End::Exited,
                };
                self.ended = Some((end, now));
            }
        }
    }

    fn join(&mut self, size: [u16; 4], now: Instant) {
        let Ok((data, ctl)) = session::connect(&self.id) else {
            self.ended = Some((End::Exited, now));
            return;
        };
        let Ok(mut reader) = data.try_clone() else {
            return;
        };
        pump(self.tell.clone(), Some(Heard::Closed), move |buf| {
            let n = reader.read(buf).ok().filter(|&n| n > 0)?;
            Some(Heard::Output(buf[..n].to_vec()))
        });
        self.joined = Some(Joined { data, ctl, at: now });
        self.resize(size);
    }

    fn resize(&mut self, size: [u16; 4]) {
        self.size = size;
        self.screen.resize(size[0], size[1]);
        if let Some(joined) = &self.joined {
            session::resize(&joined.ctl, bridge::window_size(size));
        }
    }

    /// Starts a client off with a picture of the screen once joining settled, and starts it
    /// over with one when it fell too far behind for the output in between to be worth sending.
    fn paint_when_due(&mut self, now: Instant) {
        let settled = self.joined.as_ref().is_some_and(|j| now.duration_since(j.at) >= SETTLE);
        if !self.painted() && settled {
            self.out.restart(self.screen.paint(MOST_HISTORY), now);
            self.seen = self.screen.history();
        } else if self.painted() && self.out.backlog() > MOST_BEHIND {
            let history = self.screen.history();
            let scrolled = if history >= HISTORY {
                MOST_HISTORY
            } else {
                history.saturating_sub(self.seen).min(MOST_HISTORY)
            };
            self.out.restart(self.screen.paint(scrolled), now);
            self.seen = history;
        }
    }

    fn send(&mut self, now: Instant) {
        let Some(peer) = self.peer else {
            return;
        };
        let mut sent = false;
        while let Some((at, data)) = self.out.next(now, self.timing.wait()) {
            self.datagram(peer, at, data, End::No, now);
            sent = true;
        }
        // An end is told once all output got there, so the client has it all when it hears.
        let end = match self.ended {
            Some((end, _)) if self.out.backlog() == 0 || !self.painted() => end,
            _ => End::No,
        };
        let since = now.duration_since(self.sent);
        let due = self.owed || since >= HEARTBEAT || (end != End::No && since >= Duration::from_millis(200));
        if !sent && due {
            self.datagram(peer, 0, Vec::new(), end, now);
        }
        self.owed = false;
    }

    fn datagram(&mut self, peer: SocketAddr, at: u64, data: Vec<u8>, end: End, now: Instant) {
        let down = Down {
            clock: self.timing.stamp(now),
            keys: self.keys.have,
            epoch: self.out.epoch,
            at,
            data,
            end,
        };
        let _ = self.socket.send_to(&self.sealer.seal(&down.encode()), peer);
        self.sent = now;
    }
}
