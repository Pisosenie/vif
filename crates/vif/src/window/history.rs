//! ⇧⌘F: search everything. What this pane shows, what every open pane shows, or everything,
//! the commands kept in the archive with what they printed included; by words, or a regular
//! expression, with or without case.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use regex::Regex;
use vif_render::{Rect, Scene, Style};
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};

use super::keys::{quote, shell_run};
use super::{Shared, Win};
use crate::archive::{self, Entry};
use crate::colors::{mix, rgba};
use crate::layout::PaneId;
use crate::pane::Pane;

const SHOWN: usize = 9;
const LOADED: usize = 20_000;
/// Matches kept from one pane, newest first.
const PER_PANE: usize = 200;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Pane,
    Sessions,
    Everything,
}

const SCOPES: [(Scope, &str); 3] = [
    (Scope::Pane, "Pane"),
    (Scope::Sessions, "Open sessions"),
    (Scope::Everything, "Everything"),
];

#[derive(Clone)]
enum Hit {
    /// A line on a pane's grid, with the columns the match covers.
    Line {
        pane: PaneId,
        group: String,
        line: i32,
        text: String,
        span: (usize, usize),
    },
    Kept(Entry),
}

impl Hit {
    fn group(&self) -> String {
        match self {
            Hit::Line { group, .. } => group.clone(),
            Hit::Kept(_) => "kept commands".to_owned(),
        }
    }
}

pub(super) struct History {
    query: String,
    selected: usize,
    scope: Scope,
    regex: bool,
    case: bool,
    lines: Vec<String>,
    hits: Vec<Hit>,
    problem: Option<String>,
}

impl History {
    fn new() -> Self {
        Self {
            query: String::new(),
            selected: 0,
            scope: Scope::Everything,
            regex: false,
            case: false,
            lines: archive::lines(LOADED),
            hits: Vec::new(),
            problem: None,
        }
    }

    /// What the query looks for: itself, or a regular expression.
    fn matcher(&self) -> Result<Option<Regex>, String> {
        if self.query.trim().is_empty() {
            return Ok(None);
        }
        let pattern = if self.regex {
            self.query.clone()
        } else {
            regex::escape(self.query.trim())
        };
        let pattern = if self.case { pattern } else { format!("(?i){pattern}") };
        Regex::new(&pattern)
            .map(Some)
            .map_err(|_| "not a regular expression yet".to_owned())
    }
}

impl Win {
    pub(super) fn open_history(&mut self) {
        self.history = Some(History::new());
        self.refind();
    }

    fn refind(&mut self) {
        let Some(history) = &self.history else {
            return;
        };
        let matcher = history.matcher();
        let scope = history.scope;
        let mut hits = Vec::new();
        let mut problem = None;
        match matcher {
            Err(err) => problem = Some(err),
            Ok(matcher) => {
                if let Some(re) = &matcher {
                    let panes: Vec<(usize, PaneId)> = match scope {
                        Scope::Pane => self
                            .tabs
                            .get(self.active)
                            .map(|t| (self.active, t.focus))
                            .into_iter()
                            .collect(),
                        _ => self
                            .tabs
                            .iter()
                            .enumerate()
                            .flat_map(|(i, t)| {
                                let mut ids = Vec::new();
                                t.root.panes(&mut ids);
                                ids.into_iter().map(move |id| (i, id))
                            })
                            .collect(),
                    };
                    for (tab, id) in panes {
                        let tab = self.tab(tab);
                        let group = if tab.detail.is_empty() {
                            tab.title
                        } else {
                            format!("{} {}", tab.title, tab.detail)
                        };
                        if let Some(pane) = self.panes.get(&id) {
                            hits.extend(pane_hits(pane, id, &group, re));
                        }
                    }
                }
                if scope == Scope::Everything
                    && let Some(history) = &self.history
                {
                    hits.extend(kept(&history.lines, matcher.as_ref()).into_iter().map(Hit::Kept));
                }
            }
        }
        if let Some(history) = self.history.as_mut() {
            history.hits = hits;
            history.problem = problem;
            history.selected = 0;
        }
    }

