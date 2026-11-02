use std::collections::HashSet;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

/// One screen row as text: the characters, the column each one starts at, and whether the row
/// runs on into the next one.
pub struct Row {
    pub text: Vec<char>,
    pub cols: Vec<usize>,
    pub wraps: bool,
}

/// An OSC 8 link on screen: its row, first and last column, and address.
pub struct Link {
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub uri: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Url,
    Path { path: PathBuf, line: Option<u32> },
    Hash,
    Ip,
}

/// Cells on screen: row, column, character.
pub type Cells = Vec<(usize, usize, char)>;

/// Something worth picking, with every place it shows on screen.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub kind: Kind,
    pub text: String,
    pub label: String,
    /// Each occurrence as the cells it covers.
    pub at: Vec<Cells>,
}

impl Target {
    /// What picking it does, in a few words: "open atlas.rs at 118 in nvim".
    pub fn describe(&self) -> String {
        match &self.kind {
            Kind::Url => {
                let rest = self.text.split_once("://").map_or(self.text.as_str(), |(_, r)| r);
                format!("open {} in the browser", short(rest, 40))
            }
            Kind::Path { path, .. } if path.is_dir() => format!("open a shell in {}", name(path)),
            Kind::Path { path, line: Some(line) } => format!("open {} at {line} in {}", name(path), editor()),
            Kind::Path { path, line: None } => format!("open {} in {}", name(path), editor()),
            Kind::Hash => format!("show commit {}", short(&self.text, 12)),
            Kind::Ip => format!("copy {}", self.text),
        }
    }
}

fn name(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

fn short(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    text.chars().take(max - 1).collect::<String>() + "…"
}

/// The editor files open in, by its name.
pub fn editor() -> String {
    let command = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_owned());
    let program = command.split_whitespace().next().unwrap_or("vi");
    program.rsplit('/').next().unwrap_or(program).to_owned()
}

pub struct Hints {
    pub targets: Vec<Target>,
    pub typed: String,
    cwd: Option<PathBuf>,
    alphabet: Vec<char>,
}

impl Hints {
    pub fn new(cwd: Option<PathBuf>, alphabet: &str) -> Self {
        Self {
            targets: Vec::new(),
            typed: String::new(),
            cwd,
            alphabet: alphabet.chars().collect(),
        }
    }

    /// Rescans the screen; labels stay put while the same things are on it.
    pub fn refresh(&mut self, rows: &[Row], links: &[Link]) {
        let mut fresh = scan(rows, links, self.cwd.as_deref());
        let same = fresh.len() == self.targets.len()
            && fresh.iter().all(|t| self.targets.iter().any(|old| old.text == t.text));
        if same {
            for t in &mut fresh {
                if let Some(old) = self.targets.iter().find(|old| old.text == t.text) {
                    t.label.clone_from(&old.label);
                }
            }
        } else {
            label(&mut fresh, &self.alphabet);
            self.typed.clear();
        }
        self.targets = fresh;
    }

    /// Types one more letter of a label; the target once a whole label is typed.
    pub fn press(&mut self, c: char) -> Option<Target> {
        self.typed.push(c);
        if let Some(t) = self.targets.iter().find(|t| t.label == self.typed) {
            return Some(t.clone());
        }
        if !self.targets.iter().any(|t| t.label.starts_with(&self.typed)) {
            self.typed.pop();
        }
        None
    }

    pub fn visible(&self) -> impl Iterator<Item = &Target> {
        self.targets
            .iter()
            .filter(|t| !t.label.is_empty() && t.label.starts_with(&self.typed))
    }
}

static URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:https?|ftp|file|ssh|git)://[^\s<>"'`]+|mailto:[^\s<>"'`]+"#).unwrap());
static PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:~|\.\.?)?/?[\w.@+~%-]+(?:/[\w.@+~%-]+)*/?(?::(\d+))?(?::\d+)?").unwrap());
static IP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b|\b[0-9A-Fa-f]{1,4}(?::[0-9A-Fa-f]{0,4}){2,7}\b").unwrap()
});
static HASH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b[0-9a-f]{7,40}\b").unwrap());

/// Rows joined where they wrap, with where each character sits on screen.
struct Logical {
    text: String,
    /// Byte offset, row, column and character of each character in `text`.
    cells: Vec<(usize, usize, usize, char)>,
}

impl Logical {
    fn cells(&self, start: usize, end: usize) -> Cells {
        let from = self.cells.partition_point(|c| c.0 < start);
        let to = self.cells.partition_point(|c| c.0 < end);
        self.cells[from..to].iter().map(|&(_, l, c, ch)| (l, c, ch)).collect()
    }
}

