//! The HUD: how long a key press takes to reach a frame, as a line of bars and a number, and the
//! screen's refresh rate. Off unless asked for; it measures nothing while off.

use std::collections::VecDeque;
use std::time::Instant;

use vif_render::{Rect, Scene, Style};

use super::Win;
use crate::colors::mix;

const KEPT: usize = 40;

#[derive(Default)]
pub(super) struct Hud {
    /// When the key that the next frame answers was pressed.
    pressed: Option<Instant>,
    /// Key to frame, in milliseconds, newest last.
    took: VecDeque<f32>,
}

impl Win {
    pub(super) fn toggle_hud(&mut self) {
        self.hud = match self.hud {
            Some(_) => None,
            None => Some(Hud::default()),
        };
        self.window.request_redraw();
    }

    pub(super) fn hud_pressed(&mut self) {
        if let Some(hud) = &mut self.hud {
            hud.pressed.get_or_insert_with(Instant::now);
        }
    }

    pub(super) fn hud_presented(&mut self) {
        if let Some(hud) = &mut self.hud
            && let Some(at) = hud.pressed.take()
        {
            hud.took.push_back(at.elapsed().as_secs_f32() * 1000.0);
            if hud.took.len() > KEPT {
                hud.took.pop_front();
            }
        }
    }

    pub(super) fn draw_hud(&mut self, scene: &mut Scene) {
        let Some(hud) = &self.hud else {
            return;
        };
        let took: Vec<f32> = hud.took.iter().copied().collect();
        let m = self.renderer.metrics();
        let scale = self.scale();
        let scheme = self.scheme;
        let (w, _) = self.size();
        let (cw, ch) = (m.cell_width, m.cell_height);
        let median = {
            let mut sorted = took.clone();
            sorted.sort_by(f32::total_cmp);
            sorted.get(sorted.len() / 2).copied()
        };
        let hz = self
            .window
            .current_monitor()
            .and_then(|m| m.refresh_rate_millihertz())
            .filter(|mhz| *mhz > 0)
            .map_or_else(String::new, |mhz| format!("{} Hz", (mhz as f32 / 1000.0).round()));
        let latency = median.map_or_else(|| "press a key".to_owned(), |ms| format!("{ms:.1} ms key→frame"));
        let label = format!("{latency}  {hz}");
        let bars = KEPT as f32 * 2.0 * scale;
        let pad = 8.0 * scale;
        let pw = bars + pad * 3.0 + label.chars().count() as f32 * cw;
        let ph = ch + 8.0 * scale;
        let (x, y) = (
            (w - pw - 10.0 * scale).round(),
            (self.bands().0 / 2.0 - ph / 2.0).max(4.0 * scale).round(),
        );
        scene
            .front
            .push(Rect::fill(x, y, pw, ph, mix(scheme.fg(), scheme.bg(), 0.88)).rounded(ph / 2.0));
        let top = took.iter().copied().fold(1.0f32, f32::max);
        for (i, ms) in took.iter().enumerate() {
            let bh = ((ms / top) * (ph - 8.0 * scale)).max(scale);
            let bx = x + pad + i as f32 * 2.0 * scale;
            scene.front.push(Rect::fill(
                bx,
                y + ph - 4.0 * scale - bh,
                scale.max(1.0),
                bh,
                scheme.accent(),
            ));
        }
        let mut text = Scene::default();
        let muted = mix(scheme.fg(), scheme.bg(), 0.35);
        self.label(
            &label,
            x + pad * 2.0 + bars,
            y + 4.0 * scale,
            muted,
            Style::Regular,
            &mut text,
        );
        scene.over.extend(text.mask);
    }
}