    pub(super) fn history_key(&mut self, event: &KeyEvent, shared: &Shared) -> bool {
        if event.state != ElementState::Pressed {
            return false;
        }
        let (alt, ctrl) = (self.mods.state().alt_key(), self.mods.state().control_key());
        let Some(history) = self.history.as_mut() else {
            return false;
        };
        let shown = history.hits.len().max(1);
        let mut again = false;
        match (&event.logical_key, event.physical_key) {
            (_, PhysicalKey::Code(KeyCode::KeyR)) if alt => {
                history.regex = !history.regex;
                again = true;
            }
            (_, PhysicalKey::Code(KeyCode::KeyC)) if alt => {
                history.case = !history.case;
                again = true;
            }
            (Key::Named(NamedKey::Tab), _) => {
                let at = SCOPES.iter().position(|(s, _)| *s == history.scope).unwrap_or(0);
                history.scope = SCOPES[(at + 1) % SCOPES.len()].0;
                again = true;
            }
            (Key::Named(NamedKey::Escape), _) => self.history = None,
            (Key::Named(NamedKey::ArrowDown), _) => history.selected = (history.selected + 1) % shown,
            (Key::Named(NamedKey::ArrowUp), _) => history.selected = (history.selected + shown - 1) % shown,
            (Key::Named(NamedKey::Backspace), _) => {
                history.query.pop();
                again = true;
            }
            (Key::Named(NamedKey::Enter), _) => {
                let hit = history.hits.get(history.selected).cloned();
                let query = history.query.clone();
                self.history = None;
                match hit {
                    Some(hit) if ctrl => self.copy_hit(&hit),
                    Some(hit) => self.go_to(hit, &query, shared),
                    None => {}
                }
            }
            _ => {
                if let Some(text) = event
                    .text
                    .as_ref()
                    .filter(|t| !alt && !ctrl && !t.chars().any(char::is_control))
                {
                    history.query.push_str(text);
                    again = true;
                }
            }
        }
        if again {
            self.refind();
        }
        self.window.request_redraw();
        false
    }

    pub(super) fn history_type(&mut self, text: &str) {
        if let Some(history) = self.history.as_mut() {
            history.query.push_str(text);
            self.refind();
            self.window.request_redraw();
        }
    }

    fn copy_hit(&mut self, hit: &Hit) {
        let text = match hit {
            Hit::Line { text, .. } => text.trim_end().to_owned(),
            Hit::Kept(entry) => entry.output.clone(),
        };
        self.clipboard.set(text);
        self.flash("Copied");
    }

    /// Shows a hit where it is: the line in its pane, selected, or a kept command in a pager.
    fn go_to(&mut self, hit: Hit, query: &str, shared: &Shared) {
        match hit {
            Hit::Kept(entry) => self.open_entry(&entry, query, shared),
            Hit::Line { pane, line, span, .. } => {
                if let Some(tab) = self.tabs.iter().position(|t| t.contains(pane)) {
                    self.active = tab;
                    self.focus(pane);
                    self.relayout();
                    self.update_title();
                }
                if let Some(pane) = self.panes.get_mut(&pane) {
                    pane.reveal(line, line);
                    let mut term = pane.term.lock();
                    let start = Point::new(Line(line), Column(span.0));
                    let mut selection = Selection::new(SelectionType::Simple, start, Side::Left);
                    selection.update(Point::new(Line(line), Column(span.1)), Side::Right);
                    term.selection = Some(selection);
                }
            }
        }
    }

    /// Shows a kept command and all it printed in a pager, in a tab of its own.
    fn open_entry(&mut self, entry: &Entry, query: &str, shared: &Shared) {
        let text = format!(
            "$ {}\n# {} · exit {} · {} · {}\n\n{}\n",
            entry.command,
            entry.host.as_deref().unwrap_or(&entry.cwd),
            entry.exit,
            crate::marks::clock(std::time::Duration::from_secs_f32(entry.took)),
            archive::ago(entry.at),
            entry.output
        );
        let name = format!("history-{}", entry.at);
        self.page(shared, &name, &text, query.split_whitespace().next());
    }

    /// Shows text in a pager in a tab of its own, at the first `find` if given.
    pub(super) fn page(&mut self, shared: &Shared, name: &str, text: &str, find: Option<&str>) {
        let path = std::env::temp_dir().join(format!("vif-{name}.txt"));
        if let Err(err) = std::fs::write(&path, text) {
            return log::warn!("can't show it: {err}");
        }
        let find = find.map(|w| format!(" -p {}", quote(w))).unwrap_or_default();
        // The output is UTF-8 whatever the shell's locale says.
        let script = format!(
            "LESSCHARSET=utf-8 exec less -R{find} {}",
            quote(&path.to_string_lossy())
        );
        let run = shell_run(&self.config, &script);
        self.open_tab(shared, Some(&run), None);
    }

