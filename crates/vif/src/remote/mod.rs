//! Shells on another machine that outlive the connection. `vif ssh HOST` runs `vif remote NAME`
//! there over ssh; that keeps the shell in a session host on HOST and joins it to ssh's terminal.
//! When the connection drops, the shell goes on, ssh connects again, and the host paints the
//! screen again: tmux and mosh's reconnecting in one, with vif's own shell integration over there.

mod agent;
mod bridge;
mod client;
mod flow;
mod netwatch;
mod predict;
mod ssh;
mod wire;

use std::net::{IpAddr, ToSocketAddrs};
use std::process::{Command, ExitCode, Stdio};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use vif_config::{Config, Udp};

pub use agent::main as udp_agent;
pub use bridge::remote;

pub const USAGE: &str = "       vif ssh [--udp|--no-udp] HOST [NAME]
                                   a shell on HOST that outlives the connection; vif must be on HOST
                                   too. NAME comes back to the same shell later. Over UDP, like mosh,
                                   where it gets through";

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

/// Where a session lives and how ssh gets there.
#[derive(Clone)]
struct Link {
    ssh: String,
    host: String,
    name: String,
    /// vif as the other machine finds it.
    vif: String,
    /// ssh's options for the host as its config settles them, from `ssh -G`.
    settled: String,
    /// Where ssh connects to, looked up once.
    address: OnceLock<Option<IpAddr>>,
}

impl Link {
    fn new(host: &str, name: String) -> Self {
        let ssh = std::env::var("VIF_SSH").unwrap_or_else(|_| "ssh".to_owned());
        let settled = Command::new(&ssh)
            .args(["-G", host])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
            .unwrap_or_default();
        Self {
            ssh,
            host: host.to_owned(),
            name,
            vif: std::env::var("VIF_REMOTE").unwrap_or_else(|_| "vif".to_owned()),
            settled,
            address: OnceLock::new(),
        }
    }

    /// An option as ssh settled it, lowercase name.
    fn option(&self, name: &str) -> Option<&str> {
        self.settled.lines().find_map(|line| {
            let (key, value) = line.split_once(' ')?;
            key.eq_ignore_ascii_case(name).then(|| value.trim())
        })
    }

    /// Keepalives, unless the user's config has its own, so a dead link shows within ten seconds
    /// rather than whenever TCP gives up.
    fn keepalive(&self) -> Vec<&'static str> {
        match self.option("serveraliveinterval") {
            None | Some("0") => vec!["-o", "ServerAliveInterval=5", "-o", "ServerAliveCountMax=2"],
            Some(_) => Vec::new(),
        }
    }

    /// `vif ARGS` over there, in a login shell so vif is found where the profile puts it.
    fn there(&self, args: &str) -> String {
        format!("exec \"${{SHELL:-/bin/sh}}\" -lc 'exec {} {args}'", self.vif)
    }

    /// Whether ssh goes through another host, so this machine never talks to HOST itself.
    fn proxied(&self) -> bool {
        ["proxyjump", "proxycommand"]
            .iter()
            .any(|o| self.option(o).is_some_and(|v| v != "none"))
    }

    /// The host's address when ssh connects to it directly, an IPv4 one where there is one.
    fn address(&self) -> Option<IpAddr> {
        *self.address.get_or_init(|| {
            if self.proxied() {
                return None;
            }
            let name = self.option("hostname").unwrap_or(&self.host);
            let found: Vec<IpAddr> = (name, 22).to_socket_addrs().ok()?.map(|a| a.ip()).collect();
            // An IPv6 address with no route to it is the usual way UDP finds nothing, so IPv4
            // goes first where there is one.
            found.iter().find(|ip| ip.is_ipv4()).or(found.first()).copied()
        })
    }
}

