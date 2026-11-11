//! Sessions that outlive the window. Each shell runs under a small host, `vif session-host`,
//! that holds its PTY and a screen of what it printed, behind two Unix sockets: one carries the
//! bytes both ways, the other sizes, the shell's pid and its end. Closing a window leaves the
//! hosts running; the next vif attaches to them and gets that screen painted, with what scrolled
//! off above it, the way tmux brings a session back, rather than raw output played again.

use crate::held::Held;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::PathBuf;
use std::process::{Command, ExitCode, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::tty::{self, ChildEvent, Options, Shell};
use polling::{Event, PollMode, Poller};
use serde::{Deserialize, Serialize};

use crate::screen::Screen;

/// Written after the painting a client starts with, so it knows where the painting ends and the
/// shell's own output begins; an OSC no terminal acts on.
pub const PAINTED: &[u8] = b"\x1b]6907;painted\x07";

/// Lines a host's screen keeps after they scroll off, to paint above it: at 120 columns about
/// as much memory as the two megabytes of output hosts used to keep.
const HISTORY: usize = crate::screen::MOST_HISTORY;
/// alacritty's poll keys for PTY bytes and for the child's end.
const READ_WRITE: usize = 0;
const CHILD: usize = 1;

/// How to start the shell, handed to the host on its command line.
#[derive(Serialize, Deserialize)]
pub struct Spec {
    pub program: Option<String>,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: HashMap<String, String>,
    pub size: (u16, u16, u16, u16),
}

pub fn dir() -> PathBuf {
    crate::private::runtime().join("vif-sessions")
}

fn socket(id: &str, kind: &str) -> PathBuf {
    dir().join(format!("{id}.{kind}"))
}

/// Starts a host for a new shell; returns its id once it listens.
pub fn start(spec: &Spec) -> io::Result<String> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let id = format!("{:x}", (nanos as u64) ^ (u64::from(std::process::id()) << 40));
    start_as(spec, &id)?;
    Ok(id)
}

/// Whether a host answers for `id`.
pub fn alive(id: &str) -> bool {
    hello(id).is_ok()
}

/// Starts a host under an id of the caller's choosing, and waits until it listens.
pub fn start_as(spec: &Spec, id: &str) -> io::Result<()> {
    crate::private::made("vif-sessions")?;
    let spec = serde_json::to_string(spec).map_err(io::Error::other)?;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(["session-host", id, &spec])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Its own session, so closing vif sends it no hangup.
    detach(&mut command);
    let mut child = command.spawn()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while !socket(id, "ctl").exists() {
        if child.try_wait()?.is_some() || Instant::now() > deadline {
            return Err(io::Error::other("the session host didn't start"));
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// Has `command` start in a session of its own, so its parent's terminal going away sends it
/// no hangup.
pub fn detach(command: &mut Command) {
    // SAFETY: setsid is async-signal-safe, which is all that may run between fork and exec.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
}

/// Why a host stopped sending to a client holding `ctl`: true when its shell ended, which the
/// host says on every control connection before it closes, false when another client took the
/// session over and the control connection stays quiet.
pub fn ended(mut ctl: &UnixStream) -> bool {
    let _ = ctl.set_nonblocking(true);
    let mut said = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        match ctl.read(&mut buf) {
            Ok(n) if n > 0 => said.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
            _ => return true,
        }
    }
    String::from_utf8_lossy(&said).lines().any(|l| l.starts_with("exit "))
}

/// Tells a host its terminal's new size, over its control socket.
pub fn resize(mut ctl: &UnixStream, size: WindowSize) {
    let line = format!(
        "resize {} {} {} {}\n",
        size.num_cols, size.num_lines, size.cell_width, size.cell_height
    );
    let _ = ctl.write_all(line.as_bytes());
}

/// Whether a host's control socket is there, without asking it anything.
pub fn listening(id: &str) -> bool {
    socket(id, "ctl").exists()
}

/// The data and control sockets of a running host, for a bridge that isn't a window.
pub fn connect(id: &str) -> io::Result<(UnixStream, UnixStream)> {
    let (ctl, _) = hello(id)?;
    Ok((UnixStream::connect(socket(id, "data"))?, ctl))
}

/// What a host says first: its shell, that shell's terminal, whether a window has it, and since
/// when none has.
struct Hello {
    pid: u32,
    tty: PathBuf,
    attached: bool,
    since: Option<SystemTime>,
}

fn hello(id: &str) -> io::Result<(UnixStream, Hello)> {
    let ctl = UnixStream::connect(socket(id, "ctl"))?;
    ctl.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut line = String::new();
    BufReader::new(ctl.try_clone()?).read_line(&mut line)?;
    let mut words = line.split_whitespace();
    let parsed = match (words.next(), words.next(), words.next(), words.next()) {
        (Some("hello"), Some(pid), Some(tty), Some(attached)) => pid.parse().ok().map(|pid| Hello {
            pid,
            tty: PathBuf::from(tty),
            attached: attached == "1",
            since: words
                .next()
                .and_then(|s| s.parse().ok())
                .filter(|s| *s > 0)
                .map(|s| UNIX_EPOCH + Duration::from_secs(s)),
        }),
        _ => None,
    };
    let hello = parsed.ok_or_else(|| io::Error::other("the session host said something odd"))?;
    ctl.set_read_timeout(None)?;
    Ok((ctl, hello))
}

/// Opens a session's terminal just long enough to ask it something.
pub fn open_tty(path: &std::path::Path) -> Option<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOCTTY | libc::O_NONBLOCK)
        .open(path)
        .ok()
}

