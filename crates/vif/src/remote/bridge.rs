//! The other machine's end of `vif ssh` over ssh: the session joined to ssh's terminal.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::ExitCode;

use alacritty_terminal::event::WindowSize;
use vif_config::Config;

use super::{TAKEN, clean};
use crate::session::{self, Spec};

/// `vif remote NAME`, on the other machine: the session NAME, started if it isn't running, joined
/// to this terminal until the connection or the shell ends.
pub fn remote(args: &[String]) -> ExitCode {
    let Some(id) = args.first().and_then(|n| clean(n)) else {
        eprintln!("usage: vif remote NAME");
        return ExitCode::from(2);
    };
    if let Err(err) = ensure(&id) {
        eprintln!("vif: can't start the session: {err}");
        return ExitCode::FAILURE;
    }
    match bridge(&id) {
        Ok(code) => ExitCode::from(code as u8),
        Err(err) => {
            eprintln!("vif: {err}");
            ExitCode::FAILURE
        }
    }
}

/// The session `id`, started if it isn't running.
pub(super) fn ensure(id: &str) -> io::Result<()> {
    if session::alive(id) {
        return Ok(());
    }
    session::start_as(&spec(), id).and_then(|()| session::rename(id, id))
}

/// The shell for a new remote session, set up as a window here would set it up.
fn spec() -> Spec {
    let (config, _) = Config::load();
    let mut env = crate::shell::env(&crate::terminfo::setup(&config.terminal.term));
    let program = crate::shell::prepare(&config, &mut env);
    let [cols, lines, x, y] = size();
    Spec {
        program: program.as_ref().map(|(p, _)| p.clone()),
        args: program.map(|(_, a)| a).unwrap_or_default(),
        cwd: std::env::var_os("HOME").map(PathBuf::from),
        env,
        size: (cols, lines, x, y),
    }
}

/// This process's terminal: columns, lines, and pixels across and down.
pub(super) fn size() -> [u16; 4] {
    crate::tty::size(libc::STDIN_FILENO).map_or([80, 24, 0, 0], |s| [s.ws_col, s.ws_row, s.ws_xpixel, s.ws_ypixel])
}

/// A terminal's size as a session host takes it.
pub(super) fn window_size([cols, lines, x, y]: [u16; 4]) -> WindowSize {
    let cell = |px: u16, n: u16| px.checked_div(n).unwrap_or(0);
    WindowSize {
        num_cols: cols,
        num_lines: lines,
        cell_width: cell(x, cols),
        cell_height: cell(y, lines),
    }
}

fn tell_size(ctl: &UnixStream) {
    session::resize(ctl, window_size(size()));
}

/// Passes keys to the session and its output back until either side ends; 0 when the shell
/// ended, [`TAKEN`] when another connection took it over.
fn bridge(id: &str) -> io::Result<i32> {
    let (mut data, ctl) = session::connect(id)?;
    // Raw, so every key goes to the shell over there.
    // SAFETY: cfmakeraw only changes the flags of the termios it's given.
    let raw = crate::tty::Modes::change(|t| unsafe { libc::cfmakeraw(t) });
    tell_size(&ctl);
    let resizes = ctl.try_clone()?;
    crate::tty::on_signal(libc::SIGWINCH, move || tell_size(&resizes));
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
    Ok(if session::ended(&ctl) { 0 } else { TAKEN })
}
