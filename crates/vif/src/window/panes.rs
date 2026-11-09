//! A window's panes and tabs coming and going: built from what a restart kept, split, opened,
//! closed and focused.

use std::path::PathBuf;

use alacritty_terminal::term::TermMode;

use super::*;
use crate::layout::{Area, Node, PaneId};
use crate::layouts::{self, Saved, Shape};
use crate::pane::{Pane, Spawn};

impl Win {
    /// Takes over a saved window's sessions, in its tabs and splits.
    pub(super) fn bring_back(&mut self, shared: &Shared, saved: Saved) {
        let Saved {
            tabs,
            active,
            places,
            restart,
            ..
        } = saved;
        let fresh = restart.then_some(&places);
        for tab in tabs {
            let Some(root) = self.build(shared, tab.root, fresh) else {
                continue;
            };
            let mut ids = Vec::new();
            root.panes(&mut ids);
            let focus = ids
                .iter()
                .copied()
                .find(|id| self.panes.get(id).and_then(|p| p.session.as_deref()) == Some(tab.focus.as_str()))
                .unwrap_or(ids[0]);
            self.tabs.push(Tab {
                root,
                focus,
                back: None,
                zoomed: false,
            });
        }
        self.active = active.min(self.tabs.len().saturating_sub(1));
        if !self.tabs.is_empty() {
            self.relayout();
            self.update_title();
        }
    }

    /// Panes for `shape`: its sessions taken over, or with `fresh`, new shells in the folders
    /// the old ones were in, under what those printed last. The splits are laid out first, so
    /// every pane starts at the size it will have.
    pub(super) fn build(
        &mut self,
        shared: &Shared,
        shape: Shape,
        fresh: Option<&std::collections::BTreeMap<String, PathBuf>>,
    ) -> Option<Node> {
        let mut sessions = Vec::new();
        let mut root = plan(shape, &mut sessions);
        let mut areas = Vec::new();
        root.layout(self.content(), self.gap(), &mut areas);
        for (id, session) in sessions {
            let area = crate::layout::area_of(&areas, id).unwrap_or(self.content());
            let (cwd, attach, before) = match fresh {
                Some(places) => (places.get(&session).cloned(), None, layouts::last_output(&session)),
                None => (None, Some(session), None),
            };
            let spawned = self.spawn_with(id, area, shared, None, cwd, attach, before);
            if let Err(err) = spawned {
                log::warn!("can't take a session back: {err}");
                if !root.remove(id) {
                    return None;
                }
            }
        }
        Some(root)
    }

    pub(super) fn spawn_pane(
        &mut self,
        shared: &Shared,
        run: Option<&[String]>,
        cwd: Option<PathBuf>,
        attach: Option<String>,
    ) -> std::io::Result<PaneId> {
        let id = crate::app::next_pane();
        self.spawn_with(id, self.content(), shared, run, cwd, attach, None)?;
        Ok(id)
    }

