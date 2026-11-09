//! This machine's end of a session over UDP. ssh is only the way in: it starts the agent over
//! there and brings back its port and key. From then on keys go to the agent and output comes
//! back as datagrams, from whatever address this machine has at the moment, so a laptop that
//! sleeps or changes networks finds its shell where it was. Output passes through untouched, so
//! the terminal here keeps its own scrollback. When the agent stays silent long enough to have
//! gone, ssh starts another without asking anything.

use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use super::agent::GREETING;
use super::flow::{Inbox, Timing, Typed};
use super::netwatch::Changes;
use super::predict::Guesses;
use super::wire::{Down, End, KEY_LEN, Opener, Sealer, Side, Up};
use super::{Link, TAKEN, bridge, explain, pump};
use crate::screen::{self, Screen};

/// How long to wait for the agent's first word before taking it that UDP can't get through.
const FIRST_WORD: Duration = Duration::from_secs(3);
const HEARTBEAT: Duration = Duration::from_secs(3);
/// Silence after which the bar at the top says so.
const QUIET: Duration = Duration::from_secs(6);
/// Silence after which the agent is taken to be gone and ssh asked for another.
const GONE: Duration = Duration::from_secs(20);
/// ctrl-^, which with `.` after it leaves the session running and this end.
const ESCAPE: u8 = 0x1e;

pub(super) enum Outcome {
    Done(ExitCode),
    /// UDP isn't to be had, and why when the user should hear it.
    Instead(Option<String>),
}

/// Where an agent listens and the key it expects.
struct Invite {
    port: u16,
    key: [u8; KEY_LEN],
}

enum Refused {
    /// No vif over there.
    Missing,
    /// A vif too old to have an agent, or one that couldn't start it.
    Unable,
    /// ssh's own failure, with its code.
    Ssh(i32),
}

/// Asks ssh to start an agent; `quiet` asks nothing of the user and shows nothing.
fn invite(link: &Link, quiet: bool) -> Result<Invite, Refused> {
    let mut command = Command::new(&link.ssh);
    command.args(link.keepalive());
    if quiet {
        command
            .args(["-o", "BatchMode=yes"])
            .stdin(Stdio::null())
            .stderr(Stdio::null());
    }
    let out = command
        .args(["-T", &link.host, &link.there(&format!("udp-agent {}", link.name))])
        .stdout(Stdio::piped())
        .output()
        .map_err(|_| Refused::Ssh(255))?;
    match out.status.code() {
        Some(0) => parse(&String::from_utf8_lossy(&out.stdout)).ok_or(Refused::Unable),
        Some(127) => Err(Refused::Missing),
        Some(255) | None => Err(Refused::Ssh(255)),
        Some(_) => Err(Refused::Unable),
    }
}

fn parse(said: &str) -> Option<Invite> {
    let line = said.lines().find(|l| l.starts_with(GREETING))?;
    let mut words = line[GREETING.len()..].split_whitespace();
    let port = words.next()?.parse().ok()?;
    let key = STANDARD.decode(words.next()?).ok()?.try_into().ok()?;
    Some(Invite { port, key })
}

/// Networks UDP didn't get through from to a host, since this machine started: the host and
/// the address this machine left from, so another network tries again.
fn blocked_file() -> std::path::PathBuf {
    crate::private::runtime().join("vif-udp-blocked")
}

fn blocked_line(host: &str, ip: IpAddr) -> String {
    format!(
        "{host} {}",
        super::ssh::source(ip).map_or_else(String::new, |s| s.to_string())
    )
}

fn blocked(line: &str) -> bool {
    std::fs::read_to_string(blocked_file()).is_ok_and(|s| s.lines().any(|l| l == line))
}

fn remember_blocked(line: &str, blocked: bool) {
    let path = blocked_file();
    let mut lines: Vec<String> = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .lines()
        .filter(|l| *l != line)
        .map(str::to_owned)
        .collect();
    if blocked {
        lines.push(line.to_owned());
    }
    if crate::private::runtime_made().is_ok() {
        let _ = std::fs::write(path, lines.iter().map(|l| format!("{l}\n")).collect::<String>());
    }
}

