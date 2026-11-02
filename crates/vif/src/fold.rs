//! Folded blocks: a command's output drawn as one line that says how much it hides.
//!
//! While a fold is on screen each screen line knows the grid line it shows, counted up from the
//! bottom of the screen; everything that turns a screen line into a grid line asks [`Pane::shows`].

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::Term;
use vif_render::{Rect, Scene, Style};

use crate::colors::mix;
use crate::pane::{Look, Pane, Proxy, output_lines};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shown {
    Line(i32),
    Folded { id: u32, lines: usize },
    Blank,
}

impl Pane {
    /// What each screen line shows when a fold is on screen; None draws the grid as it is.
    pub(crate) fn fold_view(&self, term: &Term<Proxy>) -> Option<Vec<Shown>> {
        if self.folds.is_empty() || self.search.is_some() || self.hints.is_some() {
            return None;
        }
        let grid = term.grid();
        let offset = grid.display_offset() as i32;
        let first = -(grid.history_size() as i32);
        let folds: Vec<(u32, i32, i32)> = self
            .folds
            .iter()
            .filter_map(|&id| output_lines(term, id).map(|(start, end)| (id, start, end)))
            .filter(|(_, start, end)| end > start)
            .collect();
        let mut rows = Vec::with_capacity(self.lines);
        let mut line = self.lines as i32 - 1 - offset;
        while rows.len() < self.lines && line >= first {
            match folds.iter().find(|(_, start, end)| (*start..=*end).contains(&line)) {
                Some(&(id, start, end)) => {
                    rows.push(Shown::Folded {
                        id,
                        lines: (end - start + 1) as usize,
                    });
                    line = start - 1;
                }
                None => {
                    rows.push(Shown::Line(line));
                    line -= 1;
                }
            }
        }
        if !rows.iter().any(|r| matches!(r, Shown::Folded { .. })) {
            return None;
        }
        rows.reverse();
        rows.resize(self.lines, Shown::Blank);
        Some(rows)
    }

    /// What a screen line shows, for a caller that holds the grid and knows its offset.
    pub(crate) fn shown(&self, offset: i32, line: usize) -> Shown {
        match &self.view {
            Some(view) => view.get(line).copied().unwrap_or(Shown::Blank),
            None => Shown::Line(line as i32 - offset),
        }
    }

    /// The screen line showing a grid line, for a caller that holds the grid.
    pub(crate) fn screen_of(&self, offset: i32, line: i32) -> Option<usize> {
        match &self.view {
            Some(view) => view.iter().position(|s| *s == Shown::Line(line)),
            None => {
                let at = line + offset;
                (at >= 0 && (at as usize) < self.lines).then_some(at as usize)
            }
        }
    }

    /// What a screen line shows.
    pub fn shows(&self, line: usize) -> Shown {
        let offset = self.term.lock().grid().display_offset() as i32;
        self.shown(offset, line)
    }

    /// The fold drawn on a screen line, for a click to open it.
    pub fn fold_at(&self, line: usize) -> Option<u32> {
        match self.shows(line) {
            Shown::Folded { id, .. } => Some(id),
            _ => None,
        }
    }
}

/// The line a fold is drawn as: a pill with a chevron and how many lines it holds.
pub(crate) fn folded_row(r: &mut vif_render::Renderer, look: &Look, lines: usize, x: f32, y: f32) -> Scene {
    let m = r.metrics();
    let scheme = look.scheme;
    let text = format!("▸ {lines} lines folded");
    let w = (text.chars().count() as f32 + 2.0) * m.cell_width;
    let mut scene = Scene::default();
    let fill = mix(scheme.fg(), scheme.bg(), 0.88);
    scene
        .back
        .push(Rect::fill(x, y + 1.0, w, m.cell_height - 2.0, fill).rounded(m.cell_height / 2.0));
    let color = mix(scheme.fg(), scheme.bg(), 0.35);
    vif_render::chrome::label(r, &text, x + m.cell_width, y, color, Style::Regular, &mut scene);
    scene
}