fn logical(rows: &[Row]) -> Vec<Logical> {
    let mut out = Vec::new();
    let mut current = Logical {
        text: String::new(),
        cells: Vec::new(),
    };
    for (line, row) in rows.iter().enumerate() {
        for (&ch, &col) in row.text.iter().zip(&row.cols) {
            current.cells.push((current.text.len(), line, col, ch));
            current.text.push(ch);
        }
        if !row.wraps {
            out.push(std::mem::replace(
                &mut current,
                Logical {
                    text: String::new(),
                    cells: Vec::new(),
                },
            ));
        }
    }
    if !current.cells.is_empty() {
        out.push(current);
    }
    out
}

pub fn scan(rows: &[Row], links: &[Link], cwd: Option<&Path>) -> Vec<Target> {
    let mut found: Vec<(Kind, String, Cells)> = Vec::new();
    let mut taken: HashSet<(usize, usize)> = HashSet::new();
    for link in links {
        let Some(row) = rows.get(link.line) else {
            continue;
        };
        let cells: Vec<_> = row
            .text
            .iter()
            .zip(&row.cols)
            .filter(|&(_, &col)| col >= link.start && col <= link.end)
            .map(|(&ch, &col)| (link.line, col, ch))
            .collect();
        if !cells.is_empty() {
            taken.extend(cells.iter().map(|c| (c.0, c.1)));
            found.push((Kind::Url, link.uri.clone(), cells));
        }
    }
    for logical in logical(rows) {
        let text = logical.text.as_str();
        let mut add = |kind: Kind, start: usize, end: usize, found: &mut Vec<_>| {
            let cells = logical.cells(start, end);
            if !cells.is_empty() && !cells.iter().any(|c| taken.contains(&(c.0, c.1))) {
                taken.extend(cells.iter().map(|c| (c.0, c.1)));
                found.push((kind, text[start..end].to_owned(), cells));
            }
        };
        for m in URL.find_iter(text) {
            let url = trim_url(m.as_str());
            add(Kind::Url, m.start(), m.start() + url.len(), &mut found);
        }
        for c in PATH.captures_iter(text) {
            let m = c.get(0).expect("whole match");
            let line = c.get(1).and_then(|l| l.as_str().parse().ok());
            let body = c.get(1).map_or(m.as_str(), |l| &text[m.start()..l.start() - 1]);
            let trimmed = body.trim_end_matches(['.', ',']);
            if let Some(path) = path(trimmed, line.is_some(), cwd) {
                let end = if line.is_some() {
                    m.end()
                } else {
                    m.start() + trimmed.len()
                };
                add(Kind::Path { path, line }, m.start(), end, &mut found);
            }
        }
        for m in IP.find_iter(text) {
            let s = m.as_str();
            let ip = s.parse::<Ipv4Addr>().is_ok()
                || (s.parse::<Ipv6Addr>().is_ok() && s.contains(|c: char| c.is_ascii_digit()));
            if ip {
                add(Kind::Ip, m.start(), m.end(), &mut found);
            }
        }
        for m in HASH.find_iter(text) {
            let s = m.as_str();
            if s.contains(|c: char| c.is_ascii_digit()) && s.contains(|c: char| c.is_ascii_alphabetic()) {
                add(Kind::Hash, m.start(), m.end(), &mut found);
            }
        }
    }

    let mut targets: Vec<Target> = Vec::new();
    for (kind, text, cells) in found {
        match targets.iter_mut().find(|t| t.text == text) {
            Some(t) => t.at.push(cells),
            None => targets.push(Target {
                kind,
                text,
                label: String::new(),
                at: vec![cells],
            }),
        }
    }
    targets
}

/// A path worth a hint: one with a line number, one that is plainly a path, or one that exists.
fn path(text: &str, has_line: bool, cwd: Option<&Path>) -> Option<PathBuf> {
    if text.len() < 2 || text.chars().all(|c| c == '/' || c == '.') {
        return None;
    }
    let explicit = ["/", "~/", "./", "../"].iter().any(|p| text.starts_with(p));
    let shaped = text.contains('/') || (has_line && text.contains('.'));
    if !shaped {
        return None;
    }
    let full = match text.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rest))?,
        None if text.starts_with('/') => PathBuf::from(text),
        None => cwd.map_or_else(|| PathBuf::from(text), |cwd| cwd.join(text)),
    };
    (explicit || has_line || full.exists()).then_some(full)
}