/// Sessions no window has, oldest first; hosts that are gone get their sockets cleared.
pub fn free() -> Vec<String> {
    let mut found: Vec<(SystemTime, String)> = Vec::new();
    for entry in fs::read_dir(dir()).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "ctl") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else {
            continue;
        };
        match hello(&id) {
            Ok((_, hello)) if !hello.attached => {
                let born = entry.metadata().and_then(|m| m.modified()).unwrap_or(UNIX_EPOCH);
                found.push((born, id));
            }
            Ok(_) => {}
            Err(_) => {
                let _ = fs::remove_file(&path);
                let _ = fs::remove_file(socket(&id, "data"));
                let _ = fs::remove_file(dir().join(format!("{id}.name")));
            }
        }
    }
    found.sort();
    found.into_iter().map(|(_, id)| id).collect()
}

/// A session as the sessions list shows it.
pub struct Shown {
    pub id: String,
    pub name: Option<String>,
    pub what: String,
    pub attached: bool,
    pub since: Option<SystemTime>,
}

/// Every session there is, newest first.
pub fn all() -> Vec<Shown> {
    let mut found: Vec<(SystemTime, Shown)> = Vec::new();
    for entry in fs::read_dir(dir()).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "ctl") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else {
            continue;
        };
        let Ok((_, hello)) = hello(&id) else {
            continue;
        };
        let born = entry.metadata().and_then(|m| m.modified()).unwrap_or(UNIX_EPOCH);
        found.push((
            born,
            Shown {
                name: name(&id),
                what: describe(&id),
                attached: hello.attached,
                since: hello.since,
                id,
            },
        ));
    }
    found.sort_by_key(|(born, _)| std::cmp::Reverse(*born));
    found.into_iter().map(|(_, s)| s).collect()
}

/// The last `bytes` a session printed, without taking it from the window that has it.
pub fn peek(id: &str, bytes: usize) -> Option<Vec<u8>> {
    let (ctl, _) = hello(id).ok()?;
    ctl.set_read_timeout(Some(Duration::from_millis(500))).ok()?;
    (&ctl).write_all(format!("peek {bytes}\n").as_bytes()).ok()?;
    let mut reader = BufReader::new(&ctl);
    let mut head = String::new();
    reader.read_line(&mut head).ok()?;
    let len: usize = head.trim().strip_prefix("peek ")?.parse().ok()?;
    let mut out = vec![0; len];
    reader.read_exact(&mut out).ok()?;
    Some(out)
}

/// A session's name, if it was given one.
pub fn name(id: &str) -> Option<String> {
    fs::read_to_string(dir().join(format!("{id}.name")))
        .ok()
        .map(|n| n.trim().to_owned())
        .filter(|n| !n.is_empty())
}

pub fn rename(id: &str, name: &str) -> io::Result<()> {
    let path = dir().join(format!("{id}.name"));
    if name.trim().is_empty() {
        return fs::remove_file(path).or(Ok(()));
    }
    fs::write(path, name.trim())
}