/// The exit code for how ssh or the session ended, said in words where the user should know.
fn explain(link: &Link, code: i32) -> ExitCode {
    let (host, name) = (&link.host, &link.name);
    match code {
        127 => eprintln!(
            "vif: {host} has no `{}` on its PATH; vif has to be installed there too",
            link.vif
        ),
        TAKEN => eprintln!("vif: {name} on {host} is open somewhere else now"),
        255 => eprintln!("vif: can't reach {host}"),
        _ => {}
    }
    ExitCode::from(code.clamp(0, 255) as u8)
}

/// `vif ssh [--udp|--no-udp] HOST [NAME]`, here.
pub fn ssh(args: &[String]) -> ExitCode {
    let (flags, args): (Vec<&String>, Vec<&String>) = args.iter().partition(|a| a.starts_with("--"));
    let udp = match flags.last().map(|f| f.as_str()) {
        None => Config::load().0.ssh.udp,
        Some("--udp") => Udp::On,
        Some("--no-udp") => Udp::Off,
        Some(_) => {
            eprintln!("usage:\n{USAGE}");
            return ExitCode::from(2);
        }
    };
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
    let link = Link::new(host, name);
    if udp != Udp::Off {
        match client::run(&link, udp == Udp::On) {
            client::Outcome::Done(code) => return code,
            client::Outcome::Instead(why) if udp == Udp::On => {
                let why = why.unwrap_or_else(|| "it didn't get through".to_owned());
                eprintln!("vif: UDP to {host} didn't work: {why}");
                return ExitCode::FAILURE;
            }
            client::Outcome::Instead(Some(why)) => eprintln!("vif: {why}; staying on ssh"),
            client::Outcome::Instead(None) => {}
        }
    }
    ssh::run(&link)
}

/// What a datagram can make either end parse, and output its screens take in and paint again.
pub fn fuzz(data: &[u8]) {
    let _ = wire::Up::decode(data);
    if let Some(down) = wire::Down::decode(data) {
        let _ = flow::Inbox::default().take(down.epoch, down.at, &down.data);
    }
    let _ = wire::Opener::new(&[0; wire::KEY_LEN], wire::Side::Agent).open(data);
    let mut there = crate::screen::Screen::new(20, 6, 10);
    there.feed(data);
    crate::screen::Screen::new(20, 6, 10).feed(&there.paint(10));
}

/// Hands on what `next` reads, on a thread of its own, until it reads nothing more; then `end`.
fn pump<T: Send + 'static>(
    tell: std::sync::mpsc::Sender<T>,
    end: Option<T>,
    mut next: impl FnMut(&mut [u8]) -> Option<T> + Send + 'static,
) {
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 64 * 1024];
        while let Some(item) = next(&mut buf) {
            if tell.send(item).is_err() {
                return;
            }
        }
        if let Some(end) = end {
            let _ = tell.send(end);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(settled: &str) -> Link {
        Link {
            ssh: "ssh".into(),
            host: "box".into(),
            name: "work".into(),
            vif: "vif".into(),
            settled: settled.into(),
            address: OnceLock::new(),
        }
    }

    #[test]
    fn names_are_kept_safe() {
        assert_eq!(clean("build-2_x").as_deref(), Some("build-2_x"));
        assert_eq!(clean("a;rm -rf /").as_deref(), Some("arm-rf"));
        assert_eq!(clean("'$(x)'"), Some("x".to_owned()));
        assert_eq!(clean("..//"), None);
    }

    #[test]
    fn keepalives_only_where_the_config_has_none() {
        assert_eq!(link("").keepalive().len(), 4);
        assert_eq!(link("user me\nserveraliveinterval 0\n").keepalive().len(), 4);
        assert!(link("serveraliveinterval 30\n").keepalive().is_empty());
    }

    #[test]
    fn a_jump_host_means_no_direct_address() {
        assert!(!link("hostname 10.0.0.1\nproxyjump none\n").proxied());
        assert!(link("proxyjump bastion\n").proxied());
        assert!(link("proxycommand nc %h %p\n").proxied());
        assert_eq!(link("hostname 127.0.0.1\n").address(), Some([127, 0, 0, 1].into()));
        assert_eq!(link("hostname 127.0.0.1\nproxyjump b\n").address(), None);
    }
}
