use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::{env, fs};

const NAME: &str = "xterm-vif";

const SOURCE: &str = include_str!("../../../assets/xterm-vif.terminfo");

/// The TERM value and TERMINFO directory to give child processes.
pub fn setup(term: &str) -> (String, Option<PathBuf>) {
    if term != NAME {
        return (term.to_owned(), None);
    }
    if installed() {
        return (NAME.to_owned(), None);
    }
    let dir = data_dir().join("terminfo");
    if compiled(&dir) || compile(&dir) {
        return (NAME.to_owned(), Some(dir));
    }
    log::warn!("couldn't compile the {NAME} terminfo, falling back to xterm-256color");
    ("xterm-256color".to_owned(), None)
}

/// A package already put the entry where every program looks.
fn installed() -> bool {
    ["/usr/share/terminfo", "/usr/lib/terminfo", "/etc/terminfo"]
        .iter()
        .any(|dir| {
            ["x", "78"]
                .iter()
                .any(|sub| std::path::Path::new(dir).join(sub).join(NAME).exists())
        })
}

pub fn data_dir() -> PathBuf {
    env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(env::temp_dir)
        .join("vif")
}

fn compiled(dir: &std::path::Path) -> bool {
    let stamp = dir.join(".source");
    let fresh = fs::read_to_string(&stamp).is_ok_and(|s| s == SOURCE);
    fresh && (dir.join("x").join(NAME).exists() || dir.join("78").join(NAME).exists())
}

fn compile(dir: &std::path::Path) -> bool {
    if fs::create_dir_all(dir).is_err() {
        return false;
    }
    let src = dir.join("xterm-vif.terminfo");
    if fs::write(&src, SOURCE).is_err() {
        return false;
    }
    let ok = Command::new("tic")
        .arg("-x")
        .arg("-o")
        .arg(dir)
        .arg(&src)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    ok && fs::write(dir.join(".source"), SOURCE).is_ok()
}
