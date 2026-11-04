//! This process's own terminal and signals, for the commands that run in one: its size, its modes
//! changed for a while, and a signal handled on a thread of its own.

/// The size of the terminal on `fd`, when it is one.
pub fn size(fd: libc::c_int) -> Option<libc::winsize> {
    // SAFETY: a C struct of plain numbers, for which all zeroes is a valid value.
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    // SAFETY: TIOCGWINSZ fills the winsize we pass.
    let ok = unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut size) } == 0;
    (ok && size.ws_col > 0).then_some(size)
}

/// The terminal on stdin with its modes changed, as they were again when this goes.
pub struct Modes(libc::termios);

impl Modes {
    /// Applies `change` to stdin's modes; None when stdin isn't a terminal.
    pub fn change(change: impl FnOnce(&mut libc::termios)) -> Option<Self> {
        // SAFETY: a C struct of plain numbers and arrays, for which all zeroes is a valid value.
        let mut saved: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: tcgetattr fills the termios we pass; failure is checked.
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut saved) } != 0 {
            return None;
        }
        let mut changed = saved;
        change(&mut changed);
        // SAFETY: a termios tcgetattr filled, with `change` applied to its flags.
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &changed) };
        Some(Self(saved))
    }
}

impl Drop for Modes {
    fn drop(&mut self) {
        // SAFETY: the termios as tcgetattr gave it.
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.0) };
    }
}

/// Runs `then` on a thread of its own each time `signal` comes. The signal is blocked on this
/// thread and on every thread it starts afterwards, so only that thread takes it: call this
/// before starting the others, and after starting any child that should keep the signal.
pub fn on_signal(signal: libc::c_int, mut then: impl FnMut() + Send + 'static) {
    // SAFETY: the set is ours and starts empty; the mask is this thread's, inherited by new ones.
    let set = unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, signal);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        set
    };
    std::thread::spawn(move || {
        let mut got = 0;
        // SAFETY: waits on the set made above, with the signal blocked so only this call takes it.
        while unsafe { libc::sigwait(&set, &mut got) } == 0 {
            then();
        }
    });
}