    pub(super) fn draw_history(&mut self, scene: &mut Scene) {
        let Some(history) = &self.history else {
            return;
        };
        let (query, selected, scope) = (history.query.clone(), history.selected, history.scope);
        let (regex, case, problem) = (history.regex, history.case, history.problem.clone());
        let total = history.hits.len();
        // The rows up to the picked one.
        let first = selected.saturating_sub(SHOWN - 1);
        let hits: Vec<(usize, Hit)> = history
            .hits
            .iter()
            .cloned()
            .enumerate()
            .skip(first)
            .take(SHOWN)
            .collect();
        let m = self.renderer.metrics();
        let scale = self.scale();
        let scheme = self.scheme;
        let (w, h) = self.size();
        let (cw, ch) = (m.cell_width, m.cell_height);
        let pad = 12.0 * scale;
        let width = (96.0 * cw + 2.0 * pad).min(w - 32.0 * scale);
        let cols = ((width - 2.0 * pad) / cw) as usize;
        let row_h = |hit: &Hit| match hit {
            Hit::Line { .. } => ch + 10.0 * scale,
            Hit::Kept(_) => ch * 2.0 + 14.0 * scale,
        };
        let mut groups = 0;
        let mut last = String::new();
        for (_, hit) in &hits {
            if hit.group() != last {
                groups += 1;
                last = hit.group();
            }
        }
        let body = hits.iter().map(|(_, hit)| row_h(hit)).sum::<f32>() + groups as f32 * (ch + 8.0 * scale);
        let height = 2.0 * (ch + 12.0 * scale) + body.max(ch + 14.0 * scale) + 2.0 * pad;
        let (x, y) = (((w - width) / 2.0).round(), (h * 0.08).round());
        let mut text = Scene::default();
        scene
            .front
            .push(Rect::fill(x, y, width, height, mix(scheme.fg(), scheme.bg(), 0.9)).rounded(self.radius()));
        let muted = mix(scheme.fg(), scheme.bg(), 0.45);
        let ty = (y + pad + 5.0 * scale).round();
        let typed = if query.is_empty() {
            let hint = "Search what the panes show and every command kept";
            self.label(hint, x + pad, ty, muted, Style::Regular, &mut text);
            0.0
        } else {
            self.label(&query, x + pad, ty, scheme.fg(), Style::Regular, &mut text)
        };
        scene.front.push(Rect::fill(
            x + pad + typed,
            ty,
            (2.0 * scale).round(),
            ch,
            scheme.accent(),
        ));
        // The toggles sit at the right of the query.
        let mut tx = x + width - pad;
        for (on, label) in [(case, "Aa"), (regex, ".*")] {
            let bw = 3.0 * cw;
            tx -= bw + 4.0 * scale;
            let fill = if on {
                let mut a = scheme.accent();
                a[3] = 0.35;
                a
            } else {
                mix(scheme.fg(), scheme.bg(), 0.82)
            };
            scene
                .front
                .push(Rect::fill(tx, ty - 3.0 * scale, bw, ch + 6.0 * scale, fill).rounded(self.radius() * 0.5));
            self.label(
                label,
                tx + (bw - 2.0 * cw) / 2.0,
                ty,
                scheme.fg(),
                Style::Regular,
                &mut text,
            );
        }
        let sy = (ty + ch + 12.0 * scale).round();
        let mut sx = x + pad;
        for (s, label) in SCOPES {
            let lw = label.chars().count() as f32 * cw;
            if s == scope {
                let mut tint = scheme.accent();
                tint[3] = 0.25;
                scene.front.push(
                    Rect::fill(
                        sx - 6.0 * scale,
                        sy - 3.0 * scale,
                        lw + 12.0 * scale,
                        ch + 6.0 * scale,
                        tint,
                    )
                    .rounded(self.radius() * 0.5),
                );
            }
            let color = if s == scope { scheme.fg() } else { muted };
            self.label(label, sx, sy, color, Style::Regular, &mut text);
            sx += lw + 20.0 * scale;
        }
        let count = match total {
            0 => String::new(),
            1 => "1 match".to_owned(),
            n => format!("{n} matches"),
        };
        let cx = x + width - pad - count.chars().count() as f32 * cw;
        self.label(&count, cx, sy, muted, Style::Regular, &mut text);
        let mut top = sy + ch + 10.0 * scale;
        scene
            .front
            .push(Rect::fill(x, top, width, 1.0, mix(scheme.fg(), scheme.bg(), 0.8)));
        if hits.is_empty() {
            let note = match (&problem, query.is_empty()) {
                (Some(problem), _) => problem.as_str(),
                (None, true) if scope != Scope::Everything => "type to search",
                _ if scope == Scope::Everything && self.config.terminal.archive_days == 0 => {
                    "keeping commands is off: archive_days = 0"
                }
                _ => "nothing like that",
            };
            self.label(
                note,
                x + pad,
                (top + 7.0 * scale).round(),
                muted,
                Style::Regular,
                &mut text,
            );
        }
        let red = rgba(scheme.palette.ansi[1], 1.0);
        let mut group = String::new();
        for (i, hit) in &hits {
            if hit.group() != group {
                group = hit.group();
                let g: String = group.chars().take(cols).collect();
                self.label(&g, x + pad, (top + 6.0 * scale).round(), muted, Style::Bold, &mut text);
                top += ch + 8.0 * scale;
            }
            let rh = row_h(hit);
            if *i == selected {
                let mut tint = scheme.accent();
                tint[3] = 0.22;
                scene.front.push(
                    Rect::fill(x + pad / 2.0, top + 2.0 * scale, width - pad, rh - 4.0 * scale, tint)
                        .rounded(self.radius() * 0.6),
                );
            }
            let l1 = (top + 5.0 * scale).round();
            match hit {
                Hit::Line { text: line, span, .. } => {
                    let room = cols.saturating_sub(2);
                    let shown: String = line.chars().take(room).collect();
                    let (from, to) = (span.0.min(room), (span.1 + 1).min(room));
                    let lx = x + pad + 2.0 * cw;
                    if to > from {
                        let mut mark = scheme.accent();
                        mark[3] = 0.45;
                        scene
                            .front
                            .push(Rect::fill(lx + from as f32 * cw, l1, (to - from) as f32 * cw, ch, mark));
                    }
                    self.label(&shown, lx, l1, scheme.fg(), Style::Regular, &mut text);
                }
                Hit::Kept(entry) => {
                    let (mark, color) = if entry.exit == 0 {
                        ("✓".to_owned(), muted)
                    } else {
                        (format!("✗ {}", entry.exit), red)
                    };
                    let mw = self.label(&mark, x + pad, l1, color, Style::Regular, &mut text);
                    let place = entry
                        .host
                        .clone()
                        .unwrap_or_else(|| crate::tilde(std::path::Path::new(&entry.cwd)));
                    let right = format!("{} · exit {} · {place}", archive::ago(entry.at), entry.exit);
                    let right: String = right.chars().take(cols / 3).collect();
                    let room = cols.saturating_sub(right.chars().count() + mark.chars().count() + 3);
                    let command: String = entry.command.chars().take(room).collect();
                    self.label(&command, x + pad + mw + cw, l1, scheme.fg(), Style::Bold, &mut text);
                    let rx = x + width - pad - right.chars().count() as f32 * cw;
                    self.label(&right, rx, l1, muted, Style::Regular, &mut text);
                    let line: String = archive::snippet(entry, &query).chars().take(cols - 2).collect();
                    self.label(&line, x + pad + 2.0 * cw, l1 + ch, muted, Style::Regular, &mut text);
                }
            }
            top += rh;
        }
        scene.over.extend(text.mask);
        let alt = if cfg!(target_os = "macos") { "⌥" } else { "Alt " };
        let (regex_key, case_key) = (format!("{alt}R"), format!("{alt}C"));
        let items = [
            ("↵", "open there"),
            ("Ctrl ↵", "copy"),
            ("Tab", "where"),
            (regex_key.as_str(), "regex"),
            (case_key.as_str(), "case"),
            ("Esc", "close"),
        ];
        self.draw_footer("Search everything", "", &items, &[], scene);
    }
}

