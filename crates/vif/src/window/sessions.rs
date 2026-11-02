//! The sessions dialog: every shell there is, as cards with what is on its screen, to take here,
//! open in a window of its own, let go of, or name.

use std::time::SystemTime;

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config as TermConfig, Term};
use alacritty_terminal::vte::ansi::Processor;
use vif_render::{RECT_HOLLOW, Rect, Scene, Style};
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, NamedKey};

use super::{Shared, Win};
use crate::app::UserEvent;
use crate::colors::mix;
use crate::layout::{Area, PaneId};
use crate::session;

const PREVIEW: usize = 4;
const COLUMNS: usize = 3;

struct Card {
    id: String,
    name: Option<String>,
    what: String,
    status: String,
    preview: Vec<String>,
    /// The pane here that has it.
    here: Option<PaneId>,
}

#[derive(Default)]
pub(super) struct Sessions {
    cards: Vec<Card>,
    query: String,
    selected: usize,
    /// Typing names the picked session instead of filtering.
    renaming: bool,
}

impl Win {
    pub(super) fn open_sessions(&mut self) {
        let mine: Vec<(String, PaneId)> = self
            .panes
            .iter()
            .filter_map(|(id, pane)| Some((pane.session.clone()?, *id)))
            .collect();
        let cards = session::all()
            .into_iter()
            .map(|s| {
                let here = mine.iter().find(|(id, _)| *id == s.id).map(|(_, pane)| *pane);
                let status = match (here, s.attached, s.since) {
                    (Some(_), _, _) if self.quick => "quick terminal".to_owned(),
                    (Some(_), _, _) => "this window".to_owned(),
                    (None, true, _) => "running · in another window".to_owned(),
                    (None, false, Some(since)) => format!("running · window closed {}", ago(since)),
                    (None, false, None) => "running · no window".to_owned(),
                };
                let preview = match here.and_then(|p| self.panes.get(&p)) {
                    Some(pane) => tail(&pane.term.lock(), PREVIEW),
                    None => session::peek(&s.id, 32 * 1024).map(|b| screen(&b)).unwrap_or_default(),
                };
                Card {
                    id: s.id,
                    name: s.name,
                    what: s.what,
                    status,
                    preview,
                    here,
                }
            })
            .collect();
        self.sessions = Some(Sessions {
            cards,
            ..Sessions::default()
        });
        self.window.request_redraw();
    }