pub(super) fn run(link: &Link, insist: bool) -> Outcome {
    let Some(ip) = link.address() else {
        let why = link.proxied().then(|| "ssh reaches it through another host".to_owned());
        return Outcome::Instead(why);
    };
    let network = blocked_line(&link.host, ip);
    if !insist && blocked(&network) {
        return Outcome::Instead(None);
    }
    let invite = match invite(link, false) {
        Ok(invite) => invite,
        Err(Refused::Missing) => return Outcome::Done(explain(link, 127)),
        Err(Refused::Ssh(code)) => return Outcome::Done(explain(link, code)),
        Err(Refused::Unable) => return Outcome::Instead(Some("the vif there has no UDP agent".to_owned())),
    };
    let socket = match UdpSocket::bind(if ip.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" }) {
        Ok(socket) => socket,
        Err(err) => return Outcome::Instead(Some(format!("can't open a UDP socket: {err}"))),
    };
    let mut client = Client::new(link, socket, ip, invite);
    let through = client.first_word();
    remember_blocked(&network, !through);
    if !through {
        let port = client.peer.port();
        return Outcome::Instead(Some(format!(
            "no UDP reply from port {port}; a firewall may be in the way"
        )));
    }
    Outcome::Done(client.run())
}

enum Heard {
    Datagram(Vec<u8>),
    Keys(Vec<u8>),
    Closed,
    Resized,
    Moved,
    Invited(Option<Invite>),
}

struct Client<'a> {
    link: &'a Link,
    socket: UdpSocket,
    peer: SocketAddr,
    sealer: Sealer,
    opener: Opener,
    timing: Timing,
    inbox: Inbox,
    typed: Typed,
    screen: Screen,
    guesses: Guesses,
    size: [u16; 4],
    heard: Instant,
    sent: Instant,
    /// Output came that wants acknowledging.
    owed: bool,
    /// The agent should answer the next datagram even with nothing to acknowledge.
    ask: bool,
    escape: bool,
    /// What goes to the terminal, written once each time round.
    out: Vec<u8>,
    /// When the bar about silence was last drawn.
    told: Option<Instant>,
    asking: Option<Instant>,
    tell: Sender<Heard>,
    heard_rx: mpsc::Receiver<Heard>,
}

impl<'a> Client<'a> {
    fn new(link: &'a Link, socket: UdpSocket, ip: IpAddr, invite: Invite) -> Self {
        let now = Instant::now();
        let size = bridge::size();
        let (tell, heard_rx) = mpsc::channel();
        if let Ok(socket) = socket.try_clone() {
            pump(tell.clone(), None, move |buf| {
                let n = socket.recv(buf).ok()?;
                Some(Heard::Datagram(buf[..n].to_vec()))
            });
        }
        Self {
            link,
            socket,
            peer: SocketAddr::new(ip, invite.port),
            sealer: Sealer::new(&invite.key, Side::Client),
            opener: Opener::new(&invite.key, Side::Agent),
            timing: Timing::new(now),
            inbox: Inbox::default(),
            typed: Typed::default(),
            screen: Screen::new(size[0], size[1], 0),
            guesses: Guesses::default(),
            size,
            heard: now,
            sent: now,
            owed: false,
            ask: true,
            escape: false,
            out: Vec::new(),
            told: None,
            asking: None,
            tell,
            heard_rx,
        }
    }

    fn open(&mut self, datagram: &[u8]) -> Option<Down> {
        self.opener.open(datagram).and_then(|m| Down::decode(&m))
    }

    /// Says hello until the agent answers; false when it never does.
    fn first_word(&mut self) -> bool {
        let deadline = Instant::now() + FIRST_WORD;
        while Instant::now() < deadline {
            self.ask = true;
            self.datagram(Instant::now(), false);
            let until = Instant::now() + Duration::from_millis(300);
            while let Ok(heard) = self
                .heard_rx
                .recv_timeout(until.saturating_duration_since(Instant::now()))
            {
                if let Heard::Datagram(bytes) = heard
                    && let Some(down) = self.open(&bytes)
                {
                    self.heard(down);
                    self.flush();
                    return true;
                }
            }
        }
        false
    }

