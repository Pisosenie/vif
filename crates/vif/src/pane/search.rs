//! Searching what a pane shows and keeps, for the search bar.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::search::RegexSearch;

use super::*;

impl Pane {
    pub fn search_open(&mut self) {
        self.hints = None;
        if self.search.is_some() {
            return;
        }
        let term = self.term.lock();
        let bottom = term.screen_lines() as i32 - 1 - term.grid().display_offset() as i32;
        let origin = Point::new(Line(bottom), Column(term.columns() - 1));
        self.search = Some(Search {
            query: String::new(),
            regex: None,
            current: None,
            origin,
            missed: false,
        });
    }

    pub fn search_close(&mut self) {
        self.search = None;
        self.term.lock().selection = None;
    }

    pub fn search_edit(&mut self, edit: impl FnOnce(&mut String)) {
        let Some(search) = &mut self.search else {
            return;
        };
        edit(&mut search.query);
        search.regex = (!search.query.is_empty())
            .then(|| RegexSearch::new(&literal(&search.query)).ok())
            .flatten();
        search.current = None;
        let origin = search.origin;
        self.find(origin, Direction::Left, Side::Right);
    }

    /// The next match further back, or forward again with `older` false.
    pub fn search_step(&mut self, older: bool) {
        let Some(search) = &self.search else {
            return;
        };
        let term = self.term.lock();
        let (origin, direction, side) = match (&search.current, older) {
            (Some(m), true) => (m.start().sub(&*term, Boundary::None, 1), Direction::Left, Side::Right),
            (Some(m), false) => (m.end().add(&*term, Boundary::None, 1), Direction::Right, Side::Left),
            (None, _) => (search.origin, Direction::Left, Side::Right),
        };
        drop(term);
        self.find(origin, direction, side);
    }

    pub(super) fn find(&mut self, origin: Point, direction: Direction, side: Side) {
        let Some(search) = &mut self.search else {
            return;
        };
        let mut term = self.term.lock();
        let Some(regex) = &mut search.regex else {
            search.missed = false;
            term.selection = None;
            return;
        };
        let hit = term.search_next(regex, origin, direction, side, None);
        search.missed = hit.is_none();
        match &hit {
            Some(m) => {
                let mut selection = Selection::new(SelectionType::Simple, *m.start(), Side::Left);
                selection.update(*m.end(), Side::Right);
                term.selection = Some(selection);
                term.scroll_to_point(*m.start());
            }
            None => term.selection = None,
        }
        search.current = hit.or(search.current.take());
    }
}

pub(super) fn literal(query: &str) -> String {
    let mut out = String::with_capacity(query.len() * 2);
    for c in query.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
