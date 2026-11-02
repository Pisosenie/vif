//! Block mode: one command and what it printed picked out, with what can be done to it.

use alacritty_terminal::grid::Scroll;
use vif_render::{RECT_HOLLOW, Rect, Scene, Style};
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, NamedKey};

use super::{Shared, Win};
use crate::colors::mix;
use crate::fold::Shown;
use crate::layout::{Area, PaneId};

pub(super) struct BlockMode {
    pane: PaneId,
    id: u32,
    /// Where the toolbar's buttons were drawn, for clicks.
    buttons: Vec<(Act, Area)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Act {
    Output,
    Command,
    Rerun,
    Fold,
    Pager,
    Notify,
}

impl Win {
    /// Picks the block before (or after) the one picked, starting from the newest; going past the
    /// newest leaves block mode.
    pub(super) fn block_step(&mut self, older: bool) {
        let Some(id) = self.tabs.get(self.active).map(|t| t.focus) else {
            return;
        };
        let Some(pane) = self.panes.get_mut(&id) else {
            return;
        };
        let ran = pane.ran();
        let picked = self.block.as_ref().filter(|b| b.pane == id).map(|b| b.id);
        let at = picked.and_then(|p| ran.iter().position(|r| *r == p));
        let next = match (at, older) {
            (None, true) => ran.last().copied(),
            (None, false) => None,
            (Some(i), true) => Some(ran[i.saturating_sub(1)]),
            (Some(i), false) => ran.get(i + 1).copied(),
        };
        match next {
            Some(block) => {
                if let Some((top, bottom)) = pane.span(block) {
                    pane.reveal(top, bottom);
                }
                self.block = Some(BlockMode {
                    pane: id,
                    id: block,
                    buttons: Vec::new(),
                });
            }
            None => self.leave_blocks(),
        }
        self.window.request_redraw();
    }

    pub(super) fn leave_blocks(&mut self) {
        if let Some(mode) = self.block.take()
            && let Some(pane) = self.panes.get_mut(&mode.pane)
        {
            pane.scroll(Scroll::Bottom);
        }
        self.window.request_redraw();
    }

