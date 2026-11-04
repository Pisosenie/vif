//! Shells on another machine that outlive the connection. `vif ssh HOST` runs `vif remote NAME`
//! there over ssh; that keeps the shell in a session host on HOST and joins it to ssh's terminal.
//! When the connection drops, the shell goes on, ssh connects again, and the host plays the
//! screen back: tmux and mosh's reconnecting in one, with vif's own shell integration over there.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use vif_config::Config;

use crate::session::{self, Spec};

pub const USAGE: &str =
    "       vif ssh HOST [NAME]         a shell on HOST that outlives the connection; vif must be on HOST
                                   too. NAME comes back to the same shell later";

/// An attempt that lasted this long got through, so its end was a dropped connection.
const THROUGH: Duration = Duration::from_secs(5);
/// What `vif remote` says when another `vif ssh` took the session over.
const TAKEN: i32 = 3;

/// A session name safe in a socket path and on a command line.
fn clean(name: &str) -> Option<String> {
    let name: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(40)
        .collect();
    (!name.is_empty()).then_some(name)
}

/// `vif ssh HOST [NAME]`, here.
pub fn ssh(args: &[String]) -> ExitCode {
    let Some(host) = args.first().filter(|h| !h.starts_with('-')) else {
        eprintln!("usage:\n{USAGE}");
        return ExitCode::from(2);
    };
    let name = match args.get(1) {
        Some(name) => match clean(name) {
            Some(name) => name,
            None => {
                eprintln!("vif: a session name is letters, digits, - and _");
                return ExitCode::from(2);
            }
        },
        None => {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.subsec_nanos());
            format!("ssh-{:05x}", nanos & 0xfffff)
        }
    };
    let ssh = std::env::var("VIF_SSH").unwrap_or_else(|_| "ssh".to_owned());
    let vif = std::env::var("VIF_REMOTE").unwrap_or_else(|_| "vif".to_owned());
    // A login shell over there, so vif is found where the user's profile puts it on PATH.
    let there = format!("exec \"${{SHELL:-/bin/sh}}\" -lc 'exec {vif} remote {name}'");
    let mut through = false;
    let mut wait = Duration::from_secs(1);
    loop {
        let started = Instant::now();
        let status = Command::new(&ssh).args(["-t", host.as_str(), there.as_str()]).status();
        let code = match status {
            Ok(status) => status.code(),
            Err(err) => {
                eprintln!("vif: can't run {ssh}: {err}");
                return ExitCode::FAILURE;
            }
        };
        through |= started.elapsed() >= THROUGH;
        match next(code, through) {
            Next::Again => {
                eprintln!("\r\nvif: the connection to {host} dropped; connecting again to {name} (ctrl-c stops)\r");
                std::thread::sleep(wait);
                wait = (wait * 2).min(Duration::from_secs(10));
            }
            Next::Stop(code) => {
                match code {
                    127 => eprintln!("vif: {host} has no `{vif}` on its PATH; vif has to be installed there too"),
                    TAKEN => eprintln!("vif: {name} on {host} is open somewhere else now"),
                    255 => eprintln!("vif: can't reach {host}"),
                    _ => {}
                }
                return ExitCode::from(code.clamp(0, 255) as u8);
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Next {
    Again,
    Stop(i32),
}

/// What to do after ssh ends with `code`: 255 is ssh's own failure, which is worth another try
/// once a connection has got through before; anything else is the remote side's answer.
fn next(code: Option<i32>, through: bool) -> Next {
    match code {
        Some(255) if through => Next::Again,
        Some(code) => Next::Stop(code),
        None => Next::Stop(130),
    }
}

/// `vif remote NAME`, on the other machine: the session NAME, started if it isn't running, joined
/// to this terminal until the connection or the shell ends.
pub fn remote(args: &[String]) -> ExitCode {
    let Some(id) = args.first().and_then(|n| clean(n)) else {
        eprintln!("usage: vif remote NAME");
        return ExitCode::from(2);
    };
    if !session::alive(&id) {
        let started = session::start_as(&spec(), &id).and_then(|()| session::rename(&id, &id));
        if let Err(err) = started {
            eprintln!("vif: can't start the session: {err}");
            return ExitCode::FAILURE;
        }
    }
    match bridge(&id) {
        Ok(code) => ExitCode::from(code as u8),
        Err(err) => {
            eprintln!("vif: {err}");
            ExitCode::FAILURE
        }
    }
}

/// The shell for a new remote session, set up as a window here would set it up.
fn spec() -> Spec {
    let (config, _) = Config::load();
    let (term, terminfo) = crate::terminfo::setup(&config.terminal.term);
    let mut env = std::collections::HashMap::from([
        ("TERM".to_owned(), term),
        ("COLORTERM".to_owned(), "truecolor".to_owned()),
        ("TERM_PROGRAM".to_owned(), "vif".to_owned()),
        ("TERM_PROGRAM_VERSION".to_owned(), env!("CARGO_PKG_VERSION").to_owned()),
    ]);
    if let Some(dir) = terminfo {
        env.insert("TERMINFO".to_owned(), dir.display().to_string());
    }
    let program = crate::shell::prepare(&config, &mut env);
    let size = size();
    Spec {
        program: program.as_ref().map(|(p, _)| p.clone()),
        args: program.map(|(_, a)| a).unwrap_or_default(),
        cwd: std::env::var_os("HOME").map(PathBuf::from),
        env,
        size: (size.ws_col, size.ws_row, size.ws_xpixel, size.ws_ypixel),
    }
}

fn size() -> libc::winsize {
    // SAFETY: a C struct of plain numbers, for which all zeroes is a valid value.
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    // SAFETY: TIOCGWINSZ fills the winsize we pass.
    if unsafe { libc::ioctl(libc::STDIN_FILENO, libc::TIOCGWINSZ, &mut size) } != 0 || size.ws_col == 0 {
        size.ws_col = 80;
        size.ws_row = 24;
    }
    size
}

fn tell_size(mut ctl: &UnixStream) {
    let s = size();
    let cell = |px: u16, n: u16| px.checked_div(n).unwrap_or(0);
    let line = format!(
        "resize {} {} {} {}\n",
        s.ws_col,
        s.ws_row,
        cell(s.ws_xpixel, s.ws_col),
        cell(s.ws_ypixel, s.ws_row)
    );
    let _ = ctl.write_all(line.as_bytes());
}

/// This terminal in raw mode while it lasts, so every key goes to the shell over there.
struct Raw(Option<libc::termios>);

impl Raw {
    fn enter() -> Self {
        // SAFETY: a C struct of plain numbers, for which all zeroes is a valid value.
        let mut saved: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: tcgetattr fills the termios we pass; failure, as with no terminal, is checked.
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut saved) } != 0 {
            return Self(None);
        }
        let mut raw = saved;
        // SAFETY: cfmakeraw changes the flags of the termios we pass, and tcsetattr applies it.
        unsafe {
            libc::cfmakeraw(&mut raw);
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw);
        }
        Self(Some(saved))
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        if let Some(saved) = &self.0 {
            // SAFETY: the termios as tcgetattr gave it.
            unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, saved) };
        }
    }
}

