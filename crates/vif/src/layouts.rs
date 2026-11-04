//! Where each window's sessions sat, so the next vif puts them back in the same windows, tabs and splits.
//! A copy with each shell's folder is kept in the data folder too: after a restart, when the
//! shells are gone, the windows come back with new shells where the old ones were.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, DirBuilder};
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::session;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Shape {
    Pane(String),
    Split {
        across: bool,
        ratio: f32,
        first: Box<Shape>,
        second: Box<Shape>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedTab {
    pub root: Shape,
    pub focus: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub tabs: Vec<SavedTab>,
    pub active: usize,
    /// The quick terminal's, which comes back as the quick terminal.
    #[serde(default)]
    pub quick: bool,
    /// The folder each session's shell was in.
    #[serde(default)]
    pub places: BTreeMap<String, PathBuf>,
    /// Its shells are gone: the window comes back with new ones in their folders.
    #[serde(skip)]
    pub restart: bool,
}

impl Shape {
    fn sessions<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            Shape::Pane(id) => out.push(id),
            Shape::Split { first, second, .. } => {
                first.sessions(out);
                second.sessions(out);
            }
        }
    }

    /// The same shape with only the sessions `keep` allows; a split that loses a side becomes the other.
    fn keep(self, keep: &mut impl FnMut(&str) -> bool) -> Option<Shape> {
        match self {
            Shape::Pane(id) => keep(&id).then_some(Shape::Pane(id)),
            Shape::Split {
                across,
                ratio,
                first,
                second,
            } => match (first.keep(keep), second.keep(keep)) {
                (Some(first), Some(second)) => Some(Shape::Split {
                    across,
                    ratio,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (one, other) => one.or(other),
            },
        }
    }
}

fn path(window: u64) -> PathBuf {
    session::dir().join(name(window))
}

fn name(window: u64) -> String {
    format!("{}-{window}.layout", std::process::id())
}

/// Where layouts and the shells' last output outlive a restart.
pub fn kept() -> PathBuf {
    crate::terminfo::data_dir().join("layouts")
}

fn write(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    let fresh = path.with_extension("new");
    fs::write(&fresh, text)?;
    fs::rename(&fresh, path)
}

/// Every window that wrote down its layout, oldest first: whether it's the quick terminal, and
/// its sessions.
pub fn windows() -> Vec<(bool, Vec<String>)> {
    let mut found: Vec<(SystemTime, bool, Vec<String>)> = fs::read_dir(session::dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "layout"))
        .filter_map(|e| {
            let at = e
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            let saved: Saved = serde_json::from_str(&fs::read_to_string(e.path()).ok()?).ok()?;
            let mut ids = Vec::new();
            for tab in &saved.tabs {
                tab.root.sessions(&mut ids);
            }
            Some((at, saved.quick, ids.into_iter().map(str::to_owned).collect()))
        })
        .collect();
    found.sort_by_key(|(at, ..)| *at);
    found.into_iter().map(|(_, quick, ids)| (quick, ids)).collect()
}

/// Writes a window's layout; one with no sessions leaves no file.
pub fn save(window: u64, saved: &Saved) {
    let path = path(window);
    if saved.tabs.is_empty() {
        let _ = fs::remove_file(&path);
        return;
    }
    let Ok(text) = serde_json::to_string(saved) else {
        return;
    };
    if crate::private::made("vif-sessions").is_ok() {
        let _ = write(&path, &text);
    }
    if DirBuilder::new().recursive(true).mode(0o700).create(kept()).is_ok() {
        let _ = write(&kept().join(name(window)), &text);
    }
}

pub fn forget(window: u64) {
    let _ = fs::remove_file(path(window));
    let _ = fs::remove_file(kept().join(name(window)));
}

/// The windows of a vif that's gone and whose shells went with it, as after a restart; each
/// session's last output is kept by its id next to them.
pub fn after_restart() -> Vec<Saved> {
    let mut found: Vec<(SystemTime, Saved)> = Vec::new();
    for entry in fs::read_dir(kept()).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "layout") || alive(pid_of(&path)) {
            continue;
        }
        let at = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let saved = fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<Saved>(&t).ok());
        let _ = fs::remove_file(&path);
        let Some(saved) = saved else {
            continue;
        };
        let mut ids = Vec::new();
        for tab in &saved.tabs {
            tab.root.sessions(&mut ids);
        }
        // A shell still running is another vif's to take, not this one's to start again.
        if ids.iter().all(|id| !session::dir().join(format!("{id}.ctl")).exists()) {
            found.push((at, Saved { restart: true, ..saved }));
        }
    }
    found.sort_by_key(|(at, _)| *at);
    found.into_iter().map(|(_, saved)| saved).collect()
}

