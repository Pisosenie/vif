//! The bar along the window's bottom that says what keys do in a mode.

use vif_render::{Rect, Scene, Style};

use super::Win;
use crate::colors::mix;

impl Win {
    /// `title` in the accent, `note` after it, then each (keys, what) with the keys in little boxes.
    /// It sits at the bottom, or under the tabs when something in `keep` (top and bottom y) would
    /// be under it there.
    pub(super) fn draw_footer(
        &mut self,
        title: &str,
        note: &str,
        items: &[(&str, &str)],
        keep: &[(f32, f32)],
        scene: &mut Scene,
    ) {
        let m = self.renderer.metrics();
        let scale = self.scale();
        let scheme = self.scheme;
        let (w, h) = self.size();
        let (cw, ch) = (m.cell_width, m.cell_height);
        let bar = (44.0 * scale).max(ch + 16.0 * scale).round();
        let covered = keep.iter().any(|(top, bottom)| *bottom > h - bar && *top < h);
        let y = if covered { self.content().y.round() } else { h - bar };
        scene
            .front
            .push(Rect::fill(0.0, y, w, bar, mix(scheme.fg(), scheme.bg(), 0.93)));
        scene.front.push(Rect::fill(
            0.0,
            y,
            w,
            scale.max(1.0),
            mix(scheme.fg(), scheme.bg(), 0.82),
        ));
        let muted = mix(scheme.fg(), scheme.bg(), 0.42);
        let ty = (y + (bar - ch) / 2.0).round();
        let mut text = Scene::default();
        let mut x = (16.0 * scale).round();
        x += self.label(title, x, ty, scheme.accent(), Style::Bold, &mut text) + 2.0 * cw;
        if !note.is_empty() {
            x += self.label(note, x, ty, muted, Style::Regular, &mut text) + 2.0 * cw;
        }
        for (keys, what) in items {
            let width = keys.chars().count() as f32 * cw + 10.0 * scale;
            let needed = width + 6.0 * scale + what.chars().count() as f32 * cw;
            if x + needed > w - 16.0 * scale {
                break;
            }
            let cap = Rect::fill(
                x,
                ty - 3.0 * scale,
                width,
                ch + 6.0 * scale,
                mix(scheme.fg(), scheme.bg(), 0.8),
            );
            scene.front.push(cap.rounded(self.radius() * 0.5));
            self.label(keys, x + 5.0 * scale, ty, scheme.fg(), Style::Regular, &mut text);
            x += width + 6.0 * scale;
            x += self.label(what, x, ty, muted, Style::Regular, &mut text) + 2.0 * cw;
        }
        scene.over.extend(text.mask);
    }
}
