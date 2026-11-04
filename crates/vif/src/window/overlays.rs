//! What floats over the panes: find, hints, the config notice and IME text.

use std::time::Instant;

use vif_render::{LineCell, Rect, Scene, Style};

use super::{Answer, Win};
use crate::colors::{Rgba, Scheme, mix, rgba};
use crate::hints::Target;
use crate::layout::Area;

impl Win {
    /// A rule's label in the pane's top right corner, like "PROD".
    pub(super) fn draw_label(&mut self, area: &Area, label: &str, scheme: &Scheme, scene: &mut Scene) {
        let m = self.renderer.metrics();
        let scale = self.scale();
        let w = (label.chars().count() as f32 + 2.0) * m.cell_width;
        let h = m.cell_height + 4.0 * scale;
        let (x, y) = ((area.x + area.w - w).round(), area.y.round());
        scene
            .front
            .push(Rect::fill(x, y, w, h, scheme.accent()).rounded(self.radius() * 0.6));
        let mut text = Scene::default();
        let ty = (y + (h - m.cell_height) / 2.0).round();
        self.label(label, x + m.cell_width, ty, scheme.bg(), Style::Bold, &mut text);
        scene.over.extend(text.mask);
    }

    pub(super) fn label(&mut self, text: &str, x: f32, y: f32, color: Rgba, style: Style, scene: &mut Scene) -> f32 {
        let chars: Vec<String> = text.chars().map(String::from).collect();
        let cells: Vec<LineCell> = chars
            .iter()
            .map(|t| LineCell {
                text: t,
                style,
                color,
                wide: false,
            })
            .collect();
        self.renderer.draw_line(&cells, x, y, scene);
        cells.len() as f32 * self.renderer.metrics().cell_width
    }

    pub(super) fn draw_preedit(&mut self, scene: &mut Scene) {
        if self.preedit.is_empty() {
            return;
        }
        let m = self.renderer.metrics();
        let Some((area, (col, line))) = self.focused_pane().and_then(|p| p.cursor_cell().map(|c| (p.area, c))) else {
            return;
        };
        let x = area.x + col as f32 * m.cell_width;
        let y = area.y + line as f32 * m.cell_height;
        let text = self.preedit.clone();
        let w = text.chars().count() as f32 * m.cell_width;
        let scheme = self.scheme;
        scene.front.push(Rect::fill(x, y, w, m.cell_height, scheme.bg()));
        scene
            .front
            .push(Rect::fill(x, y + m.underline, w, m.stroke, scheme.accent()));
        let mut overlay = Scene::default();
        self.label(&text, x, y, scheme.fg(), Style::Regular, &mut overlay);
        scene.over.extend(overlay.mask);
    }

    pub(super) fn draw_search(&mut self, scene: &mut Scene) {
        let Some(pane) = self.focused_pane() else {
            return;
        };
        let Some(search) = &pane.search else {
            return;
        };
        let (query, missed, area) = (search.query.clone(), search.missed, pane.area);
        let m = self.renderer.metrics();
        let scale = self.scale();
        let scheme = self.scheme;
        let hint = if missed {
            "no match"
        } else if query.is_empty() {
            "type to find"
        } else {
            "↵ older  ⇧↵ newer"
        };
        let field = query.chars().count().max(18) as f32;
        let pad = 12.0 * scale;
        let w = ((5.0 + field + 2.0 + hint.chars().count() as f32) * m.cell_width + 2.0 * pad).min(area.w);
        let h = m.cell_height + 12.0 * scale;
        let (x, y) = ((area.x + area.w - w).round(), area.y.round());
        let fill = mix(scheme.fg(), scheme.bg(), 0.88);
        scene.front.push(Rect::fill(x, y, w, h, fill).rounded(self.radius()));
        let ty = (y + (h - m.cell_height) / 2.0).round();
        let mut text = Scene::default();
        self.label(
            "Find",
            x + pad,
            ty,
            mix(scheme.fg(), scheme.bg(), 0.45),
            Style::Regular,
            &mut text,
        );
        let qx = x + pad + 5.0 * m.cell_width;
        let typed = self.label(&query, qx, ty, scheme.fg(), Style::Regular, &mut text);
        scene.front.push(Rect::fill(
            qx + typed,
            ty,
            (2.0 * scale).round(),
            m.cell_height,
            scheme.accent(),
        ));
        let hint_color = if missed {
            rgba(vif_config::Rgb::hex(0xf2b35c), 1.0)
        } else {
            mix(scheme.fg(), scheme.bg(), 0.45)
        };
        let hx = x + w - pad - hint.chars().count() as f32 * m.cell_width;
        self.label(hint, hx, ty, hint_color, Style::Regular, &mut text);
        scene.over.extend(text.mask);
    }

