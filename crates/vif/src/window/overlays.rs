//! What floats over the panes: find, hints, the config notice and IME text.

use vif_render::{LineCell, Rect, Scene, Style};

use super::Win;
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
        let lit = mix(scheme.fg(), scheme.bg(), 0.86);
        let mut text = Scene::default();
        for t in &targets {
            let rest: Vec<String> = t.label.chars().skip(typed).map(String::from).collect();
            for cells in &t.at {
                for &(line, col, _) in cells {
                    let (x, y) = cell(line, col);
                    scene.front.push(Rect::fill(x, y, cw, ch, lit));
                }
                let (l0, c0, _) = cells[0];
                let (x, y) = cell(l0, c0);
                let badge = rest.len() as f32 * cw;
                scene
                    .front
                    .push(Rect::fill(x, y, badge, ch, scheme.accent()).rounded(self.radius() * 0.4));
                let label: Vec<LineCell> = rest
                    .iter()
                    .map(|c| LineCell {
                        text: c,
                        style: Style::Bold,
                        color: scheme.bg(),
                        wide: false,
                    })
                    .collect();
                self.renderer.draw_line(&label, x, y, &mut text);
                let shown: Vec<(usize, usize, String)> = cells
                    .iter()
                    .skip(rest.len())
                    .map(|&(l, c, ch)| (l, c, ch.to_string()))
                    .collect();
                for (l, c, s) in &shown {
                    let (x, y) = cell(*l, *c);
                    let one = [LineCell {
                        text: s,
                        style: Style::Regular,
                        color: scheme.fg(),
                        wide: false,
                    }];
                    self.renderer.draw_line(&one, x, y, &mut text);
                }
            }
        }
        let info = match (total, cfg!(target_os = "macos")) {
            (0, _) => "nothing to pick   esc".to_owned(),
            (n, true) => format!("{n} on screen   ⇧ copy   ⌥ paste   esc"),
            (n, false) => format!("{n} on screen   shift copy   alt paste   esc"),
        };
        let pad = 12.0 * scale;
        let w = ((6.0 + info.chars().count() as f32) * cw + 2.0 * pad).min(area.w);
        let h = ch + 12.0 * scale;
        let (x, y) = ((area.x + area.w - w).round(), (area.y + area.h - h).round());
        scene
            .front
            .push(Rect::fill(x, y, w, h, mix(scheme.fg(), scheme.bg(), 0.88)).rounded(self.radius()));
        let ty = (y + (h - ch) / 2.0).round();
        self.label("Hints", x + pad, ty, scheme.accent(), Style::Bold, &mut text);
        let muted = mix(scheme.fg(), scheme.bg(), 0.4);
        self.label(&info, x + pad + 6.0 * cw, ty, muted, Style::Regular, &mut text);
        scene.over.extend(text.mask);
    }

    pub(super) fn draw_notice(&mut self, scene: &mut Scene) {
        let Some(notice) = &self.notice else {
            return;
        };
        let lines = notice.lines.clone();
        let m = self.renderer.metrics();
        let scale = self.scale();
        let content = self.content();
        let width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as f32 * m.cell_width + 28.0 * scale;
        let w = width.min(content.w);
        let h = lines.len() as f32 * m.cell_height + 20.0 * scale;
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
        scene.front.extend(panel.front);
        scene.over.extend(panel.mask);
        if let Some(n) = &mut self.notice {
            n.area = Area { x, y, w, h };
        }
    }
}
