//! Desktop notifications, through what the system already has.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// Something to do when the notification's Show button is pressed.
pub type Show = Box<dyn FnOnce() + Send>;

/// Whether this notify-send can put buttons on a notification and say which was pressed.
fn buttons() -> bool {
    static CAN: OnceLock<bool> = OnceLock::new();
    *CAN.get_or_init(|| {
        Command::new("notify-send")
            .arg("--help")
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("--action"))
    })
}

/// Shows a notification. Where the notifications can have buttons (on Linux, with a daemon like
/// mako or swaync), it has Show block and Dismiss, and `show` runs when Show block is pressed.
pub fn send(title: &str, body: &str, show: Option<Show>) {
    let (title, body) = (title.to_owned(), body.to_owned());
    // Off the event loop: asking notify-send what it can do takes a moment, once.
    std::thread::spawn(move || notify(&title, &body, show));
}

fn notify(title: &str, body: &str, show: Option<Show>) {
    let show = show.filter(|_| cfg!(target_os = "linux") && buttons());
    let mut command = if cfg!(target_os = "macos") {
        let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
        let mut c = Command::new("osascript");
        c.arg("-e").arg(format!(
            "display notification {} with title {}",
            quote(body),
            quote(title)
        ));
        c
    } else {
        let mut c = Command::new("notify-send");
        c.args(["--app-name=vif", "--icon=vif"]);
        if show.is_some() {
            c.args(["--action=show=Show block", "--action=dismiss=Dismiss", "--wait"]);
        }
        c.args([title, body]);
        c
    };
    let spawned = command
        .stdin(Stdio::null())
        .stdout(if show.is_some() { Stdio::piped() } else { Stdio::null() })
        .stderr(Stdio::null())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(err) => return log::warn!("can't notify: {err}"),
    };
    // notify-send waits for an answer and prints the button's name.
    let mut answer = String::new();
    if let Some(out) = child.stdout.as_mut() {
        let _ = out.read_to_string(&mut answer);
    }
    let _ = child.wait();
    if let Some(show) = show.filter(|_| answer.trim() == "show") {
        show();
    }
}