    fn shown_cards(&self) -> Vec<usize> {
        let Some(s) = &self.sessions else {
            return Vec::new();
        };
        let query = if s.renaming {
            String::new()
        } else {
            s.query.to_lowercase()
        };
        s.cards
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                let text = format!("{} {} {}", c.name.as_deref().unwrap_or(""), c.what, c.status).to_lowercase();
                query.split_whitespace().all(|w| text.contains(w))
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub(super) fn sessions_key(&mut self, event: &KeyEvent, shared: &Shared) {
        if event.state != ElementState::Pressed {
            return;
        }
        let shown = self.shown_cards();
        let ctrl = self.mods.state().control_key();
        let Some(s) = self.sessions.as_mut() else {
            return;
        };
        let picked = shown.get(s.selected).copied();
        match &event.logical_key {
            Key::Named(NamedKey::Escape) if s.renaming => {
                s.renaming = false;
                s.query.clear();
            }
            Key::Named(NamedKey::Escape) => self.sessions = None,
            Key::Named(NamedKey::ArrowRight) => s.selected = (s.selected + 1).min(shown.len().saturating_sub(1)),
            Key::Named(NamedKey::ArrowLeft) => s.selected = s.selected.saturating_sub(1),
            Key::Named(NamedKey::ArrowDown) => s.selected = (s.selected + COLUMNS).min(shown.len().saturating_sub(1)),
            Key::Named(NamedKey::ArrowUp) => s.selected = s.selected.saturating_sub(COLUMNS),
            Key::Named(NamedKey::Backspace) => {
                s.query.pop();
                if !s.renaming {
                    s.selected = 0;
                }
            }
            Key::Named(NamedKey::Enter) if s.renaming => {
                if let Some(card) = picked.and_then(|i| s.cards.get_mut(i)) {
                    let name = s.query.trim().to_owned();
                    if let Err(err) = session::rename(&card.id, &name) {
                        log::warn!("can't name the session: {err}");
                    }
                    card.name = Some(name).filter(|n| !n.is_empty());
                    if let Some(pane) = card.here.and_then(|p| self.panes.get_mut(&p)) {
                        pane.name = card.name.clone();
                    }
                }
                s.renaming = false;
                s.query.clear();
            }
            Key::Named(NamedKey::Enter) => {
                let Some(card) = picked.and_then(|i| s.cards.get(i)) else {
                    return;
                };
                let (id, here) = (card.id.clone(), card.here);
                self.sessions = None;
                if ctrl && here.is_none() {
                    let _ = shared.proxy.send_event(UserEvent::OpenSession(id));
                } else {
                    self.take_session(shared, id, here);
                }
            }
            Key::Character(c) if ctrl && c.as_str() == "d" => {
                if let Some(pane) = picked.and_then(|i| s.cards.get(i)).and_then(|c| c.here) {
                    self.sessions = None;
                    if self.detach_pane(pane) {
                        let _ = shared.proxy.send_event(UserEvent::CloseWindow(self.window.id()));
                    }
                }
            }
            Key::Character(c) if ctrl && c.as_str() == "r" => {
                if let Some(card) = picked.and_then(|i| s.cards.get(i)) {
                    s.query = card.name.clone().unwrap_or_default();
                    s.renaming = true;
                }
            }
            _ => {
                if let Some(text) = event
                    .text
                    .as_ref()
                    .filter(|t| !ctrl && !t.chars().any(char::is_control))
                {
                    s.query.push_str(text);
                    if !s.renaming {
                        s.selected = 0;
                    }
                }
            }
        }
        self.window.request_redraw();
    }

    /// Focuses the pane that has a session here, or takes the session into a tab.
    fn take_session(&mut self, shared: &Shared, id: String, here: Option<PaneId>) {
        if let Some(pane) = here {
            if let Some(tab) = self.tabs.iter().position(|t| t.contains(pane)) {
                self.active = tab;
                self.focus(pane);
                self.relayout();
                self.update_title();
            }
            return;
        }
        if let Err(err) = self.attach_tab(shared, id) {
            log::warn!("can't take the session: {err}");
        }
    }

    pub(super) fn sessions_type(&mut self, text: &str) {
        if let Some(s) = self.sessions.as_mut() {
            s.query.push_str(text);
            self.window.request_redraw();
        }
    }

    pub(super) fn draw_sessions(&mut self, scene: &mut Scene) {
        if self.sessions.is_none() {
            return;
        }
        let shown = self.shown_cards();
        let Some(s) = &self.sessions else {
            return;
        };
        let (query, renaming, selected) = (s.query.clone(), s.renaming, s.selected);
        let running = s.cards.len();
        let cards: Vec<(Option<String>, String, String, Vec<String>)> = shown
            .iter()
            .filter_map(|&i| s.cards.get(i))
            .map(|c| (c.name.clone(), c.what.clone(), c.status.clone(), c.preview.clone()))
            .collect();
        let m = self.renderer.metrics();
        let scale = self.scale();
        let scheme = self.scheme;
        let (w, h) = self.size();
        let (cw, ch) = (m.cell_width, m.cell_height);
        let pad = (14.0 * scale).round();
        let gap = (10.0 * scale).round();
        let width = (w - 48.0 * scale).min(COLUMNS as f32 * 34.0 * cw + 2.0 * pad + gap * 2.0);
        let card_w = ((width - 2.0 * pad - gap * (COLUMNS as f32 - 1.0)) / COLUMNS as f32).floor();
        let card_h = (PREVIEW as f32 + 2.6) * ch + 2.0 * pad;
        let rows = cards.len().div_ceil(COLUMNS).max(1);
        let height = (2.0 * ch + pad * 3.0 + rows as f32 * (card_h + gap)).min(h - 96.0 * scale);
        let (x, y) = (((w - width) / 2.0).round(), (h * 0.08).round());
        let panel = mix(scheme.fg(), scheme.bg(), 0.92);
        scene
            .front
            .push(Rect::fill(x, y, width, height, panel).rounded(self.radius() * 1.5));
        let mut text = Scene::default();
        let muted = mix(scheme.fg(), scheme.bg(), 0.45);
        let ty = (y + pad).round();
        let tw = self.label("Sessions", x + pad, ty, scheme.fg(), Style::Bold, &mut text);
        self.label(
            &format!("{running}"),
            x + pad + tw + cw,
            ty,
            muted,
            Style::Regular,
            &mut text,
        );
        let (hint, typed) = if renaming {
            ("Name it", query.as_str())
        } else {
            ("Filter", query.as_str())
        };
        let field_x = x + width - pad - 30.0 * cw;
        let field = mix(scheme.fg(), scheme.bg(), 0.85);
        scene.front.push(
            Rect::fill(
                field_x - 6.0 * scale,
                ty - 4.0 * scale,
                30.0 * cw,
                ch + 8.0 * scale,
                field,
            )
            .rounded(self.radius() * 0.6),
        );
        let shown_text = if typed.is_empty() { hint } else { typed };
        let color = if typed.is_empty() { muted } else { scheme.fg() };
        let fw = self.label(shown_text, field_x, ty, color, Style::Regular, &mut text);
        let caret_x = if typed.is_empty() { field_x } else { field_x + fw };
        scene
            .front
            .push(Rect::fill(caret_x, ty, (2.0 * scale).round(), ch, scheme.accent()));
        let top = ty + ch + pad;
        let mut areas = Vec::new();
        for (i, (name, what, status, preview)) in cards.iter().enumerate() {
            let (col, row) = (i % COLUMNS, i / COLUMNS);
            let cx = x + pad + col as f32 * (card_w + gap);
            let cy = top + row as f32 * (card_h + gap);
            if cy + card_h > y + height {
                break;
            }
            let fill = mix(scheme.fg(), scheme.bg(), 0.88);
            scene
                .front
                .push(Rect::fill(cx, cy, card_w, card_h, fill).rounded(self.radius()));
            let screen_bg = mix(scheme.fg(), scheme.bg(), 0.97);
            let sh = PREVIEW as f32 * ch + 12.0 * scale;
            scene.front.push(
                Rect::fill(cx + 6.0 * scale, cy + 6.0 * scale, card_w - 12.0 * scale, sh, screen_bg)
                    .rounded(self.radius() * 0.6),
            );
            let room = ((card_w - 2.0 * pad) / cw) as usize;
            for (l, line) in preview.iter().enumerate() {
                let line: String = line.chars().take(room).collect();
                let ly = cy + 12.0 * scale + l as f32 * ch;
                self.label(&line, cx + pad, ly, muted, Style::Regular, &mut text);
            }
            let ny = cy + sh + 14.0 * scale;
            let title: String = name
                .clone()
                .unwrap_or_else(|| what.clone())
                .chars()
                .take(room)
                .collect();
            self.label(&title, cx + pad, ny, scheme.fg(), Style::Bold, &mut text);
            let status: String = status.chars().take(room).collect();
            let dot = if status.starts_with("this") || status.starts_with("quick") {
                scheme.accent()
            } else {
                crate::colors::rgba(scheme.palette.ansi[2], 1.0)
            };
            let d = (ch * 0.36).round();
            scene
                .front
                .push(Rect::fill(cx + pad, ny + ch + (ch - d) / 2.0, d, d, dot).rounded(d / 2.0));
            self.label(&status, cx + pad + 1.6 * cw, ny + ch, muted, Style::Regular, &mut text);
            if i == selected {
                let ring = Rect::fill(cx, cy, card_w, card_h, scheme.accent()).kind(RECT_HOLLOW, 2.0 * scale);
                scene.front.push(ring.rounded(self.radius()));
            }
            areas.push(Area {
                x: cx,
                y: cy,
                w: card_w,
                h: card_h,
            });
        }
        if cards.is_empty() {
            self.label("no sessions like that", x + pad, top, muted, Style::Regular, &mut text);
        }
        scene.over.extend(text.mask);
        let items: &[(&str, &str)] = &[
            ("↵", "take it here"),
            ("Ctrl ↵", "new window"),
            ("Ctrl D", "let go"),
            ("Ctrl R", "rename"),
            ("Esc", "close"),
        ];
        self.draw_footer("Sessions", "", items, &[], scene);
    }
}

/// A screen's last non-empty lines, oldest first.
fn tail(term: &Term<crate::pane::Proxy>, count: usize) -> Vec<String> {
    lines_of(term.grid(), count)
}

fn lines_of(grid: &alacritty_terminal::grid::Grid<alacritty_terminal::term::cell::Cell>, count: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in (0..grid.screen_lines() as i32).rev() {
        let row = &grid[Line(line)];
        let text: String = (0..row.len())
            .map(|c| row[Column(c)].c)
            .collect::<String>()
            .trim_end()
            .to_owned();
        if text.is_empty() && out.is_empty() {
            continue;
        }
        out.push(text);
        if out.len() == count {
            break;
        }
    }
    out.reverse();
    out
}

/// What a session's last output puts on a screen, read through a terminal of its own.
fn screen(bytes: &[u8]) -> Vec<String> {
    let mut term = Term::new(TermConfig::default(), &TermSize::new(120, 24), VoidListener);
    let mut parser: Processor = Processor::new();
    parser.advance(&mut term, bytes);
    // A lone % is zsh marking output that didn't end its line.
    lines_of(term.grid(), PREVIEW + 1)
        .into_iter()
        .map(|l| l.chars().filter(|c| !crate::marks::is_tag(*c)).collect::<String>())
        .filter(|l| l.trim() != "%")
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .take(PREVIEW)
        .rev()
        .collect()
}

fn ago(at: SystemTime) -> String {
    let secs = at.elapsed().map_or(0, |d| d.as_secs());
    match secs {
        0..60 => "just now".to_owned(),
        60..3600 => format!("{}m ago", secs / 60),
        3600..86_400 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86_400),
    }
}