    pub(super) fn draw_hints(&mut self, scene: &mut Scene) {
        let Some(pane) = self.focused_pane() else {
            return;
        };
        let Some(hints) = &pane.hints else {
            return;
        };
        let targets: Vec<Target> = hints.visible().cloned().collect();
        let (typed, total, area, outer) = (hints.typed.chars().count(), hints.targets.len(), pane.area, pane.outer);
        let m = self.renderer.metrics();
        let scale = self.scale();
        let scheme = self.scheme;
        let mut dim = scheme.bg();
        dim[3] = 0.62;
        scene.front.push(Rect::fill(outer.x, outer.y, outer.w, outer.h, dim));
        let (cw, ch) = (m.cell_width, m.cell_height);
        let cell = |line: usize, col: usize| (area.x + col as f32 * cw, area.y + line as f32 * ch);
        let lit = mix(scheme.fg(), scheme.bg(), 0.84);
        let mut text = Scene::default();
        let mut chips = Scene::default();
        let mut keep = Vec::new();
        for t in &targets {
            let rest: String = t.label.chars().skip(typed).collect();
            for cells in &t.at {
                for &(line, col, c) in cells {
                    let (x, y) = cell(line, col);
                    scene.front.push(Rect::fill(x, y, cw, ch, lit));
                    self.label(&c.to_string(), x, y, scheme.fg(), Style::Regular, &mut text);
                    keep.push((y - 0.62 * ch, y + ch));
                }
                // The label floats over the target's top-left corner, so all of it stays readable.
                let (l0, c0, _) = cells[0];
                let (x, y) = cell(l0, c0);
                let (w, h) = (rest.chars().count() as f32 * cw + 6.0 * scale, ch);
                let (cx, cy) = ((x - 6.0 * scale).max(outer.x).round(), (y - 0.62 * ch).round());
                let mut shadow = [0.0, 0.0, 0.0, 0.35];
                shadow[3] *= scheme.opacity.max(0.5);
                chips
                    .front
                    .push(Rect::fill(cx + scale, cy + 2.0 * scale, w, h, shadow).rounded(self.radius() * 0.5));
                chips
                    .front
                    .push(Rect::fill(cx, cy, w, h, scheme.accent()).rounded(self.radius() * 0.5));
                self.label(&rest, cx + 3.0 * scale, cy, scheme.bg(), Style::Bold, &mut chips);
            }
        }
        scene.over.extend(text.mask);
        scene.front.extend(chips.front);
        scene.over.extend(chips.mask);
        let mac = cfg!(target_os = "macos");
        let next = targets
            .first()
            .map(|t| (t.label.chars().skip(typed).collect::<String>(), t.describe()));
        let mut items: Vec<(&str, &str)> = Vec::new();
        if let Some((label, what)) = &next {
            items.push((label, what));
        }
        items.extend([
            ("letter", "open"),
            (if mac { "⇧" } else { "Shift" }, "copy"),
            (if mac { "⌥" } else { "Alt" }, "paste into prompt"),
            ("Esc", "back"),
        ]);
        let note = if total == 0 {
            "nothing to pick".to_owned()
        } else {
            format!("{total} on screen")
        };
        self.draw_footer("Hints", &note, &items, &keep, scene);
    }

    /// What a modifier-click on the path under the mouse would do.
    pub(super) fn draw_link_tip(&mut self, scene: &mut Scene) {
        let Some(super::Link {
            pane,
            span: Some((line, start, _)),
            target: Some(target),
            ..
        }) = self.link.clone()
        else {
            return;
        };
        let Some(area) = self.panes.get(&pane).map(|p| p.area) else {
            return;
        };
        let m = self.renderer.metrics();
        let scale = self.scale();
        let (cw, ch) = (m.cell_width, m.cell_height);
        let mut what = target.describe();
        if let Some(first) = what.get(..1) {
            what = first.to_uppercase() + &what[1..];
        }
        let keys = if cfg!(target_os = "macos") {
            "⌘ click"
        } else {
            "Ctrl click"
        };
        let pad = 10.0 * scale;
        let w = (what.chars().count() + keys.chars().count() + 2) as f32 * cw + 2.0 * pad;
        let h = ch + 12.0 * scale;
        let x = (area.x + start as f32 * cw).min(area.x + area.w - w).max(area.x);
        let above = area.y + line as f32 * ch - h - 4.0 * scale;
        let y = if above >= area.y {
            above
        } else {
            area.y + (line + 1) as f32 * ch + 4.0 * scale
        };
        let fill = mix(self.scheme.fg(), self.scheme.bg(), 0.88);
        scene.front.push(Rect::fill(x, y, w, h, fill).rounded(self.radius()));
        let mut text = Scene::default();
        let ty = (y + (h - ch) / 2.0).round();
        let tw = self.label(&what, x + pad, ty, self.scheme.fg(), Style::Regular, &mut text);
        let muted = mix(self.scheme.fg(), self.scheme.bg(), 0.45);
        self.label(keys, x + pad + tw + 2.0 * cw, ty, muted, Style::Regular, &mut text);
        scene.over.extend(text.mask);
    }