    /// A pane started at the size of `area`, so what its shell draws first fits.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn spawn_with(
        &mut self,
        id: PaneId,
        area: Area,
        shared: &Shared,
        run: Option<&[String]>,
        cwd: Option<PathBuf>,
        attach: Option<String>,
        before: Option<Vec<u8>>,
    ) -> std::io::Result<()> {
        let spawn = Spawn {
            config: &self.config,
            cwd: cwd.or_else(|| self.focused_pane().and_then(Pane::cwd)),
            term: &shared.term,
            command: run.or_else(|| shared.command.as_deref().filter(|_| self.panes.is_empty())),
            proxy: shared.proxy.clone(),
            window: self.window.id(),
            attach,
            first: self.panes.is_empty(),
            before,
        };
        let pane = Pane::spawn(id, spawn, area, self.renderer.metrics())?;
        self.panes.insert(id, pane);
        Ok(())
    }

    pub(super) fn new_tab(&mut self, shared: &Shared) -> std::io::Result<()> {
        self.push_tab(shared, None)
    }

    /// A tab for a session a closed window left running.
    pub(super) fn attach_tab(&mut self, shared: &Shared, session: String) -> std::io::Result<()> {
        self.push_tab(shared, Some(session))
    }

    pub(super) fn push_tab(&mut self, shared: &Shared, attach: Option<String>) -> std::io::Result<()> {
        let id = self.spawn_pane(shared, None, None, attach)?;
        self.tabs.push(Tab {
            root: Node::Pane(id),
            focus: id,
            back: None,
            zoomed: false,
        });
        self.active = self.tabs.len() - 1;
        self.relayout();
        self.update_title();
        Ok(())
    }

    /// A tab next to this one that runs `run` (or the shell) and comes back here when it ends.
    pub(super) fn open_tab(&mut self, shared: &Shared, run: Option<&[String]>, cwd: Option<PathBuf>) {
        let back = self.tabs.get(self.active).map(|t| t.focus);
        let id = match self.spawn_pane(shared, run, cwd, None) {
            Ok(id) => id,
            Err(err) => return log::warn!("can't open a tab: {err}"),
        };
        let at = (self.active + 1).min(self.tabs.len());
        self.tabs.insert(
            at,
            Tab {
                root: Node::Pane(id),
                focus: id,
                back,
                zoomed: false,
            },
        );
        self.active = at;
        self.relayout();
        self.update_title();
    }

    pub(super) fn split(&mut self, shared: &Shared, across: bool) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let focus = tab.focus;
        if std::mem::take(&mut tab.zoomed) {
            self.relayout();
        }
        let from = self
            .areas
            .iter()
            .find(|(p, _)| *p == focus)
            .map_or(self.content(), |(_, a)| *a);
        let id = crate::app::next_pane();
        let area = crate::layout::split_off(from, self.gap(), across);
        if self.spawn_with(id, area, shared, None, None, None, None).is_err() {
            return;
        }
        let tab = &mut self.tabs[self.active];
        tab.root.split(focus, id, across);
        tab.focus = id;
        self.relayout();
    }

    /// Closes a pane, ending its shell; returns true when the window has nothing left.
    pub fn close_pane(&mut self, id: PaneId) -> bool {
        self.remove_pane(id, true)
    }

    /// Takes a pane out of the window and leaves its session running; true when nothing is left.
    pub(super) fn detach_pane(&mut self, id: PaneId) -> bool {
        self.remove_pane(id, false)
    }

    pub(super) fn remove_pane(&mut self, id: PaneId, end: bool) -> bool {
        if let Some(pane) = self.panes.remove(&id) {
            if end {
                pane.end();
            } else {
                pane.shutdown();
            }
        }
        self.forget_ticks(id);
        if let Some(index) = self.tabs.iter().position(|t| t.contains(id)) {
            let tab = &mut self.tabs[index];
            if tab.root.remove(id) {
                if tab.focus == id {
                    tab.zoomed = false;
                    let mut ids = Vec::new();
                    tab.root.panes(&mut ids);
                    tab.focus = ids[0];
                }
            } else {
                let closed = self.tabs.remove(index);
                let was_active = index == self.active;
                if index < self.active {
                    self.active -= 1;
                }
                if was_active && let Some(home) = closed.back.and_then(|b| self.tabs.iter().position(|t| t.contains(b)))
                {
                    self.active = home;
                }
                self.active = self.active.min(self.tabs.len().saturating_sub(1));
            }
        }
        if self.tabs.is_empty() {
            layouts::forget(self.seq);
            return true;
        }
        self.relayout();
        self.update_title();
        false
    }

    pub fn shutdown(&mut self) {
        for pane in self.panes.values() {
            pane.shutdown();
        }
    }

    pub(super) fn focused_pane(&self) -> Option<&Pane> {
        self.tabs.get(self.active).and_then(|t| self.panes.get(&t.focus))
    }

    pub(super) fn focus(&mut self, id: PaneId) {
        if let Some(tab) = self.tabs.get_mut(self.active)
            && tab.focus != id
        {
            let old = tab.focus;
            tab.focus = id;
            let unzoomed = std::mem::take(&mut tab.zoomed);
            for (pane, on) in [(old, false), (id, true)] {
                if let Some(pane) = self.panes.get_mut(&pane) {
                    if pane.mode().contains(TermMode::FOCUS_IN_OUT) {
                        pane.write(if on { &b"\x1b[I"[..] } else { &b"\x1b[O"[..] });
                    }
                    pane.invalidate();
                }
            }
            if unzoomed {
                self.relayout();
            }
            self.update_title();
            self.window.request_redraw();
        }
    }
}

pub(super) fn first_session(shape: &Shape) -> String {
    match shape {
        Shape::Pane(id) => id.clone(),
        Shape::Split { first, .. } => first_session(first),
    }
}

/// The splits of `shape` with a new pane id for each session, which `sessions` gets in order.
pub(super) fn plan(shape: Shape, sessions: &mut Vec<(PaneId, String)>) -> Node {
    match shape {
        Shape::Pane(session) => {
            let id = crate::app::next_pane();
            sessions.push((id, session));
            Node::Pane(id)
        }
        Shape::Split {
            across,
            ratio,
            first,
            second,
        } => Node::Split {
            across,
            ratio,
            first: Box::new(plan(*first, sessions)),
            second: Box::new(plan(*second, sessions)),
        },
    }
}
