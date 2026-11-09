//! The session over ssh's own connection: ssh runs `vif remote NAME` with a terminal, and runs
//! again when the connection drops. A network change that moves this machine to another
//! address ends the old connection at once instead of waiting for keepalives to notice.

use std::net::{IpAddr, UdpSocket};
use std::process::{Command, ExitCode};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::netwatch::Changes;
use super::{Link, explain};

/// An attempt that lasted this long got through, so its end was a dropped connection.
const THROUGH: Duration = Duration::from_secs(5);

pub(super) fn run(link: &Link) -> ExitCode {
    let cut = Cut::watch(link);
    let (host, name) = (&link.host, &link.name);
    let mut through = false;
    let mut wait = Duration::from_secs(1);
    loop {
        let started = Instant::now();
        let status = Command::new(&link.ssh)
            .args(link.keepalive())
            .args(["-t", host.as_str(), link.there(&format!("remote {name}")).as_str()])
            .spawn()
            .and_then(|mut child| {
                cut.track(Some(child.id()));
                let status = child.wait();
                cut.track(None);
                status
            });
        let code = match status {
            Ok(status) => status.code(),
            Err(err) => {
                eprintln!("vif: can't run {}: {err}", link.ssh);
                return ExitCode::FAILURE;
            }
        };
        if started.elapsed() >= THROUGH {
            through = true;
            wait = Duration::from_secs(1);
        }
        if cut.happened() {
            eprintln!("\r\nvif: the network changed; connecting again to {name}\r");
            continue;
        }
        match next(code, through) {
            Next::Again => {
                eprintln!("\r\nvif: the connection to {host} dropped; connecting again to {name} (ctrl-c stops)\r");
                std::thread::sleep(wait);
                wait = (wait * 2).min(Duration::from_secs(10));
            }
            Next::Stop(code) => return explain(link, code),
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

/// The address this machine would leave from to reach `to` now.
pub(super) fn source(to: IpAddr) -> Option<IpAddr> {
    let any: IpAddr = if to.is_ipv4() {
        [0, 0, 0, 0].into()
    } else {
        [0u16; 8].into()
    };
    let socket = UdpSocket::bind((any, 0)).ok()?;
    // Connecting a UDP socket sends nothing; it only asks the system for a route.
    socket.connect((to, 22)).ok()?;
    socket.local_addr().ok().map(|a| a.ip())
}

/// Ends the running ssh when the network moved this machine off the address it connected from.
#[derive(Default)]
struct Cut {
    /// Where ssh connects to, once looked up off the way of the first connection.
    to: OnceLock<Option<IpAddr>>,
    /// The running ssh and the address it left from.
    running: Mutex<Option<(u32, Option<IpAddr>)>>,
    happened: AtomicBool,
}

impl Cut {
    fn watch(link: &Link) -> Arc<Self> {
        let cut = Arc::new(Self::default());
        let (watching, link) = (Arc::clone(&cut), link.clone());
        std::thread::spawn(move || {
            let to = link.address();
            let _ = watching.to.set(to);
            // The first ssh may have started before the address was known.
            if let Ok(mut running) = watching.running.lock()
                && let Some((_, from @ None)) = running.as_mut()
            {
                *from = to.and_then(source);
            }
            if let (Some(_), Ok(changes)) = (to, Changes::watch()) {
                while changes.next().is_ok() {
                    watching.changed();
                }
            }
        });
        cut
    }

    fn track(&self, pid: Option<u32>) {
        let from = pid.and_then(|_| self.to.get().copied().flatten().and_then(source));
        if let Ok(mut running) = self.running.lock() {
            *running = pid.map(|pid| (pid, from));
        }
    }

    fn changed(&self) {
        let Some(to) = self.to.get().copied().flatten() else {
            return;
        };
        let Ok(running) = self.running.lock() else {
            return;
        };
        let Some((pid, Some(from))) = *running else {
            return;
        };
        if source(to) == Some(from) {
            return;
        }
        self.happened.store(true, Ordering::SeqCst);
        // SAFETY: signals the ssh we started and still wait for, so the pid is ours.
        unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    }

    fn happened(&self) -> bool {
        self.happened.swap(false, Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::super::TAKEN;
    use super::*;

    #[test]
    fn only_a_dropped_connection_is_tried_again() {
        assert_eq!(next(Some(255), true), Next::Again);
        assert_eq!(next(Some(255), false), Next::Stop(255));
        assert_eq!(next(Some(0), true), Next::Stop(0));
        assert_eq!(next(Some(127), true), Next::Stop(127));
        assert_eq!(next(Some(TAKEN), true), Next::Stop(TAKEN));
        assert_eq!(next(None, true), Next::Stop(130));
    }

    #[test]
    fn the_route_to_this_machine_leaves_from_loopback() {
        assert_eq!(source([127, 0, 0, 1].into()), Some([127, 0, 0, 1].into()));
    }
}
