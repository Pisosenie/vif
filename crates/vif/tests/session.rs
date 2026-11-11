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
            .env("XDG_DATA_HOME", &runtime)
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
        let mut ctl = UnixStream::connect(self.socket("ctl")).unwrap();
        ctl.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let hello = said(&mut ctl);
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

/// The next line a control connection says.
fn said(ctl: &mut UnixStream) -> String {
    let mut line = String::new();
    BufReader::new(ctl).read_line(&mut line).unwrap();
    line
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
    assert_eq!(said(&mut ctl).trim(), "exit 3");
}

#[test]
fn peeks_answer_and_leave_nothing_open() {
    let host = Host::start("peek");
    let mut data = host.attach();
    data.write_all(b"printf 'tail-of-it'\n").unwrap();
    until(&mut data, "tail-of-it$");
    let (mut ctl, _) = host.control();
    // The screen whole, however little was asked for, so no sequence is cut in half.
    assert!(String::from_utf8_lossy(&peek(&mut ctl, 10)).contains("tail-of-it"));

    // More peeks than a host may have files open: each one closed must let go of its socket.
    for _ in 0..300 {
        let (mut ctl, _) = host.control();
        assert!(!peek(&mut ctl, 16).is_empty());
    }
    data.write_all(b"echo still-here\n").unwrap();
    until(&mut data, "still-here");
}

#[test]
fn coming_back_paints_the_screen_as_it_is_rather_than_the_bytes_again() {
    let host = Host::start("paint");
    let (mut ctl, _) = host.control();
    let mut data = host.attach();
    // A full-screen program, as far as the terminal can tell, that's still running.
    data.write_all(b"printf '\\033[?1049h\\033[2;3Hfull-%s' screen; sleep 30\n")
        .unwrap();
    until(&mut data, "full-screen");
    drop(data);
    // The window comes back narrower than the program drew for.
    ctl.write_all(b"resize 40 12 8 16\n").unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let mut again = host.attach();
    let painted = until(&mut again, "\x1b]6907;painted\x07");
    // The painting is all there before the mark that ends it.
    let painted = painted.split("\x1b]6907;painted").next().unwrap().to_owned();
    // The shell's screen first, with the command that started the program, then the program's.
    let program = painted.rfind("\x1b[?1049h").expect("the alternate screen");
    assert!(painted.find("printf").is_some_and(|at| at < program), "{painted:?}");
    assert!(painted.contains("\x1b[2;1H\x1b[0m  full-screen"), "{painted:?}");
}

#[test]
fn stopping_the_host_keeps_what_it_printed() {
    let mut host = Host::start("stop");
    let mut data = host.attach();
    data.write_all(b"echo API_TOKEN=abc123secret done-$((2+2))\n").unwrap();
    until(&mut data, "done-4");
    // SAFETY: a signal to the host this test started.
    unsafe { libc_kill(host.child.id() as i32, 15) };
    let status = host.child.wait().unwrap();
    assert!(status.code().is_some() || status.success(), "{status:?}");
    let ring = host.dir.parent().unwrap().join("vif/layouts/t1.ring");
    let kept = std::fs::read_to_string(&ring).unwrap();
    assert!(kept.contains("done-4"), "{kept}");
    assert!(
        kept.contains("API_TOKEN=••••") && !kept.contains("abc123secret"),
        "{kept}"
    );
}

#[test]
fn what_scrolled_off_comes_back_above_the_screen() {
    let host = Host::start("history");
    let mut data = host.attach();
    data.write_all(b"i=0; while [ $i -lt 60 ]; do echo row-$i; i=$((i+1)); done; echo last-$((6*7))\n")
        .unwrap();
    until(&mut data, "last-42");
    drop(data);
    let mut again = host.attach();
    let painted = until(&mut again, "\x1b]6907;painted\x07");
    // row-0 left a 24-line screen long ago; it comes back first, the rest in order after it.
    let first = painted.find("row-0\x1b").expect("the first line");
    let later = painted.find("row-59").expect("the last line");
    assert!(first < later, "{painted:?}");
}

#[test]
fn a_new_size_reaches_the_shell() {
    let host = Host::start("size");
    let (mut ctl, _) = host.control();
    let mut data = host.attach();
    ctl.write_all(b"resize 100 30 8 16\n").unwrap();
    // The shell waits for the size to come rather than the test guessing how long it takes.
    data.write_all(b"until [ \"$(stty size)\" = '30 100' ]; do sleep 0.05; done; echo size-$(stty size | tr ' ' x)\n")
        .unwrap();
    until(&mut data, "size-30x100");
}

#[test]
fn kill_ends_the_shell_and_every_control_hears_it() {
    let host = Host::start("kill");
    let mut data = host.attach();
    data.write_all(b"echo up-$((1+1))\n").unwrap();
    until(&mut data, "up-2");
    let (mut watcher, _) = host.control();
    let (mut killer, _) = host.control();
    // Nonsense on a control connection is ignored, and the connection goes on working.
    killer.write_all(b"frobnicate 7\n").unwrap();
    killer.write_all(b"kill\n").unwrap();
    let line = said(&mut watcher);
    assert!(line.starts_with("exit "), "{line:?}");
    assert!(closed(&mut data));
    wait(|| !host.socket("ctl").exists());
}

#[test]
fn a_taken_session_leaves_the_old_control_quiet() {
    let host = Host::start("taken");
    let (mut old_ctl, _) = host.control();
    let mut first = host.attach();
    first.write_all(b"echo one-$((2+3))\n").unwrap();
    until(&mut first, "one-5");
    let mut second = host.attach();
    until(&mut second, "one-5");
    assert!(closed(&mut first));
    // Nothing on the control connection: the shell goes on, someone else has it.
    old_ctl.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    let quiet = old_ctl
        .read(&mut [0u8; 64])
        .is_err_and(|e| matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut));
    assert!(quiet, "the old control heard something");
    second.write_all(b"exit 4\n").unwrap();
    old_ctl.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    assert_eq!(said(&mut old_ctl).trim(), "exit 4");
}

unsafe extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, signal: i32) -> i32;
}