/// Passes keys to the session and its output back until either side ends; 0 when the shell
/// ended, [`TAKEN`] when another connection took it over.
fn bridge(id: &str) -> io::Result<i32> {
    let (mut data, ctl) = session::connect(id)?;
    let raw = Raw::enter();
    tell_size(&ctl);
    // Size changes reach one thread of their own, which every thread after this leaves them to.
    // SAFETY: the set is ours and starts empty; the mask is this thread's, inherited by new ones.
    let winch = unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGWINCH);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        set
    };
    let resizes = ctl.try_clone()?;
    std::thread::spawn(move || {
        let mut signal = 0;
        // SAFETY: waits on the set made above, with SIGWINCH blocked so only this call takes it.
        while unsafe { libc::sigwait(&winch, &mut signal) } == 0 {
            tell_size(&resizes);
        }
    });
    let mut keys = data.try_clone()?;
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut stdin = io::stdin().lock();
        while let Ok(n) = stdin.read(&mut buf) {
            if n == 0 || keys.write_all(&buf[..n]).is_err() {
                break;
            }
        }
        // The connection is gone: let go of the session, which goes on without us.
        let _ = keys.shutdown(std::net::Shutdown::Both);
    });
    let mut out = io::stdout().lock();
    let mut buf = [0u8; 64 * 1024];
    while let Ok(n) = data.read(&mut buf) {
        if n == 0 || out.write_all(&buf[..n]).and_then(|()| out.flush()).is_err() {
            break;
        }
    }
    drop(raw);
    Ok(if session::alive(id) { TAKEN } else { 0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_kept_safe() {
        assert_eq!(clean("build-2_x").as_deref(), Some("build-2_x"));
        assert_eq!(clean("a;rm -rf /").as_deref(), Some("arm-rf"));
        assert_eq!(clean("'$(x)'"), Some("x".to_owned()));
        assert_eq!(clean("..//"), None);
    }

    #[test]
    fn only_a_dropped_connection_is_tried_again() {
        assert_eq!(next(Some(255), true), Next::Again);
        assert_eq!(next(Some(255), false), Next::Stop(255));
        assert_eq!(next(Some(0), true), Next::Stop(0));
        assert_eq!(next(Some(127), true), Next::Stop(127));
        assert_eq!(next(Some(TAKEN), true), Next::Stop(TAKEN));
        assert_eq!(next(None, true), Next::Stop(130));
    }
}