    fn run(&mut self) -> ExitCode {
        // SAFETY: cfmakeraw only changes the flags of the termios it's given.
        let raw = crate::tty::Modes::change(|t| unsafe { libc::cfmakeraw(t) });
        self.listen();
        let code = loop {
            let now = Instant::now();
            let mut ended = match self
                .heard_rx
                .recv_timeout(self.wake(now).saturating_duration_since(now))
            {
                Ok(heard) => self.on(heard),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => Some(1),
            };
            while ended.is_none()
                && let Ok(heard) = self.heard_rx.try_recv()
            {
                ended = self.on(heard);
            }
            if let Some(code) = ended {
                break code;
            }
            let now = Instant::now();
            self.send(now);
            self.mind_silence(now);
            self.guesses.expire(&self.screen, now, &mut self.out);
            self.flush();
        };
        self.out.extend_from_slice(&screen::reset());
        self.flush();
        drop(raw);
        match code {
            LEFT => {
                eprintln!(
                    "vif: left {name} running on {host}; `vif ssh {host} {name}` comes back to it",
                    name = self.link.name,
                    host = self.link.host
                );
                ExitCode::SUCCESS
            }
            code => explain(self.link, code),
        }
    }

    fn flush(&mut self) {
        if !self.out.is_empty() {
            let mut stdout = io::stdout().lock();
            let _ = stdout.write_all(&self.out).and_then(|()| stdout.flush());
            self.out.clear();
        }
    }

    /// Keys, size changes and network changes, each on a thread of its own.
    fn listen(&self) {
        let mut stdin = io::stdin();
        pump(self.tell.clone(), Some(Heard::Closed), move |buf| {
            let n = stdin.read(buf).ok().filter(|&n| n > 0)?;
            Some(Heard::Keys(buf[..n].to_vec()))
        });
        let tell = self.tell.clone();
        crate::tty::on_signal(libc::SIGWINCH, move || {
            let _ = tell.send(Heard::Resized);
        });
        if let Ok(changes) = Changes::watch() {
            pump(self.tell.clone(), None, move |_| {
                changes.next().ok().map(|()| Heard::Moved)
            });
        }
    }

    fn wake(&self, now: Instant) -> Instant {
        let mut wake = (self.sent + HEARTBEAT).min(self.heard + QUIET);
        if let Some(told) = self.told {
            wake = wake.min(told + Duration::from_secs(1));
        }
        if !self.typed.is_empty() {
            wake = wake.min(self.sent + self.timing.wait());
        }
        if let Some(due) = self.guesses.due() {
            wake = wake.min(due);
        }
        wake.max(now + Duration::from_millis(1))
    }

    /// Acts on what came; the code to end with, when it ended.
    fn on(&mut self, heard: Heard) -> Option<i32> {
        match heard {
            Heard::Datagram(bytes) => {
                let down = self.open(&bytes)?;
                return self.heard(down);
            }
            Heard::Keys(keys) => return self.typed_keys(&keys),
            Heard::Closed => return Some(LEFT),
            Heard::Resized => {
                self.size = bridge::size();
                self.screen.resize(self.size[0], self.size[1]);
                self.ask = true;
            }
            Heard::Moved => self.ask = true,
            Heard::Invited(invite) => {
                self.asking = None;
                if let Some(invite) = invite {
                    self.start_over(invite);
                }
            }
        }
        None
    }

    fn heard(&mut self, down: Down) -> Option<i32> {
        let now = Instant::now();
        self.heard = now;
        self.timing.heard(down.clock, now);
        self.typed.acknowledged(down.keys);
        if self.told.take().is_some() {
            self.out.extend_from_slice(&self.screen.repaint_line(0));
        }
        // Output, new or not, is acknowledged, or a piece sent twice would never be.
        self.owed |= !down.data.is_empty();
        if let Some(fresh) = self.inbox.take(down.epoch, down.at, &down.data) {
            self.guesses.before_output(&self.screen, &mut self.out);
            self.out.extend_from_slice(fresh);
            self.screen.feed(fresh);
            self.guesses.after_output(&self.screen, &mut self.out);
        }
        match down.end {
            End::No => None,
            End::Exited => Some(0),
            End::Taken => Some(TAKEN),
        }
    }

