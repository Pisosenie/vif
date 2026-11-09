//! `vif remote` and `vif ssh` from outside, with a stand-in for ssh that runs the command here.

use std::io::{Read, Write};
use std::net::UdpSocket;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The start of every stand-in for ssh: any host settles on this machine, and `$last` is the
/// command ssh would run over there.
const SSH: &str = "#!/bin/sh\nif [ \"$1\" = -G ]; then echo 'hostname 127.0.0.1'; exit 0; fi\nfor last; do :; done\n";

struct Place {
    root: PathBuf,
}

impl Place {
    fn new(name: &str) -> Self {
        // Short, because a socket's path has to fit in about a hundred bytes.
        let root = PathBuf::from(format!("/tmp/vif-r{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("vif.toml"), "[terminal]\nshell = \"/bin/sh\"\n").unwrap();
        let ssh = root.join("ssh");
        // Settles any host on this machine, and runs the last argument here, the way the other
        // machine would run the command it's given.
        std::fs::write(&ssh, format!("{SSH}exec /bin/sh -c \"$last\"\n")).unwrap();
        Command::new("chmod").args(["+x"]).arg(&ssh).status().unwrap();
        Self { root }
    }

    fn vif(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_vif"));
        command
            .args(args)
            .env("XDG_RUNTIME_DIR", &self.root)
            .env("XDG_DATA_HOME", &self.root)
            .env("VIF_CONFIG", self.root.join("vif.toml"))
            .env("VIF_SSH", self.root.join("ssh"))
            .env("VIF_REMOTE", env!("CARGO_BIN_EXE_vif"))
            .env("SHELL", "/bin/sh")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    fn alive(&self, id: &str) -> bool {
        self.root.join(format!("vif-sessions/{id}.ctl")).exists()
    }
}

impl Drop for Place {
    fn drop(&mut self) {
        for entry in std::fs::read_dir(self.root.join("vif-sessions"))
            .into_iter()
            .flatten()
            .flatten()
        {
            if entry.path().extension().is_some_and(|e| e == "ctl")
                && let Ok(mut ctl) = std::os::unix::net::UnixStream::connect(entry.path())
            {
                let _ = ctl.write_all(b"kill\n");
            }
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Reads until `text` has come, and returns all that came; gives up, ending the child, after ten
/// seconds without it.
fn until(child: &mut Child, text: &str) -> String {
    use std::os::fd::AsRawFd;
    let fd = child.stdout.as_ref().unwrap().as_raw_fd();
    // SAFETY: sets a flag on a pipe this test owns.
    unsafe { libc::fcntl(fd, libc::F_SETFL, libc::fcntl(fd, libc::F_GETFL) | libc::O_NONBLOCK) };
    let mut got = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut buf = [0u8; 4096];
    while !String::from_utf8_lossy(&got).contains(text) {
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("never saw {text:?} in {:?}", String::from_utf8_lossy(&got));
        }
        match child.stdout.as_mut().unwrap().read(&mut buf) {
            Ok(0) => panic!("ended before {text:?} in {:?}", String::from_utf8_lossy(&got)),
            Ok(n) => got.extend_from_slice(&buf[..n]),
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(err) => panic!("{err}"),
        }
    }
    String::from_utf8_lossy(&got).into_owned()
}

/// Reads until nothing more has come for a moment: whatever was coming, a prompt say, is here.
fn quiet(child: &mut Child) {
    use std::os::fd::AsRawFd;
    let fd = child.stdout.as_ref().unwrap().as_raw_fd();
    // SAFETY: sets a flag on a pipe this test owns.
    unsafe { libc::fcntl(fd, libc::F_SETFL, libc::fcntl(fd, libc::F_GETFL) | libc::O_NONBLOCK) };
    let mut buf = [0u8; 4096];
    let mut last = Instant::now();
    while last.elapsed() < Duration::from_millis(500) {
        match child.stdout.as_mut().unwrap().read(&mut buf) {
            Ok(n) if n > 0 => last = Instant::now(),
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

fn wait(child: &mut Child) -> Option<i32> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status.code();
        }
        assert!(Instant::now() < deadline, "it didn't end");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_remote_shell_outlives_its_connection() {
    let place = Place::new("drop");
    let mut first = place.vif(&["remote", "work"]).spawn().unwrap();
    first.stdin.as_mut().unwrap().write_all(b"echo r-$((3+4))\n").unwrap();
    until(&mut first, "r-7");

    // The connection drops: the shell goes on.
    drop(first.stdin.take());
    wait(&mut first);
    assert!(place.alive("work"));

    // Back again: the screen as it was, and the same shell.
    let mut again = place.vif(&["remote", "work"]).spawn().unwrap();
    let replay = until(&mut again, "r-7");
    assert!(replay.contains("echo r-$((3+4))"), "{replay}");
    again.stdin.as_mut().unwrap().write_all(b"exit\n").unwrap();
    assert_eq!(wait(&mut again), Some(0));
    assert!(!place.alive("work"));
}

#[test]
fn vif_ssh_runs_through_and_gives_up_on_a_host_it_cant_reach() {
    let place = Place::new("ssh");
    let mut ssh = place.vif(&["ssh", "--no-udp", "somewhere", "box1"]).spawn().unwrap();
    ssh.stdin.as_mut().unwrap().write_all(b"echo via-$((5*5))\n").unwrap();
    until(&mut ssh, "via-25");
    ssh.stdin.as_mut().unwrap().write_all(b"exit\n").unwrap();
    assert_eq!(wait(&mut ssh), Some(0));

    std::fs::write(place.root.join("ssh"), "#!/bin/sh\nexit 255\n").unwrap();
    let started = Instant::now();
    let mut unreachable = place.vif(&["ssh", "nowhere"]).spawn().unwrap();
    assert_eq!(wait(&mut unreachable), Some(255));
    assert!(started.elapsed() < Duration::from_secs(5));
    let mut said = String::new();
    unreachable.stderr.as_mut().unwrap().read_to_string(&mut said).unwrap();
    assert!(said.contains("can't reach nowhere"), "{said}");
}

#[test]
fn vif_ssh_over_udp_keeps_the_session_and_comes_back_to_it() {
    let place = Place::new("udp");
    let mut first = place.vif(&["ssh", "--udp", "somewhere", "box2"]).spawn().unwrap();
    first.stdin.as_mut().unwrap().write_all(b"echo udp-$((6*7))\n").unwrap();
    until(&mut first, "udp-42");

    // ctrl-^ . leaves the shell running over there.
    first.stdin.as_mut().unwrap().write_all(b"\x1e.").unwrap();
    assert_eq!(wait(&mut first), Some(0));
    let mut said = String::new();
    first.stderr.as_mut().unwrap().read_to_string(&mut said).unwrap();
    assert!(said.contains("left box2 running"), "{said}");
    assert!(place.alive("box2"));

    // Back again: a new agent paints the screen as it was.
    let mut again = place.vif(&["ssh", "--udp", "somewhere", "box2"]).spawn().unwrap();
    until(&mut again, "udp-42");
    again.stdin.as_mut().unwrap().write_all(b"exit\n").unwrap();
    assert_eq!(wait(&mut again), Some(0));
    assert!(!place.alive("box2"));
}

#[test]
fn an_old_vif_over_there_means_ssh_after_all() {
    let place = Place::new("old");
    // A vif without the agent says how it's used and nothing else, as 0.1.5 does.
    let old = place.root.join("oldvif");
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = udp-agent ]; then echo usage >&2; exit 2; fi\nexec {} \"$@\"\n",
        env!("CARGO_BIN_EXE_vif")
    );
    std::fs::write(&old, script).unwrap();
    Command::new("chmod").args(["+x"]).arg(&old).status().unwrap();
    let mut ssh = place
        .vif(&["ssh", "somewhere", "box3"])
        .env("VIF_REMOTE", &old)
        .spawn()
        .unwrap();
    ssh.stdin.as_mut().unwrap().write_all(b"echo tcp-$((2*4))\n").unwrap();
    until(&mut ssh, "tcp-8");
    ssh.stdin.as_mut().unwrap().write_all(b"exit\n").unwrap();
    assert_eq!(wait(&mut ssh), Some(0));
    let mut said = String::new();
    ssh.stderr.as_mut().unwrap().read_to_string(&mut said).unwrap();
    assert!(said.contains("staying on ssh"), "{said}");
}

/// Passes datagrams between the client and the agent, either way, losing about one in five
/// when `lossy` and holding each back `delay`.
fn proxy(root: &std::path::Path, lossy: bool, delay: Duration) -> u16 {
    let proxy = UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = proxy.local_addr().unwrap().port();
    let agent_port = root.join("agent-port");
    std::thread::spawn(move || {
        let mut client = None;
        let mut buf = [0u8; 2048];
        let mut dice = 0x2545_f491_u32;
        while let Ok((n, from)) = proxy.recv_from(&mut buf) {
            dice ^= dice << 13;
            dice ^= dice >> 17;
            dice ^= dice << 5;
            if lossy && dice.is_multiple_of(5) {
                continue;
            }
            let Some(agent) = std::fs::read_to_string(&agent_port)
                .ok()
                .and_then(|p| p.trim().parse::<u16>().ok())
            else {
                continue;
            };
            let to = if from.port() == agent {
                match client {
                    Some(c) => c,
                    None => continue,
                }
            } else {
                client = Some(from);
                std::net::SocketAddr::from(([127, 0, 0, 1], agent))
            };
            let datagram = buf[..n].to_vec();
            match proxy.try_clone() {
                Ok(out) if !delay.is_zero() => {
                    std::thread::spawn(move || {
                        std::thread::sleep(delay);
                        let _ = out.send_to(&datagram, to);
                    });
                }
                _ => {
                    let _ = proxy.send_to(&datagram, to);
                }
            }
        }
    });
    port
}

/// Starts the agent here and hands the client the proxy's port in place of the agent's.
fn ssh_through(place: &Place, proxy: u16) {
    let script = format!(
        "{SSH}/bin/sh -c \"$last\" > {root}/said || exit $?\n\
         read -r word one port key < {root}/said\necho \"$port\" > {root}/agent-port\n\
         echo \"$word $one {proxy} $key\"\n",
        root = place.root.display()
    );
    std::fs::write(place.root.join("ssh"), script).unwrap();
}

#[test]
fn on_a_slow_link_typing_shows_before_its_echo_once_guesses_come_true() {
    let place = Place::new("slow");
    ssh_through(&place, proxy(&place.root, false, Duration::from_millis(75)));
    let mut ssh = place.vif(&["ssh", "--udp", "somewhere", "box5"]).spawn().unwrap();
    let mut keys = ssh.stdin.take().unwrap();
    keys.write_all(b"echo ready-$((2+3))\n").unwrap();
    // The prompt comes after the output, and a guess goes where the cursor is.
    until(&mut ssh, "ready-5");
    quiet(&mut ssh);
    // The first guess in a line waits for its echo to be trusted; the next one shows at once.
    keys.write_all(b"e").unwrap();
    until(&mut ssh, "e");
    keys.write_all(b"c").unwrap();
    let shown = until(&mut ssh, "\x1b[0;4mc");
    assert!(!shown.contains("\x1b[0;4me"), "{shown:?}");
    keys.write_all(b"ho guessed\n").unwrap();
    until(&mut ssh, "guessed\r\n");
    keys.write_all(b"exit\n").unwrap();
    assert_eq!(wait(&mut ssh), Some(0));
}

#[test]
fn output_gets_through_a_link_that_loses_datagrams_whole_and_in_order() {
    let place = Place::new("loss");
    ssh_through(&place, proxy(&place.root, true, Duration::ZERO));
    let mut ssh = place.vif(&["ssh", "--udp", "somewhere", "box4"]).spawn().unwrap();
    ssh.stdin.as_mut().unwrap().write_all(b"echo ready-$((1+1))\n").unwrap();
    until(&mut ssh, "ready-2");
    ssh.stdin
        .as_mut()
        .unwrap()
        .write_all(b"seq 1 3000; echo done-$((1+1))\n")
        .unwrap();
    let got = until(&mut ssh, "done-2");
    let numbers: Vec<u32> = got
        .split("\r\n")
        .filter_map(|l| l.trim().parse().ok())
        .skip_while(|n| *n != 1)
        .collect();
    let first_wrong = numbers.iter().zip(1..).position(|(n, i)| *n != i);
    assert_eq!((numbers.len(), first_wrong), (3000, None));
    ssh.stdin.as_mut().unwrap().write_all(b"exit\n").unwrap();
    assert_eq!(wait(&mut ssh), Some(0));
}