/// The session a name or an id stands for.
pub fn find(name_or_id: &str) -> Option<String> {
    let all = all();
    all.iter()
        .find(|s| s.id == name_or_id)
        .or_else(|| all.iter().find(|s| s.name.as_deref() == Some(name_or_id)))
        .map(|s| s.id.clone())
}

/// A session in words: its folder, and what runs there unless that is the shell itself.
pub fn describe(id: &str) -> String {
    let Some((pid, tty)) = hello(id).ok().and_then(|(_, h)| Some((h.pid, open_tty(&h.tty)?))) else {
        return id.to_owned();
    };
    let place = crate::process::cwd(&tty, pid).map_or_else(|| id.to_owned(), |p| crate::tilde(&p));
    match crate::process::command(&tty, pid).filter(|argv| !crate::process::is_shell(argv)) {
        Some(argv) => format!("{place} · {}", argv.join(" ")),
        None => place,
    }
}

/// A window's end of a session: what alacritty's event loop needs from a PTY.
pub struct Link {
    data: File,
    ctl: UnixStream,
    pending: Vec<u8>,
    pub pid: u32,
    /// The shell's terminal, for asking what runs in front. Held open, it would keep the host
    /// from ever seeing the shell end.
    pub tty: PathBuf,
}

impl Link {
    pub fn attach(id: &str) -> io::Result<Self> {
        let (ctl, hello) = hello(id)?;
        let data = UnixStream::connect(socket(id, "data"))?;
        data.set_nonblocking(true)?;
        ctl.set_nonblocking(true)?;
        Ok(Self {
            data: File::from(OwnedFd::from(data)),
            ctl,
            pending: Vec::new(),
            pid: hello.pid,
            tty: hello.tty,
        })
    }

    pub fn bytes(&self) -> io::Result<File> {
        self.data.try_clone()
    }

    /// A handle that can end the session from outside the event loop.
    pub fn control(&self) -> io::Result<UnixStream> {
        self.ctl.try_clone()
    }

    /// # Safety
    /// The sockets must outlive their registration, as with alacritty's own PTY.
    pub unsafe fn register(&mut self, poll: &Arc<Poller>, mut interest: Event, mode: PollMode) -> io::Result<()> {
        interest.key = READ_WRITE;
        // SAFETY: both sources live in self, which the caller keeps registered no longer than it lives.
        unsafe {
            poll.add_with_mode(&self.data, interest, mode)?;
            poll.add_with_mode(&self.ctl, Event::readable(CHILD), PollMode::Level)
        }
    }

    pub fn reregister(&mut self, poll: &Arc<Poller>, mut interest: Event, mode: PollMode) -> io::Result<()> {
        interest.key = READ_WRITE;
        poll.modify_with_mode(&self.data, interest, mode)?;
        poll.modify_with_mode(&self.ctl, Event::readable(CHILD), PollMode::Level)
    }

    pub fn deregister(&mut self, poll: &Arc<Poller>) -> io::Result<()> {
        poll.delete(&self.data)?;
        poll.delete(&self.ctl)
    }

    pub fn writer(&mut self) -> &mut File {
        &mut self.data
    }

    pub fn next_child_event(&mut self) -> Option<ChildEvent> {
        let mut buf = [0u8; 256];
        loop {
            match self.ctl.read(&mut buf) {
                Ok(0) => return Some(ChildEvent::Exited(None)),
                Ok(n) => self.pending.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => return Some(ChildEvent::Exited(None)),
            }
        }
        let text = String::from_utf8_lossy(&self.pending).into_owned();
        let code = text
            .lines()
            .find_map(|l| l.strip_prefix("exit ")?.trim().parse::<i32>().ok())?;
        Some(ChildEvent::Exited(Some(ExitStatus::from_raw(code << 8))))
    }
}

impl OnResize for Link {
    fn on_resize(&mut self, size: WindowSize) {
        resize(&self.ctl, size);
    }
}

struct State {
    screen: Screen,
    /// The window that has the session now, by its turn number.
    client: Option<(u64, UnixStream)>,
    turns: u64,
    /// Control connections open now, by number, to tell when the shell ends.
    controls: Vec<(u64, UnixStream)>,
    connections: u64,
    /// When the last window let go of it, in seconds since 1970.
    since: u64,
}

