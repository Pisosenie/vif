//! Commands as blocks on the grid: which ran, where each one sits, for block mode.

use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line};

use crate::marks::{self, Kind};
use crate::pane::{Pane, output_lines, row_has};

impl Pane {
    /// Commands that ran and whose prompt is still in the scrollback, oldest first.
    pub fn ran(&self) -> Vec<u32> {
        let term = self.term.lock();
        let grid = term.grid();
        let (first, last) = (-(grid.history_size() as i32), grid.screen_lines() as i32 - 1);
        let mut ids = Vec::new();
        for line in first..=last {
            let row = &grid[Line(line)];
            for c in 0..row.len() {
                if let Some(id) = row[Column(c)].zerowidth().and_then(|z| marks::find(z, Kind::Command))
                    && self.blocks.get(id).is_some()
                    && ids.last() != Some(&id)
                {
                    ids.push(id);
                }
            }
        }
        ids
    }

    /// A block's first and last grid line: its prompt through what it printed.
    pub fn span(&self, id: u32) -> Option<(i32, i32)> {
        let term = self.term.lock();
        let grid = term.grid();
        let (first, last) = (-(grid.history_size() as i32), grid.screen_lines() as i32 - 1);
        let command = (first..=last)
            .rev()
            .find(|&l| row_has(grid, l, |z| marks::find(z, Kind::Command) == Some(id)))?;
        let top = (first..=command)
            .rev()
            .take(8)
            .find(|&l| row_has(grid, l, |z| marks::find(z, Kind::Prompt) == Some(id)))
            .unwrap_or(command);
        let bottom = output_lines(&term, id).map_or(command, |(_, end)| end.max(command));
        Some((top, bottom))
    }

    /// Scrolls just enough that grid lines `top` to `bottom` show, or at least `top`.
    pub fn reveal(&mut self, top: i32, bottom: i32) {
        let mut term = self.term.lock();
        let offset = term.grid().display_offset() as i32;
        let lines = term.grid().screen_lines() as i32;
        let (shown_top, shown_bottom) = (-offset, lines - 1 - offset);
        let delta = if top < shown_top {
            shown_top - top
        } else if bottom > shown_bottom {
            -(bottom - shown_bottom).min(top - shown_top)
        } else {
            0
        };
        if delta != 0 {
            term.scroll_display(Scroll::Delta(delta));
        }
    }

    /// Whether a command runs now.
    pub fn busy(&self) -> bool {
        self.blocks
            .last()
            .and_then(|id| self.blocks.get(id))
            .is_some_and(|b| b.ended.is_none())
    }
}
