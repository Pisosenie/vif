//! Drawing a window's frame: every pane, the chrome around them and what shows over them.

use vif_config::Divider;
use vif_render::{Rect, Scene};
use winit::dpi::{PhysicalPosition, PhysicalSize};

use super::*;
use crate::colors::mix;
use crate::pane::Look;

impl Win {
    pub(super) fn invalidate_all(&mut self) {
        for pane in self.panes.values_mut() {
            pane.invalidate();
        }
        self.window.request_redraw();
    }

    pub(super) fn redraw(&mut self) {
        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                self.surface.configure(&self.device, &self.surface_config);
                self.window.request_redraw();
                return;
            }
            Err(err) => {
                log::warn!("no frame: {err}");
                return;
            }
        };
        let mut scene = std::mem::take(&mut self.scene);
        scene.clear();
        self.draw_wallpaper(&mut scene);
        self.compose(&mut scene);
        self.feed_effects();
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bg = self.scheme.bg();
        let a = f64::from(self.scheme.opacity);
        let clear = wgpu::Color {
            r: f64::from(bg[0]) * a,
            g: f64::from(bg[1]) * a,
            b: f64::from(bg[2]) * a,
            a,
        };
        self.renderer.render(
            &view,
            self.surface_config.width,
            self.surface_config.height,
            clear,
            &scene,
        );
        self.window.pre_present_notify();
        frame.present();
        self.hud_presented();
        self.scene = scene;
        if self.effects_playing() {
            self.window.request_redraw();
        }
        if !self.shown {
            self.shown = true;
            self.window.set_visible(true);
            self.wake_cursor();
            crate::app::mark("first frame");
        }
        if let Some((col, line)) = self
            .focused_pane()
            .and_then(|p| p.cursor_cell().map(|c| (p.area, c)))
            .map(|(a, (c, l))| {
                let m = self.renderer.metrics();
                (a.x + c as f32 * m.cell_width, a.y + (l + 1) as f32 * m.cell_height)
            })
        {
            self.window.set_ime_cursor_area(
                PhysicalPosition::new(col as f64, line as f64),
                PhysicalSize::new(1u32, 1u32),
            );
        }
    }

    pub(super) fn compose(&mut self, scene: &mut Scene) {
        let m = self.renderer.metrics();
        let focus = self.tabs.get(self.active).map(|t| t.focus);
        let link = self.link.as_ref().filter(|l| l.span.is_none()).map(|l| l.key.clone());
        let scheme = self.scheme;
        let cursor_on = self.cursor_on || !self.config.cursor.blink;
        let split = self.areas.len() > 1;
        let mut labels = Vec::new();
        for (id, area) in self.areas.clone() {
            let Some(pane) = self.panes.get_mut(&id) else {
                continue;
            };
            let rule = if self.config.rules.is_empty() {
                None
            } else {
                crate::rules::pick(&self.config.rules, &pane.location())
            };
            if rule != pane.rule {
                pane.rule = rule;
                pane.invalidate();
            }
            let own = rule.and_then(|i| self.rule_schemes.get(i).copied()).unwrap_or(scheme);
            if own.bg() != scheme.bg() {
                let o = pane.outer;
                let mut bg = own.bg();
                bg[3] = own.opacity;
                scene.back.push(Rect::fill(o.x, o.y, o.w, o.h, bg));
            }
            if let Some(label) = rule.and_then(|i| self.config.rules[i].label.clone()) {
                labels.push((area, label, own));
            }
            let focused = self.focused && Some(id) == focus;
            let look = Look {
                scheme: &own,
                focused,
                cursor_on,
                link: link.as_deref(),
                badges: &self.config.badges,
            };
            pane.draw(&mut self.renderer, &look, scene);
            if let Some(Link {
                pane: owner,
                span: Some((line, start, end)),
                target,
                ..
            }) = &self.link
                && *owner == id
            {
                let x = area.x + *start as f32 * m.cell_width;
                let y = area.y + *line as f32 * m.cell_height + m.underline;
                let w = (*end - *start + 1) as f32 * m.cell_width;
                let under = Rect::fill(x, y, w, m.stroke, scheme.accent());
                scene.front.push(match target {
                    Some(_) => under.kind(vif_render::RECT_DOTTED, m.stroke.max(1.0)),
                    None => under,
                });
            }
            if split && Some(id) != focus && self.config.panes.dim > 0.0 {
                let mut dim = scheme.bg();
                dim[3] = self.config.panes.dim;
                scene.front.push(Rect::fill(area.x, area.y, area.w, area.h, dim));
            }
        }
        if split && self.config.panes.divider == Divider::Line {
            let line = mix(scheme.fg(), scheme.bg(), 0.86);
            let gap = pad(&self.config, self.scale()).0.max(m.cell_width / 2.0).round();
            for (_, a) in &self.areas {
                let right = a.x + a.w;
                if right + gap < self.size().0 - 1.0 {
                    scene
                        .back
                        .push(Rect::fill((right + gap / 2.0).floor(), a.y, 1.0, a.h, line));
                }
                let bottom = a.y + a.h;
                if bottom + gap < self.size().1 - 1.0 {
                    scene
                        .back
                        .push(Rect::fill(a.x, (bottom + gap / 2.0).floor(), a.w, 1.0, line));
                }
            }
        }
        for (area, label, own) in labels {
            self.draw_label(&area, &label, &own, scene);
        }
        self.draw_tabs(scene);
        self.draw_preedit(scene);
        self.draw_scrollbars(scene);
        self.draw_search(scene);
        self.draw_block(scene);
        self.draw_hints(scene);
        self.draw_link_tip(scene);
        self.draw_palette(scene);
        self.draw_sessions(scene);
        self.draw_history(scene);
        self.draw_reply(scene);
        self.draw_notice(scene);
        self.draw_toast(scene);
        self.draw_hud(scene);
    }
}