/// What a session printed last before its host was stopped, taken once.
pub fn last_output(session: &str) -> Option<Vec<u8>> {
    let path = kept().join(format!("{session}.ring"));
    let bytes = fs::read(&path).ok();
    let _ = fs::remove_file(&path);
    bytes
}

fn pid_of(path: &std::path::Path) -> i32 {
    path.file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| s.split('-').next())
        .and_then(|p| p.parse().ok())
        .unwrap_or(0)
}

fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 sends nothing; it only asks whether the process is there.
    pid > 0 && unsafe { libc::kill(pid, 0) } == 0
}

/// The windows to bring back, oldest first, holding only sessions in `free`; free sessions no
/// layout knows become tabs of the first. Layouts of a vif that still runs stay where they are.
pub fn take(free: Vec<String>) -> Vec<Saved> {
    let mut found: Vec<(SystemTime, PathBuf, i32)> = Vec::new();
    for entry in fs::read_dir(session::dir()).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "layout") {
            continue;
        }
        let pid = pid_of(&path);
        let at = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        found.push((at, path, pid));
    }
    found.sort();
    let mut left: Vec<String> = free;
    let mut windows = Vec::new();
    for (_, path, pid) in found {
        if alive(pid) {
            continue;
        }
        let saved = fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<Saved>(&text).ok());
        let _ = fs::remove_file(&path);
        if let Some(saved) = saved.and_then(|s| arrange(s, &mut left)) {
            // Its shells are back, so its copy for a restart is spent; this vif writes its own.
            if let Some(name) = path.file_name() {
                let _ = fs::remove_file(kept().join(name));
            }
            windows.push(saved);
        }
    }
    if !left.is_empty() {
        if !windows.iter().any(|w| !w.quick) {
            windows.push(Saved::default());
        }
        let first = windows.iter_mut().find(|w| !w.quick).expect("an ordinary window");
        first.tabs.extend(left.into_iter().map(|id| SavedTab {
            root: Shape::Pane(id.clone()),
            focus: id,
        }));
    }
    windows
}

/// One window's tabs cut down to the sessions in `free`, which loses the ones it takes.
fn arrange(saved: Saved, free: &mut Vec<String>) -> Option<Saved> {
    let mut used = HashSet::new();
    let mut tabs = Vec::new();
    let mut active = 0;
    for (i, tab) in saved.tabs.into_iter().enumerate() {
        let Some(root) = tab
            .root
            .keep(&mut |id| free.iter().any(|f| f == id) && used.insert(id.to_owned()))
        else {
            continue;
        };
        if i <= saved.active {
            active = tabs.len();
        }
        let mut ids = Vec::new();
        root.sessions(&mut ids);
        let focus = if ids.contains(&tab.focus.as_str()) {
            tab.focus
        } else {
            ids[0].to_owned()
        };
        tabs.push(SavedTab { root, focus });
    }
    free.retain(|id| !used.contains(id));
    (!tabs.is_empty()).then_some(Saved { tabs, active, ..saved })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: &str) -> Shape {
        Shape::Pane(id.to_owned())
    }

    fn split(first: Shape, second: Shape) -> Shape {
        Shape::Split {
            across: true,
            ratio: 0.3,
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    #[test]
    fn gone_sessions_fold_their_splits() {
        let saved = Saved {
            tabs: vec![
                SavedTab {
                    root: split(pane("a"), split(pane("b"), pane("c"))),
                    focus: "b".into(),
                },
                SavedTab {
                    root: pane("d"),
                    focus: "d".into(),
                },
                SavedTab {
                    root: pane("e"),
                    focus: "e".into(),
                },
            ],
            active: 2,
            ..Saved::default()
        };
        let mut free = vec!["a".to_owned(), "c".to_owned(), "e".to_owned(), "z".to_owned()];
        let got = arrange(saved, &mut free).unwrap();
        assert_eq!(got.tabs.len(), 2);
        assert_eq!(got.tabs[0].root, split(pane("a"), pane("c")));
        assert_eq!(got.tabs[0].focus, "a");
        assert_eq!(got.active, 1);
        assert_eq!(free, vec!["z".to_owned()]);
    }
}