impl State {
    fn new(cols: u16, lines: u16) -> Self {
        Self {
            screen: Screen::new(cols, lines, HISTORY),
            client: None,
            turns: 0,
            controls: Vec::new(),
            connections: 0,
            since: 0,
        }
    }
}

/// `vif session-host ID SPEC`: holds one shell until it ends.
pub fn host(args: &[String]) -> ExitCode {
    match serve(args) {
        Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
        Err(err) => {
            eprintln!("vif session-host: {err}");
            ExitCode::FAILURE
        }
    }
}

fn serve(args: &[String]) -> io::Result<i32> {
    let (Some(id), Some(spec)) = (args.first(), args.get(1)) else {
        return Err(io::Error::other("usage: vif session-host ID SPEC"));
    };
    crate::private::made("vif-sessions")?;
    let spec: Spec = serde_json::from_str(spec).map_err(io::Error::other)?;
    let (cols, lines, cell_width, cell_height) = spec.size;
    let options = Options {
        shell: spec.program.map(|program| Shell::new(program, spec.args)),
        working_directory: spec.cwd,
        drain_on_exit: true,
        env: spec.env,
    };
    let size = WindowSize {
        num_lines: lines,
        num_cols: cols,
        cell_width,
        cell_height,
    };
    let pty = tty::new(&options, size, 0)?;
    let pid = pty.child().id();
    let mut master = pty.file().try_clone()?;
    let input = pty.file().try_clone()?;
    // This host reads with blocking calls on threads of its own.
    // SAFETY: fcntl on an fd we own, its flags read before one is cleared.
    unsafe {
        let fd = master.as_raw_fd();
        libc::fcntl(fd, libc::F_SETFL, libc::fcntl(fd, libc::F_GETFL) & !libc::O_NONBLOCK);
    }
    let tty_path = slave_name(&master).unwrap_or_default();
    let pty = Arc::new(Mutex::new(pty));
    let state = Arc::new(Mutex::new(State::new(cols, lines)));

    // The machine going down stops every process: what this shell printed last is kept, so the
    // window that comes back after the restart can show it above a new shell. The shell, started
    // before this, keeps SIGTERM as usual.
    let (kept, keep_id) = (state.clone(), id.clone());
    crate::tty::on_signal(libc::SIGTERM, move || {
        let painted = kept.held().screen.paint(HISTORY);
        keep_output(&keep_id, &painted);
        // SAFETY: kill only sends a signal, to the shell this host started.
        unsafe { libc::kill(pid as i32, libc::SIGHUP) };
    });

    let (data_path, ctl_path) = (socket(id, "data"), socket(id, "ctl"));
    let data = UnixListener::bind(&data_path)?;
    let ctl = UnixListener::bind(&ctl_path)?;

    let shared = state.clone();
    std::thread::spawn(move || {
        for stream in data.incoming().flatten() {
            let Ok(reader) = stream.try_clone() else {
                continue;
            };
            let mut s = shared.held();
            let mut stream = stream;
            let painted = stream
                .write_all(&s.screen.paint(HISTORY))
                .and_then(|()| stream.write_all(PAINTED));
            if painted.is_err() {
                continue;
            }
            if let Some((_, old)) = s.client.take() {
                let _ = old.shutdown(std::net::Shutdown::Both);
            }
            s.turns += 1;
            let turn = s.turns;
            s.client = Some((turn, stream));
            drop(s);
            let (shared, mut input) = (shared.clone(), input.try_clone().expect("pty"));
            std::thread::spawn(move || {
                let mut reader = reader;
                let mut buf = [0u8; 8192];
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 || input.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
                let mut s = shared.held();
                if s.client.as_ref().is_some_and(|(t, _)| *t == turn) {
                    s.client = None;
                    s.since = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
                }
            });
        }
    });

    let (shared, sizes) = (state.clone(), pty.clone());
    std::thread::spawn(move || {
        for stream in ctl.incoming().flatten() {
            let (attached, since) = {
                let s = shared.held();
                (u8::from(s.client.is_some()), s.since)
            };
            let mut stream = stream;
            let hello = format!("hello {pid} {} {attached} {since}\n", tty_path.display());
            if stream.write_all(hello.as_bytes()).is_err() {
                continue;
            }
            let number = {
                let mut s = shared.held();
                s.connections += 1;
                let number = s.connections;
                if let Ok(copy) = stream.try_clone() {
                    s.controls.push((number, copy));
                }
                number
            };
            let (sizes, ring) = (sizes.clone(), shared.clone());
            let Ok(mut reply) = stream.try_clone() else {
                continue;
            };
            std::thread::spawn(move || {
                for line in BufReader::new(stream).lines() {
                    let Ok(line) = line else {
                        break;
                    };
                    let words: Vec<u16> = line.split_whitespace().skip(1).filter_map(|w| w.parse().ok()).collect();
                    if line.starts_with("resize ")
                        && let [cols, lines, cw, ch] = words[..]
                    {
                        ring.held().screen.resize(cols, lines);
                        sizes.held().on_resize(WindowSize {
                            num_lines: lines,
                            num_cols: cols,
                            cell_width: cw,
                            cell_height: ch,
                        });
                    } else if line == "kill" {
                        // SAFETY: kill only sends a signal, to the shell this host started.
                        unsafe { libc::kill(pid as i32, libc::SIGHUP) };
                    } else if line.starts_with("peek ") {
                        // What's on the screen, however many bytes were asked for: a painting
                        // cut short would leave a sequence half done.
                        let tail = ring.held().screen.paint(0);
                        let head = format!("peek {}\n", tail.len());
                        if reply
                            .write_all(head.as_bytes())
                            .and_then(|()| reply.write_all(&tail))
                            .is_err()
                        {
                            break;
                        }
                    }
                }
                // A window asks and goes many times; each one gone lets go of its socket.
                ring.held().controls.retain(|(n, _)| *n != number);
            });
        }
    });

    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match master.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let mut s = state.held();
        s.screen.feed(&buf[..n]);
        if let Some((_, client)) = s.client.as_mut()
            && client.write_all(&buf[..n]).is_err()
        {
            s.client = None;
        }
    }
    let mut status = 0;
    // SAFETY: status is ours, and pid is our own child.
    unsafe { libc::waitpid(pid as i32, &mut status, 0) };
    let code = if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else {
        1
    };
    let _ = fs::remove_file(&ctl_path);
    let _ = fs::remove_file(&data_path);
    let _ = fs::remove_file(dir().join(format!("{id}.name")));
    let mut s = state.held();
    for (_, control) in &mut s.controls {
        let _ = control.write_all(format!("exit {code}\n").as_bytes());
    }
    Ok(code)
}