/// A pane's lines that match, newest first, as screen text with the matched columns.
fn pane_hits(pane: &Pane, id: PaneId, group: &str, re: &Regex) -> Vec<Hit> {
    let term = pane.term.lock();
    let grid = term.grid();
    let (first, last) = (-(grid.history_size() as i32), grid.screen_lines() as i32 - 1);
    let mut out = Vec::new();
    for line in (first..=last).rev() {
        let row = &grid[Line(line)];
        let mut text = String::new();
        let mut cols = Vec::new();
        for c in 0..row.len() {
            let cell = &row[Column(c)];
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            cols.push((text.len(), c));
            text.push(cell.c);
        }
        let Some(found) = re.find(&text) else {
            continue;
        };
        let col = |byte: usize| cols.iter().rev().find(|(b, _)| *b <= byte).map_or(0, |(_, c)| *c);
        let span = (
            col(found.start()),
            col(found.end().saturating_sub(1).max(found.start())),
        );
        out.push(Hit::Line {
            pane: id,
            group: group.to_owned(),
            line,
            text: text.trim_end().to_owned(),
            span,
        });
        if out.len() >= PER_PANE {
            break;
        }
    }
    out
}

/// Kept commands that match, newest first; with no query, the newest of them.
fn kept(lines: &[String], re: Option<&Regex>) -> Vec<Entry> {
    let Some(re) = re else {
        return archive::find(lines, "", SHOWN);
    };
    let mut out = Vec::new();
    for line in lines.iter().filter(|l| re.is_match(l)) {
        let Ok(entry) = serde_json::from_str::<Entry>(line) else {
            continue;
        };
        if re.is_match(&entry.command) || re.is_match(&entry.output) {
            out.push(entry);
            if out.len() >= 200 {
                break;
            }
        }
    }
    out
}
