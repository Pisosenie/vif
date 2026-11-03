//! The quick terminal: one window that drops in from the top on a key and goes away again, its
//! shell always running in a session. macOS takes the key itself; elsewhere the compositor runs
//! `vif quick`, which asks the running vif through a socket.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

use winit::event_loop::EventLoopProxy;

use crate::app::UserEvent;

fn socket() -> PathBuf {
    crate::private::runtime().join("vif.sock")
}

/// Asks a running vif to show or hide its quick terminal; false when none answers.
pub fn ask() -> bool {
    UnixStream::connect(socket())
        .and_then(|mut stream| stream.write_all(b"quick\n"))
        .is_ok()
}

/// Answers `vif quick` from other processes; the first vif to start keeps the socket. Returns
/// whether this one did.
pub fn listen(proxy: EventLoopProxy<UserEvent>) -> bool {
    let path = socket();
    if UnixStream::connect(&path).is_ok() {
        return false;
    }
    if let Err(err) = crate::private::runtime_made() {
        log::warn!("no quick terminal socket: {err}");
        return false;
    }
    let _ = fs::remove_file(&path);
    let Ok(listener) = UnixListener::bind(&path) else {
        return false;
    };
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            if BufReader::new(stream).read_line(&mut line).is_ok() && line.trim() == "quick" {
                let _ = proxy.send_event(UserEvent::Quick);
            }
        }
    });
    true
}

pub fn forget() {
    let _ = fs::remove_file(socket());
}

/// The key that toggles the quick terminal from any app, while it is registered.
#[cfg(target_os = "macos")]
pub struct Hotkey {
    manager: global_hotkey::GlobalHotKeyManager,
    key: Option<global_hotkey::hotkey::HotKey>,
}

#[cfg(target_os = "macos")]
impl Hotkey {
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Result<Self, String> {
        use global_hotkey::{GlobalHotKeyEvent, HotKeyState};
        let manager = global_hotkey::GlobalHotKeyManager::new().map_err(|e| e.to_string())?;
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state == HotKeyState::Pressed {
                let _ = proxy.send_event(UserEvent::Quick);
            }
        }));
        Ok(Self { manager, key: None })
    }

    /// Takes `combo` as the key, letting go of the one before; an empty one is no key.
    pub fn set(&mut self, combo: &str) -> Result<(), String> {
        if let Some(old) = self.key.take() {
            let _ = self.manager.unregister(old);
        }
        if combo.is_empty() {
            return Ok(());
        }
        let key: global_hotkey::hotkey::HotKey = combo.parse().map_err(|e| format!("quick key {combo}: {e}"))?;
        self.manager
            .register(key)
            .map_err(|e| format!("quick key {combo}: {e}"))?;
        self.key = Some(key);
        Ok(())
    }
}

/// Shows the window over full-screen apps and on every desktop, as a drop-down should be.
#[cfg(target_os = "macos")]
pub fn everywhere(window: &winit::window::Window) {
    use objc2_app_kit::{NSView, NSWindowCollectionBehavior};
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };
    // SAFETY: winit's AppKit handle points at the window's NSView, alive while the window is.
    let view: &NSView = unsafe { handle.ns_view.cast().as_ref() };
    if let Some(ns_window) = view.window() {
        ns_window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
    }
}

/// Whether vif is the app in front.
#[cfg(target_os = "macos")]
pub fn active() -> bool {
    objc2::MainThreadMarker::new().is_some_and(|mtm| objc2_app_kit::NSApplication::sharedApplication(mtm).isActive())
}

/// Hands the focus back to the app that had it before vif.
#[cfg(target_os = "macos")]
pub fn step_back() {
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        objc2_app_kit::NSApplication::sharedApplication(mtm).hide(None);
    }
}
