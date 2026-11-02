//! Sessions that outlive the window. Each shell runs under a small host, `vif session-host`,
//! that holds its PTY and the last two megabytes of what it printed, behind two Unix sockets:
//! one carries the bytes both ways, the other sizes, the shell's pid and its end. Closing a
//! window leaves the hosts running; the next vif attaches to them and replays what they kept.

use std::collections::{HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
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

const KEEP: usize = 2 << 20;
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
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("vif-{}", unsafe { libc::getuid() })));
    base.join("vif-sessions")
}

fn socket(id: &str, kind: &str) -> PathBuf {
    dir().join(format!("{id}.{kind}"))
}

/// Starts a host for a new shell; returns its id once it listens.
pub fn start(spec: &Spec) -> io::Result<String> {
    fs::create_dir_all(dir())?;
    fs::set_permissions(dir(), fs::Permissions::from_mode(0o700))?;
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let id = format!("{:x}", (nanos as u64) ^ (u64::from(std::process::id()) << 40));
    let spec = serde_json::to_string(spec).map_err(io::Error::other)?;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(["session-host", &id, &spec])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Its own session, so closing vif sends it no hangup.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = command.spawn()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while !socket(&id, "ctl").exists() {
        if child.try_wait()?.is_some() || Instant::now() > deadline {
            return Err(io::Error::other("the session host didn't start"));
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    std::thread::spawn(move || child.wait());
    Ok(id)
}

/// What a host says first: its shell, that shell's terminal, and whether a window has it.
struct Hello {
    pid: u32,
    tty: PathBuf,
    attached: bool,
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
            }
        }
    }
    found.sort();
    found.into_iter().map(|(_, id)| id).collect()
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
        let line = format!(
            "resize {} {} {} {}\n",
            size.num_cols, size.num_lines, size.cell_width, size.cell_height
        );
        let _ = self.ctl.write_all(line.as_bytes());
    }
}

#[derive(Default)]
struct State {
    ring: VecDeque<u8>,
    /// The window that has the session now, by its turn number.
    client: Option<(u64, UnixStream)>,
    turns: u64,
    controls: Vec<UnixStream>,
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
    fs::create_dir_all(dir())?;
    fs::set_permissions(dir(), fs::Permissions::from_mode(0o700))?;
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
    unsafe {
        let fd = master.as_raw_fd();
        libc::fcntl(fd, libc::F_SETFL, libc::fcntl(fd, libc::F_GETFL) & !libc::O_NONBLOCK);
    }
    let tty_path = slave_name(&master).unwrap_or_default();
    let pty = Arc::new(Mutex::new(pty));
    let state = Arc::new(Mutex::new(State::default()));

    let (data_path, ctl_path) = (socket(id, "data"), socket(id, "ctl"));
    let data = UnixListener::bind(&data_path)?;
    let ctl = UnixListener::bind(&ctl_path)?;

    let shared = state.clone();
    std::thread::spawn(move || {
        for stream in data.incoming().flatten() {
            let Ok(reader) = stream.try_clone() else {
                continue;
            };
            let mut s = shared.lock().expect("state");
            let (front, back) = s.ring.as_slices();
            let mut stream = stream;
            if stream.write_all(front).and_then(|()| stream.write_all(back)).is_err() {
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
                let mut s = shared.lock().expect("state");
                if s.client.as_ref().is_some_and(|(t, _)| *t == turn) {
                    s.client = None;
                }
            });
        }
    });

    let (shared, sizes) = (state.clone(), pty.clone());
    std::thread::spawn(move || {
        for stream in ctl.incoming().flatten() {
            let attached = u8::from(shared.lock().expect("state").client.is_some());
            let mut stream = stream;
            let hello = format!("hello {pid} {} {attached}\n", tty_path.display());
            if stream.write_all(hello.as_bytes()).is_err() {
                continue;
            }
            if let Ok(copy) = stream.try_clone() {
                shared.lock().expect("state").controls.push(copy);
            }
            let sizes = sizes.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stream).lines() {
                    let Ok(line) = line else {
                        break;
                    };
                    let words: Vec<u16> = line.split_whitespace().skip(1).filter_map(|w| w.parse().ok()).collect();
                    if line.starts_with("resize ")
                        && let [cols, lines, cw, ch] = words[..]
                    {
                        sizes.lock().expect("pty").on_resize(WindowSize {
                            num_lines: lines,
                            num_cols: cols,
                            cell_width: cw,
                            cell_height: ch,
                        });
                    } else if line == "kill" {
                        unsafe { libc::kill(pid as i32, libc::SIGHUP) };
                    }
                }
            });
        }
    });

    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match master.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let mut s = state.lock().expect("state");
        s.ring.extend(&buf[..n]);
        let extra = s.ring.len().saturating_sub(KEEP);
        s.ring.drain(..extra);
        if let Some((_, client)) = s.client.as_mut()
            && client.write_all(&buf[..n]).is_err()
        {
            s.client = None;
        }
    }
    let mut status = 0;
    unsafe { libc::waitpid(pid as i32, &mut status, 0) };
    let code = if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else {
        1
    };
    let _ = fs::remove_file(&ctl_path);
    let _ = fs::remove_file(&data_path);
    let mut s = state.lock().expect("state");
    for control in &mut s.controls {
        let _ = control.write_all(format!("exit {code}\n").as_bytes());
    }
    Ok(code)
}

fn slave_name(master: &File) -> Option<PathBuf> {
    let name = unsafe { libc::ptsname(master.as_raw_fd()) };
    if name.is_null() {
        return None;
    }
    let name = unsafe { std::ffi::CStr::from_ptr(name) };
    Some(PathBuf::from(name.to_string_lossy().into_owned()))
}
