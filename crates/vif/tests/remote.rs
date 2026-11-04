//! `vif remote` and `vif ssh` from outside, with a stand-in for ssh that runs the command here.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

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
        // Takes `-t HOST COMMAND` and runs COMMAND here, the way the other machine would.
        std::fs::write(&ssh, "#!/bin/sh\nexec /bin/sh -c \"$3\"\n").unwrap();
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

/// Reads until `text` has come, and returns all that came.
fn until(child: &mut Child, text: &str) -> String {
    let out = child.stdout.as_mut().unwrap();
    let mut got = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut buf = [0u8; 4096];
    while !String::from_utf8_lossy(&got).contains(text) {
        assert!(Instant::now() < deadline, "never saw {text:?}");
        match out.read(&mut buf) {
            Ok(0) => panic!("ended before {text:?} in {:?}", String::from_utf8_lossy(&got)),
            Ok(n) => got.extend_from_slice(&buf[..n]),
            Err(err) => panic!("{err}"),
        }
    }
    String::from_utf8_lossy(&got).into_owned()
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
    let mut ssh = place.vif(&["ssh", "somewhere", "box1"]).spawn().unwrap();
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