    /// Says the config was read again and how long it took, unless it has problems to show.
    pub fn reloaded(&mut self, took: std::time::Duration) {
        if self.notice.is_none() {
            self.flash(&format!("Reloaded vif.toml · {} ms", took.as_millis().max(1)));
        }
    }

    /// Something just done, said for a moment.
    pub(super) fn flash(&mut self, what: &str) {
        self.toast = Some((what.to_owned(), Instant::now() + std::time::Duration::from_millis(1400)));
        self.window.request_redraw();
    }

    pub(super) fn draw_toast(&mut self, scene: &mut Scene) {
        let Some((what, _)) = self.toast.clone() else {
            return;
        };
        let m = self.renderer.metrics();
        let scale = self.scale();
        let (w, h) = self.size();
        let pad = 14.0 * scale;
        let tw = what.chars().count() as f32 * m.cell_width + 2.0 * pad;
        let th = m.cell_height + 14.0 * scale;
        let lifted = if self.block.is_some() { 56.0 } else { 16.0 } * scale;
        let (x, y) = (((w - tw) / 2.0).round(), (h - th - lifted).round());
        let fill = mix(self.scheme.fg(), self.scheme.bg(), 0.86);
        scene.front.push(Rect::fill(x, y, tw, th, fill).rounded(self.radius()));
        let mut text = Scene::default();
        let ty = (y + (th - m.cell_height) / 2.0).round();
        self.label(&what, x + pad, ty, self.scheme.fg(), Style::Regular, &mut text);
        scene.over.extend(text.mask);
    }

    pub(super) fn draw_notice(&mut self, scene: &mut Scene) {
        let Some(notice) = &self.notice else {
            return;
        };
        let lines = notice.lines.clone();
        let buttons: Vec<Answer> = notice.buttons.iter().map(|(b, _)| *b).collect();
        let m = self.renderer.metrics();
        let scale = self.scale();
        let content = self.content();
        let width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as f32 * m.cell_width + 28.0 * scale;
        let w = width.min(content.w);
        let row = if buttons.is_empty() {
            0.0
        } else {
            m.cell_height + 16.0 * scale
        };
        let h = lines.len() as f32 * m.cell_height + 20.0 * scale + row;
        let x = content.x + content.w - w;
        let y = content.y;
        let warn = rgba(vif_config::Rgb::hex(0xf2b35c), 1.0);
        let fill = mix(self.scheme.bg(), warn, 0.14);
        let mut panel = Scene::default();
        panel.front.push(Rect::fill(x, y, w, h, fill).rounded(self.radius()));
        let mut ty = y + 10.0 * scale;
        for (i, line) in lines.iter().enumerate() {
            let max = ((w - 28.0 * scale) / m.cell_width) as usize;
            let line: String = line.chars().take(max).collect();
            let color = if i == 0 { warn } else { self.scheme.fg() };
            let style = if i == 0 { Style::Bold } else { Style::Regular };
            self.label(&line, x + 14.0 * scale, ty, color, style, &mut panel);
            ty += m.cell_height;
        }
        // Fix typo and Open in settings from the left, × at the right.
        let mut drawn = Vec::new();
        let (pad, gap) = (8.0 * scale, 8.0 * scale);
        let by = ty + 6.0 * scale;
        let mut bx = x + 14.0 * scale - pad;
        for button in buttons {
            let text = match button {
                Answer::Fix => "Fix typo",
                Answer::Settings => "Open in settings",
                Answer::Close => "×",
            };
            let bw = text.chars().count() as f32 * m.cell_width + 2.0 * pad;
            let at = if button == Answer::Close {
                x + w - 14.0 * scale - bw + pad
            } else {
                bx
            };
            let area = Area {
                x: at,
                y: by - 3.0 * scale,
                w: bw,
                h: m.cell_height + 6.0 * scale,
            };
            if button != Answer::Close {
                let chip = mix(self.scheme.bg(), warn, 0.32);
                panel
                    .front
                    .push(Rect::fill(area.x, area.y, area.w, area.h, chip).rounded(4.0 * scale));
                bx += bw + gap;
            }
            let color = if button == Answer::Close {
                self.scheme.fg()
            } else {
                warn
            };
            self.label(text, at + pad, by, color, Style::Bold, &mut panel);
            drawn.push((button, area));
        }
        scene.front.extend(panel.front);
        scene.over.extend(panel.mask);
        if let Some(n) = &mut self.notice {
            n.area = Area { x, y, w, h };
            if !drawn.is_empty() {
                n.buttons = drawn;
            }
        }
    }
}
