//! The session host from outside, the way windows talk to it: a real `vif session-host`
//! running `sh`, spoken to over its data and control sockets.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Host {
    child: Child,
    dir: PathBuf,
    id: String,
}

impl Host {
    fn start(name: &str) -> Self {
        // Short, because a socket's path has to fit in about a hundred bytes.
        let runtime = PathBuf::from(format!("/tmp/vif-t{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&runtime);
        std::fs::create_dir_all(&runtime).unwrap();
        let spec = r#"{"program":"/bin/sh","args":[],"cwd":"/","env":{"PS1":"$ "},"size":[80,24,8,16]}"#;
        let id = "t1".to_owned();
        // Few files allowed, so a host that keeps sockets it's done with runs out fast.
        let child = Command::new("/bin/sh")
            .args(["-c", r#"ulimit -n 64 && exec "$0" session-host "$1" "$2""#])
            .args([env!("CARGO_BIN_EXE_vif"), &id, spec])
            .env("XDG_RUNTIME_DIR", &runtime)
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        let dir = runtime.join("vif-sessions");
        let host = Host { child, dir, id };
        wait(|| host.socket("ctl").exists() && host.socket("data").exists());
        host
    }

    fn socket(&self, kind: &str) -> PathBuf {
        self.dir.join(format!("{}.{kind}", self.id))
    }

    fn control(&self) -> (UnixStream, String) {
        let ctl = UnixStream::connect(self.socket("ctl")).unwrap();
        ctl.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut hello = String::new();
        BufReader::new(ctl.try_clone().unwrap()).read_line(&mut hello).unwrap();
        (ctl, hello)
    }

    fn attach(&self) -> UnixStream {
        let data = UnixStream::connect(self.socket("data")).unwrap();
        data.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
        data
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(runtime) = self.dir.parent() {
            let _ = std::fs::remove_dir_all(runtime);
        }
    }
}

fn wait(done: impl Fn() -> bool) {
    let until = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < until, "gave up waiting");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Reads until `text` has come, and returns all that came.
fn until(stream: &mut UnixStream, text: &str) -> String {
    let mut got = Vec::new();
    let until = Instant::now() + Duration::from_secs(10);
    while !String::from_utf8_lossy(&got).contains(text) {
        assert!(
            Instant::now() < until,
            "never saw {text:?} in {:?}",
            String::from_utf8_lossy(&got)
        );
        let mut buf = [0u8; 4096];
        if let Ok(n) = stream.read(&mut buf) {
            got.extend_from_slice(&buf[..n]);
        }
    }
    String::from_utf8_lossy(&got).into_owned()
}

/// Whether the other end let go: reads come back empty, or fail, within a few seconds.
fn closed(stream: &mut UnixStream) -> bool {
    let until = Instant::now() + Duration::from_secs(5);
    let mut buf = [0u8; 4096];
    while Instant::now() < until {
        match stream.read(&mut buf) {
            Ok(0) => return true,
            Err(err) if err.kind() != std::io::ErrorKind::WouldBlock => return true,
            _ => {}
        }
    }
    false
}

fn peek(ctl: &mut UnixStream, bytes: usize) -> Vec<u8> {
    ctl.write_all(format!("peek {bytes}\n").as_bytes()).unwrap();
    let mut reader = BufReader::new(ctl);
    let mut head = String::new();
    reader.read_line(&mut head).unwrap();
    let len: usize = head.trim().strip_prefix("peek ").unwrap().parse().unwrap();
    let mut out = vec![0; len];
    reader.read_exact(&mut out).unwrap();
    out
}

#[test]
fn hello_attach_detach_and_exit() {
    let host = Host::start("life");
    let (_, hello) = host.control();
    let words: Vec<&str> = hello.split_whitespace().collect();
    assert_eq!(words[0], "hello", "{hello}");
    assert!(words[1].parse::<u32>().unwrap() > 0);
    assert!(Path::new(words[2]).starts_with("/dev"), "{hello}");
    assert_eq!(words[3], "0", "nobody attached yet: {hello}");

    let mut data = host.attach();
    data.write_all(b"echo one-$((40+2))\n").unwrap();
    until(&mut data, "one-42");
    let (_, hello) = host.control();
    assert_eq!(hello.split_whitespace().nth(3), Some("1"), "{hello}");

    // Gone and back: what it printed meanwhile comes again first.
    drop(data);
    wait(|| host.control().1.split_whitespace().nth(3) == Some("0"));
    let mut again = host.attach();
    let replay = until(&mut again, "one-42");
    assert!(replay.contains("echo one-$((40+2))"), "{replay}");

    // A second window takes it over; the first one's connection ends.
    let mut second = host.attach();
    until(&mut second, "one-42");
    assert!(closed(&mut again), "the first window still has it");

    let (mut ctl, _) = host.control();
    second.write_all(b"exit 3\n").unwrap();
    let mut said = String::new();
    BufReader::new(&mut ctl).read_line(&mut said).unwrap();
    assert_eq!(said.trim(), "exit 3");
}

#[test]
fn peeks_answer_and_leave_nothing_open() {
    let host = Host::start("peek");
    let mut data = host.attach();
    data.write_all(b"printf 'tail-of-it'\n").unwrap();
    until(&mut data, "tail-of-it$");
    let (mut ctl, _) = host.control();
    let tail = peek(&mut ctl, 10);
    assert_eq!(tail.len(), 10);
    assert!(String::from_utf8_lossy(&peek(&mut ctl, 4096)).contains("tail-of-it"));

    // More peeks than a host may have files open: each one closed must let go of its socket.
    for _ in 0..300 {
        let (mut ctl, _) = host.control();
        assert!(!peek(&mut ctl, 16).is_empty());
    }
    data.write_all(b"echo still-here\n").unwrap();
    until(&mut data, "still-here");
}