    /// Keys while a block is picked; everything else waits until Esc.
    pub(super) fn block_key(&mut self, event: &KeyEvent, shared: &Shared) {
        if event.state != ElementState::Pressed {
            return;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Escape | NamedKey::Enter) => self.leave_blocks(),
            Key::Named(NamedKey::ArrowUp) => self.block_step(true),
            Key::Named(NamedKey::ArrowDown) => self.block_step(false),
            Key::Character(c) => match c.as_str() {
                "k" => self.block_step(true),
                "j" => self.block_step(false),
                "c" => self.block_act(Act::Output, shared),
                "C" => self.block_act(Act::Command, shared),
                "r" => self.block_act(Act::Rerun, shared),
                "f" => self.block_act(Act::Fold, shared),
                "p" => self.block_act(Act::Pager, shared),
                "n" => self.block_act(Act::Notify, shared),
                "/" => {
                    self.block = None;
                    if let Some(pane) = self.tabs.get(self.active).and_then(|t| self.panes.get_mut(&t.focus)) {
                        pane.search_open();
                    }
                    self.window.request_redraw();
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn block_act(&mut self, act: Act, shared: &Shared) {
        let Some((pane_id, id)) = self.block.as_ref().map(|b| (b.pane, b.id)) else {
            return;
        };
        let Some(pane) = self.panes.get_mut(&pane_id) else {
            return;
        };
        match act {
            Act::Output => {
                if let Some(text) = pane.output(id) {
                    self.clipboard.set(text);
                    self.flash("Output copied");
                }
            }
            Act::Command => {
                if let Some(text) = pane.command(id) {
                    self.clipboard.set(text.trim().to_owned());
                    self.flash("Command copied");
                }
            }
            Act::Rerun => {
                if pane.busy() {
                    return self.flash("A command is still running");
                }
                if let Some(command) = pane.command(id) {
                    let mut bytes = command.trim().as_bytes().to_vec();
                    bytes.push(b'\r');
                    pane.input(bytes);
                    self.block = None;
                }
            }
            Act::Fold => {
                if !pane.blocks.get(id).is_some_and(|b| b.ended.is_none()) {
                    pane.toggle_fold(id);
                    pane.invalidate();
                }
            }
            Act::Pager => {
                let command = pane.command(id).unwrap_or_default();
                let output = pane.output(id).unwrap_or_default();
                let text = format!("$ {}\n\n{output}\n", command.trim());
                self.block = None;
                self.page(shared, &format!("block-{id}"), &text, None);
            }
            Act::Notify => {
                if !self.notify.remove(&(pane_id, id)) {
                    self.notify.insert((pane_id, id));
                }
            }
        }
        self.window.request_redraw();
    }

    /// Clicks on the toolbar; true when one landed there.
    pub(super) fn block_click(&mut self, x: f32, y: f32, shared: &Shared) -> bool {
        let hit = self
            .block
            .as_ref()
            .and_then(|b| b.buttons.iter().find(|(_, a)| a.contains(x, y)).map(|(act, _)| *act));
        match hit {
            Some(act) => {
                self.block_act(act, shared);
                true
            }
            None => false,
        }
    }

    pub(super) fn draw_block(&mut self, scene: &mut Scene) {
        let Some((pane_id, id)) = self.block.as_ref().map(|b| (b.pane, b.id)) else {
            return;
        };
        if self.tabs.get(self.active).is_none_or(|t| !t.contains(pane_id)) {
            return;
        }
        let Some(pane) = self.panes.get(&pane_id) else {
            return;
        };
        let Some((top, bottom)) = pane.span(id) else {
            return;
        };
        let running = pane.blocks.get(id).is_some_and(|b| b.ended.is_none());
        let ours = |line: usize| match pane.shows(line) {
            Shown::Line(l) => (top..=bottom).contains(&l),
            Shown::Folded { id: folded, .. } => folded == id,
            Shown::Blank => false,
        };
        let shown: Vec<usize> = (0..pane.lines).filter(|&l| ours(l)).collect();
        let (Some(&first), Some(&last)) = (shown.first(), shown.last()) else {
            return;
        };
        let m = self.renderer.metrics();
        let scale = self.scale();
        let scheme = self.scheme;
        let (cw, ch) = (m.cell_width, m.cell_height);
        let area = pane.area;
        let pad = (cw * 0.6).round();
        let (x, y) = (area.x - pad, area.y + first as f32 * ch - 2.0 * scale);
        let (w, h) = (area.w + 2.0 * pad, (last + 1 - first) as f32 * ch + 4.0 * scale);
        let mut tint = scheme.accent();
        tint[3] = 0.08;
        scene.back.push(Rect::fill(x, y, w, h, tint).rounded(self.radius()));
        let ring = Rect::fill(x, y, w, h, scheme.accent()).kind(RECT_HOLLOW, 1.5 * scale);
        scene.front.push(ring.rounded(self.radius()));

        let notify = self.notify.contains(&(pane_id, id));
        let acts: Vec<(Act, String, &str)> = if running {
            let label = if notify {
                "✓ Notify when done"
            } else {
                "Notify when done"
            };
            vec![
                (Act::Output, "Copy output".into(), "c"),
                (Act::Notify, label.into(), "n"),
            ]
        } else {
            let fold = if pane.folded(id) { "Unfold" } else { "Fold" };
            vec![
                (Act::Output, "Copy output".into(), "c"),
                (Act::Command, "Command".into(), "C"),
                (Act::Rerun, "Rerun".into(), "r"),
                (Act::Fold, fold.into(), "f"),
                (Act::Pager, "Pager".into(), "p"),
            ]
        };
        let gap = 6.0 * scale;
        let inner = 8.0 * scale;
        let widths: Vec<f32> = acts
            .iter()
            .map(|(_, label, key)| (label.chars().count() + key.chars().count() + 1) as f32 * cw + 2.0 * inner)
            .collect();
        let total = widths.iter().sum::<f32>() + gap * (acts.len() as f32 + 1.0);
        let bh = ch + 10.0 * scale;
        let bx = (x + w - total - 6.0 * scale).max(area.x);
        let by = (y - bh / 2.0).max(area.y - 4.0 * scale).round();
        let panel = mix(scheme.fg(), scheme.bg(), 0.9);
        scene
            .front
            .push(Rect::fill(bx, by, total, bh, panel).rounded(self.radius()));
        let mut text = Scene::default();
        let mut buttons = Vec::new();
        let mut cx = bx + gap;
        let muted = mix(scheme.fg(), scheme.bg(), 0.45);
        for ((act, label, key), width) in acts.iter().zip(widths) {
            let ty = (by + (bh - ch) / 2.0).round();
            let lw = self.label(label, cx + inner, ty, scheme.fg(), Style::Regular, &mut text);
            self.label(key, cx + inner + lw + cw, ty, muted, Style::Regular, &mut text);
            buttons.push((
                *act,
                Area {
                    x: cx,
                    y: by,
                    w: width,
                    h: bh,
                },
            ));
            cx += width + gap;
        }
        scene.over.extend(text.mask);
        if let Some(mode) = &mut self.block {
            mode.buttons = buttons;
        }
        let items: &[(&str, &str)] = &[
            ("↑↓", "move"),
            ("c", "copy output"),
            ("C", "copy command"),
            ("r", "rerun"),
            ("f", "fold"),
            ("p", "pager"),
            ("/", "search"),
            ("Esc", "back to prompt"),
        ];
        self.draw_footer("Block mode", "", items, &[(y + h - ch, y + h)], scene);
    }
}