    fn typed_keys(&mut self, keys: &[u8]) -> Option<i32> {
        let mut send = Vec::with_capacity(keys.len());
        for &key in keys {
            if self.escape {
                self.escape = false;
                match key {
                    b'.' => {
                        self.bye();
                        return Some(LEFT);
                    }
                    ESCAPE => send.push(ESCAPE),
                    _ => send.extend_from_slice(&[ESCAPE, key]),
                }
            } else if key == ESCAPE {
                self.escape = true;
            } else {
                send.push(key);
            }
        }
        if !send.is_empty() {
            let now = Instant::now();
            let trip = self.timing.round_trip();
            self.guesses.typed(&send, &self.screen, trip, now, &mut self.out);
            self.typed.push(&send);
            self.datagram(now, true);
        }
        None
    }

    fn send(&mut self, now: Instant) {
        let since = now.duration_since(self.sent);
        let resend = !self.typed.is_empty() && since >= self.timing.wait();
        if resend {
            self.datagram(now, true);
        } else if self.owed || self.ask || since >= HEARTBEAT {
            self.datagram(now, false);
        }
    }

    /// What every datagram from here says, keys aside.
    fn up(&mut self, now: Instant) -> Up {
        Up {
            clock: self.timing.stamp(now),
            epoch: self.inbox.epoch,
            have: self.inbox.have,
            size: self.size,
            from: self.typed.base(),
            answer: std::mem::take(&mut self.ask),
            ..Up::default()
        }
    }

    fn seal(&mut self, up: &Up) {
        let _ = self.socket.send_to(&self.sealer.seal(&up.encode()), self.peer);
    }

    /// Acknowledges output, and sends the keys still waiting along when `keys`.
    fn datagram(&mut self, now: Instant, keys: bool) {
        let up = self.up(now);
        if keys && !self.typed.is_empty() {
            let pieces: Vec<(u64, Vec<u8>)> = self.typed.pieces().map(|(f, k)| (f, k.to_vec())).collect();
            for (from, keys) in pieces {
                self.seal(&Up {
                    from,
                    keys,
                    ..up.clone()
                });
            }
        } else {
            self.seal(&up);
        }
        self.sent = now;
        self.owed = false;
    }

    /// Tells the agent this end is going, a few times since any of them may be lost.
    fn bye(&mut self) {
        let up = Up {
            bye: true,
            ..self.up(Instant::now())
        };
        for _ in 0..3 {
            self.seal(&up);
        }
    }

    /// Says so at the top when the agent has been quiet a while, and asks ssh for a new one
    /// once it seems gone.
    fn mind_silence(&mut self, now: Instant) {
        let quiet = now.duration_since(self.heard);
        if quiet < QUIET {
            return;
        }
        if self
            .told
            .is_none_or(|t| now.duration_since(t) >= Duration::from_secs(1))
        {
            let bar = format!(
                "\x1b7\x1b[1;1H\x1b[0;7m vif: no word from {} for {}s · ctrl-^ . leaves it running \x1b[0m\x1b[K\x1b8",
                self.link.host,
                quiet.as_secs()
            );
            self.out.extend_from_slice(bar.as_bytes());
            self.told = Some(now);
        }
        let again = self.asking.is_none_or(|t| now.duration_since(t) >= GONE);
        if quiet >= GONE && again {
            self.asking = Some(now);
            let (link, tell) = (self.link.clone(), self.tell.clone());
            std::thread::spawn(move || {
                let _ = tell.send(Heard::Invited(invite(&link, true).ok()));
            });
        }
    }

    /// Talks to a new agent: its key, its port, and output and keys from its own beginning.
    fn start_over(&mut self, invite: Invite) {
        let now = Instant::now();
        self.peer.set_port(invite.port);
        self.sealer = Sealer::new(&invite.key, Side::Client);
        self.opener = Opener::new(&invite.key, Side::Agent);
        self.timing = Timing::new(now);
        self.inbox = Inbox::default();
        self.typed.rebase();
        self.ask = true;
    }
}

/// What the loop ends with when this end left and the session goes on.
const LEFT: i32 = -1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_invitation_is_read_from_what_the_agent_said() {
        let key = [7u8; KEY_LEN];
        let said = format!("motd noise\n{GREETING} 60123 {}\n", STANDARD.encode(key));
        let invite = parse(&said).unwrap();
        assert_eq!((invite.port, invite.key), (60123, key));
        assert!(parse("vif-udp 1 60123 short").is_none());
        assert!(parse("usage: vif ...").is_none());
    }
}