/// Drops punctuation a sentence put after the address.
pub fn trim_url(url: &str) -> &str {
    let mut url = url;
    loop {
        let Some(last) = url.chars().last() else {
            return url;
        };
        let count = |c: char| url.matches(c).count();
        let unbalanced = match last {
            ')' => count('(') < count(')'),
            ']' => count('[') < count(']'),
            '}' => count('{') < count('}'),
            _ => false,
        };
        if matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | '\'') || unbalanced {
            url = &url[..url.len() - last.len_utf8()];
        } else {
            return url;
        }
    }
}

/// Gives the lowest targets the shortest labels; no label starts another.
fn label(targets: &mut [Target], alphabet: &[char]) {
    let mut order: Vec<usize> = (0..targets.len()).collect();
    let bottom = |t: &Target| t.at.iter().flatten().map(|c| (c.0, c.1)).max().unwrap_or_default();
    order.sort_by_key(|&i| std::cmp::Reverse(bottom(&targets[i])));
    for (i, label) in order.into_iter().zip(labels(targets.len(), alphabet)) {
        targets[i].label = label;
    }
}

fn labels(n: usize, alphabet: &[char]) -> Vec<String> {
    let k = alphabet.len();
    let letter = |i: usize| alphabet[i];
    if n <= k {
        return (0..n).map(|i| letter(i).to_string()).collect();
    }
    let singles = ((k * k).saturating_sub(n) / (k - 1)).min(k);
    let mut out: Vec<String> = (0..singles).map(|i| letter(i).to_string()).collect();
    for a in singles..k {
        for b in 0..k {
            out.push(format!("{}{}", letter(a), letter(b)));
        }
    }
    out.truncate(n);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(lines: &[&str]) -> Vec<Row> {
        lines
            .iter()
            .map(|l| Row {
                text: l.chars().collect(),
                cols: (0..l.chars().count()).collect(),
                wraps: false,
            })
            .collect()
    }

    fn texts(lines: &[&str]) -> Vec<(String, Kind)> {
        let rows = screen(lines);
        scan(&rows, &[], Some(Path::new("/nowhere")))
            .into_iter()
            .map(|t| (t.text, t.kind))
            .collect()
    }

    #[test]
    fn finds_what_a_terminal_prints() {
        let found = texts(&[
            "a41c2e9 grid: pages with interned styles",
            "build  pass  https://github.com/Pisosenie/vif/actions/runs/11873.",
            "  --> crates/render/src/atlas.rs:118:17",
            "wlan0    UP       192.168.1.47/24",
            "tailscale0       fe80::1c2:3d4e",
            "at 12:34:56 this and/or that, 1234567 dead beef deadbeef",
        ]);
        let has = |text: &str, kind: Kind| found.contains(&(text.to_owned(), kind));
        assert!(has("https://github.com/Pisosenie/vif/actions/runs/11873", Kind::Url));
        assert!(has(
            "crates/render/src/atlas.rs:118:17",
            Kind::Path {
                path: PathBuf::from("/nowhere/crates/render/src/atlas.rs"),
                line: Some(118)
            }
        ));
        assert!(has("192.168.1.47", Kind::Ip));
        assert!(has("fe80::1c2:3d4e", Kind::Ip));
        assert!(has("a41c2e9", Kind::Hash));
        assert_eq!(found.len(), 5);
    }

    #[test]
    fn follows_wrapped_rows_and_merges_repeats() {
        let mut rows = screen(&[
            "see https://example.com/a/very/lo",
            "ng/path and",
            "https://example.com/x https://example.com/x",
        ]);
        rows[0].wraps = true;
        let found = scan(&rows, &[], None);
        assert_eq!(found[0].text, "https://example.com/a/very/long/path");
        assert_eq!(found[0].at[0].last().map(|c| (c.0, c.1)), Some((1, 6)));
        assert_eq!(found[1].text, "https://example.com/x");
        assert_eq!(found[1].at.len(), 2);
    }

    #[test]
    fn labels_are_prefix_free() {
        let alphabet: Vec<char> = "asdfghjklqwertyuiopzxcvbnm".chars().collect();
        for n in [1, 26, 27, 30, 200, 676] {
            let all = labels(n, &alphabet);
            assert_eq!(all.len(), n);
            for a in &all {
                assert!(
                    all.iter().filter(|b| b.starts_with(a.as_str())).count() == 1,
                    "{a} in {n}"
                );
            }
        }
    }
}