/// Writes a painting of a session's screen where [`crate::layouts::last_output`] finds it,
/// readable by this user alone and with secrets taken out. The modes it leaves are undone, since
/// a new shell comes up under it.
fn keep_output(id: &str, painted: &[u8]) {
    let mut text = crate::secrets::hide(&String::from_utf8_lossy(painted));
    text.push_str(&String::from_utf8_lossy(&crate::screen::reset()));
    let written = crate::layouts::kept_made().and_then(|dir| {
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(dir.join(format!("{id}.ring")))?
            .write_all(text.as_bytes())
    });
    if let Err(err) = written {
        eprintln!("vif session-host: can't keep the output: {err}");
    }
}

/// The terminal's own end of `master`, asked in a way that's safe from any thread: ptsname
/// shares one buffer between all of them, and the host runs several.
fn slave_name(master: &File) -> Option<PathBuf> {
    let mut buf = [0 as libc::c_char; 128];
    #[cfg(target_os = "linux")]
    // SAFETY: the buffer is ours and its length is passed with it.
    let ok = unsafe { libc::ptsname_r(master.as_raw_fd(), buf.as_mut_ptr(), buf.len()) } == 0;
    #[cfg(target_os = "macos")]
    // SAFETY: TIOCPTYGNAME writes at most 128 bytes, the buffer's size, ending in a nul.
    let ok = unsafe { libc::ioctl(master.as_raw_fd(), libc::TIOCPTYGNAME as _, buf.as_mut_ptr()) } == 0;
    if !ok {
        return None;
    }
    // SAFETY: both calls leave a nul-terminated string in the buffer when they succeed.
    let name = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) };
    Some(PathBuf::from(name.to_string_lossy().into_owned()))
}
