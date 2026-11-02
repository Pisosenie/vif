//! Desktop notifications, through what the system already has.

use std::process::{Command, Stdio};

pub fn send(title: &str, body: &str) {
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
        c.args(["--app-name=vif", "--icon=vif", title, body]);
        c
    };
    let spawned = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match spawned {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(err) => log::warn!("can't notify: {err}"),
    }
}
