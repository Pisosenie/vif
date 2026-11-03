//! Every command that ran, with what it printed, kept on disk for a while, so "which command
//! printed that error yesterday" has an answer. Plain JSON lines, one file a day, nothing leaves
//! the machine; only this user can read them, and secrets are taken out before they're written.

use std::fs::{self, DirBuilder, OpenOptions, Permissions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const OUTPUT_LIMIT: usize = 256 * 1024;
const DAY: u64 = 86_400;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// When it ended, in seconds since 1970.
    pub at: u64,
    pub took: f32,
    pub exit: i32,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    pub command: String,
    pub output: String,
}

pub fn dir() -> PathBuf {
    crate::terminfo::data_dir().join("archive")
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Keeps the end of long output: that is where errors are.
pub fn trimmed(output: &str) -> String {
    if output.len() <= OUTPUT_LIMIT {
        return output.to_owned();
    }
    let mut cut = output.len() - OUTPUT_LIMIT;
    while !output.is_char_boundary(cut) {
        cut += 1;
    }
    format!("…\n{}", &output[cut..])
}

pub fn record(entry: &Entry) {
    let path = dir().join(format!("{}.jsonl", date(entry.at / DAY)));
    let written = DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir())
        .and_then(|()| {
            // Folders and files from before vif closed them get closed too.
            fs::set_permissions(dir(), Permissions::from_mode(0o700))?;
            let mut file = OpenOptions::new().create(true).append(true).mode(0o600).open(&path)?;
            file.set_permissions(Permissions::from_mode(0o600))?;
            let hidden = Entry {
                command: crate::secrets::hide(&entry.command),
                output: crate::secrets::hide(&entry.output),
                ..entry.clone()
            };
            let line = serde_json::to_string(&hidden).map_err(std::io::Error::other)?;
            writeln!(file, "{line}")
        });
    if let Err(err) = written {
        log::warn!("can't keep the command in {}: {err}", path.display());
    }
}

/// Drops days older than `days`.
pub fn prune(days: u32) {
    let oldest = date((now() / DAY).saturating_sub(u64::from(days)));
    for entry in fs::read_dir(dir()).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".jsonl") && name.as_str() < format!("{oldest}.jsonl").as_str() {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// The kept lines, newest first, at most `max` of them.
pub fn lines(max: usize) -> Vec<String> {
    let mut days: Vec<PathBuf> = fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    days.sort();
    let mut out = Vec::new();
    for day in days.iter().rev() {
        let Ok(text) = fs::read_to_string(day) else {
            continue;
        };
        out.extend(text.lines().rev().map(str::to_owned));
        if out.len() >= max {
            out.truncate(max);
            break;
        }
    }
    out
}

/// The newest entries holding every word of `query` in the command or its output.
pub fn find(lines: &[String], query: &str, limit: usize) -> Vec<Entry> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut found = Vec::new();
    for line in lines {
        let low = line.to_lowercase();
        if !words.iter().all(|w| low.contains(w.as_str())) {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Entry>(line) else {
            continue;
        };
        let haystack = format!("{}\n{}", entry.command, entry.output).to_lowercase();
        if words.iter().all(|w| haystack.contains(w.as_str())) {
            found.push(entry);
            if found.len() >= limit {
                break;
            }
        }
    }
    found
}

/// The first line of output with a word of the query in it, or its last line.
pub fn snippet(entry: &Entry, query: &str) -> String {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let lines: Vec<&str> = entry.output.lines().filter(|l| !l.trim().is_empty()).collect();
    let hit = lines
        .iter()
        .find(|l| words.iter().any(|w| l.to_lowercase().contains(w.as_str())));
    hit.or(lines.last()).map(|l| l.trim().to_owned()).unwrap_or_default()
}

/// How long ago, the way people say it.
pub fn ago(at: u64) -> String {
    let s = now().saturating_sub(at);
    match s {
        0..60 => "just now".to_owned(),
        60..3600 => format!("{}m ago", s / 60),
        3600..DAY => format!("{}h ago", s / 3600),
        _ if s < 2 * DAY => "yesterday".to_owned(),
        _ => format!("{}d ago", s / DAY),
    }
}

/// The calendar day `days` after 1970-01-01, as YYYY-MM-DD.
fn date(days: u64) -> String {
    // Howard Hinnant's civil_from_days.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_are_calendar_dates() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(19_723), "2024-01-01");
        assert_eq!(date(20_759), "2026-11-02");
    }

    #[test]
    fn long_output_keeps_its_end() {
        let long = format!("{}the error", "x".repeat(OUTPUT_LIMIT + 10));
        let kept = trimmed(&long);
        assert!(kept.starts_with('…') && kept.ends_with("the error"));
        assert!(kept.len() <= OUTPUT_LIMIT + 4);
    }

    #[test]
    fn snippets_show_the_matching_line() {
        let entry = Entry {
            at: 0,
            took: 1.0,
            exit: 101,
            cwd: "~/src/vif".to_owned(),
            host: None,
            command: "cargo test".to_owned(),
            output: "running 3 tests\ntest a ... ok\nthread 'b' panicked at src/x.rs:4\n\ntest result: FAILED\n"
                .to_owned(),
        };
        assert_eq!(snippet(&entry, "panicked"), "thread 'b' panicked at src/x.rs:4");
        assert_eq!(snippet(&entry, "cargo"), "test result: FAILED");
    }
}
