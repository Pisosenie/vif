//! The scrollbar: where the screen sits in the scrollback, marked where prompts, failed commands
//! and search matches are. It shows while the view is scrolled back or a search is open.

use std::time::{Duration, Instant};

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use vif_render::{Rect, Scene};

use super::Win;
use crate::colors::{mix, rgba};
use crate::layout::PaneId;
use crate::marks::{self, Kind};

/// How long marks stay before they are looked for again.
const FRESH: Duration = Duration::from_millis(400);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tick {
    Prompt,
    Failed,
    Match,
}

/// Marks found in a pane's scrollback: when, how big it was, and at which grid lines.
pub(super) struct Ticks {
    at: Instant,
    history: usize,
    query: Option<String>,
    marks: Vec<(i32, Tick)>,
}

impl Win {
    pub(super) fn draw_scrollbars(&mut self, scene: &mut Scene) {
        let scale = self.scale();
        let scheme = self.scheme;
        for (id, _) in self.areas.clone() {
            let Some(pane) = self.panes.get(&id) else {
                continue;
            };
            let query = pane
                .search
                .as_ref()
                .map(|s| s.query.to_lowercase())
                .filter(|q| !q.is_empty());
            let (offset, history, lines) = {
                let term = pane.term.lock();
                let grid = term.grid();
                (grid.display_offset(), grid.history_size(), grid.screen_lines())
            };
            if history == 0 || (offset == 0 && pane.search.is_none()) {
                continue;
            }
            let stale = self
                .ticks
                .get(&id)
                .is_none_or(|t| t.at.elapsed() > FRESH || t.history != history || t.query != query);
            if stale {
                let marks = find_ticks(pane, query.as_deref());
                self.ticks.insert(
                    id,
                    Ticks {
                        at: Instant::now(),
                        history,
                        query,
                        marks,
                    },
                );
            }
            let Some(pane) = self.panes.get(&id) else {
                continue;
            };
            let (outer, area) = (pane.outer, pane.area);
            let total = (history + lines) as f32;
            let w = (4.0 * scale).round();
            let x = (outer.x + outer.w - w - 2.0 * scale).round();
            let (y, h) = (area.y, area.h);
            let track = mix(scheme.fg(), scheme.bg(), 0.9);
            scene.front.push(Rect::fill(x, y, w, h, track).rounded(w / 2.0));
            let thumb_h = (lines as f32 / total * h).max(16.0 * scale);
            // 0 with the oldest line at the top, 1 at the bottom of the scrollback.
            let along = (history - offset.min(history)) as f32 / history as f32;
            let thumb_y = (y + along * (h - thumb_h)).round();
            let thumb = mix(scheme.fg(), scheme.bg(), 0.6);
            scene
                .front
                .push(Rect::fill(x, thumb_y, w, thumb_h, thumb).rounded(w / 2.0));
            let colors = [
                (Tick::Prompt, mix(scheme.fg(), scheme.bg(), 0.35)),
                (Tick::Failed, rgba(scheme.palette.ansi[1], 1.0)),
                (Tick::Match, scheme.accent()),
            ];
            for (line, tick) in self.ticks.get(&id).map(|t| t.marks.as_slice()).unwrap_or_default() {
                let at = y + (line + history as i32) as f32 / total * h;
                let color = colors.iter().find(|(t, _)| t == tick).map_or(thumb, |(_, c)| *c);
                scene.front.push(Rect::fill(
                    x - scale,
                    at.round(),
                    w + 2.0 * scale,
                    (2.0 * scale).round(),
                    color,
                ));
            }
        }
    }

    pub(super) fn forget_ticks(&mut self, id: PaneId) {
        self.ticks.remove(&id);
    }
}

/// Prompts, failed commands, and lines holding `query`, by grid line.
fn find_ticks(pane: &crate::pane::Pane, query: Option<&str>) -> Vec<(i32, Tick)> {
    let term = pane.term.lock();
    let grid = term.grid();
    let (first, last) = (-(grid.history_size() as i32), grid.screen_lines() as i32 - 1);
    let mut out = Vec::new();
    for line in first..=last {
        let row = &grid[Line(line)];
        let mut tick = None;
        for c in 0..row.len() {
            let Some(zw) = row[Column(c)].zerowidth() else {
                continue;
            };
            if let Some(id) = marks::find(zw, Kind::Command) {
                let failed = pane
                    .blocks
                    .get(id)
                    .and_then(|b| b.ended)
                    .is_some_and(|(_, code)| code != 0);
                tick = Some(if failed { Tick::Failed } else { Tick::Prompt });
                break;
            }
        }
        if tick.is_none()
            && let Some(query) = query
        {
            let text: String = (0..row.len())
                .map(|c| row[Column(c)].c)
                .collect::<String>()
                .to_lowercase();
            if text.contains(query) {
                tick = Some(Tick::Match);
            }
        }
        if let Some(tick) = tick {
            out.push((line, tick));
        }
    }
    out
}
