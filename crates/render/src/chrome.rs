//! The tab bar, drawn the same in the terminal and in the settings preview.

use crate::{LineCell, Rect, Renderer, Scene, Style};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarStyle {
    Pills,
    Underline,
    Powerline,
    Plain,
}

/// One tab: its name in bold, a quieter second part, and a dot that says how it is.
pub struct Tab {
    pub title: String,
    pub detail: String,
    pub dot: Option<[f32; 4]>,
}

#[derive(Clone, Copy)]
pub struct Bar<'a> {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub tabs: &'a [Tab],
    /// A "+" for a new tab after the last one.
    pub plus: bool,
    pub active: usize,
    pub style: BarStyle,
    pub fg: [f32; 4],
    pub bg: [f32; 4],
    pub accent: [f32; 4],
    pub radius: f32,
    pub scale: f32,
}

/// Where a tab sits, for clicks: x, y, width, height.
pub type Hit = (f32, f32, f32, f32);

impl Bar<'_> {
    fn muted(&self, t: f32) -> [f32; 4] {
        let mix = |i: usize| self.fg[i] + (self.bg[i] - self.fg[i]) * t;
        [mix(0), mix(1), mix(2), 1.0]
    }
}

/// Draws the bar into `scene` and returns each tab's place, and the "+" button's.
pub fn tab_bar(r: &mut Renderer, bar: &Bar, scene: &mut Scene) -> (Vec<Hit>, Option<Hit>) {
    if bar.tabs.is_empty() {
        return (Vec::new(), None);
    }
    let m = r.metrics();
    let side = if bar.plus {
        (m.cell_height + 8.0 * bar.scale).round()
    } else {
        0.0
    };
    let tabs = Bar {
        w: bar.w - side - if bar.plus { 4.0 * bar.scale } else { 0.0 },
        ..*bar
    };
    let hits = match bar.style {
        BarStyle::Pills | BarStyle::Underline => even(r, &tabs, scene),
        BarStyle::Powerline => powerline(r, &tabs, scene),
        BarStyle::Plain => plain(r, &tabs, scene),
    };
    if !bar.plus {
        return (hits, None);
    }
    let end = hits.last().map_or(bar.x, |(x, _, w, _)| x + w) + 4.0 * bar.scale;
    let x = end.min(bar.x + bar.w - side);
    let y = (bar.y + (bar.h - side) / 2.0).round();
    if bar.style == BarStyle::Pills {
        scene
            .back
            .push(Rect::fill(x, y, side, side, bar.muted(0.95)).rounded(bar.radius * bar.scale));
    }
    let ty = (bar.y + (bar.h - m.cell_height) / 2.0).round();
    label(
        r,
        "+",
        x + (side - m.cell_width) / 2.0,
        ty,
        bar.muted(0.4),
        Style::Regular,
        scene,
    );
    (hits, Some((x, bar.y, side, bar.h)))
}

/// Draws a tab's dot, title and detail from (x, y) in at most `max` cells; returns its width.
#[allow(clippy::too_many_arguments)]
fn tab_text(
    r: &mut Renderer,
    tab: &Tab,
    x: f32,
    y: f32,
    max: usize,
    title: [f32; 4],
    detail: [f32; 4],
    scene: &mut Scene,
) -> f32 {
    let m = r.metrics();
    let (cw, ch) = (m.cell_width, m.cell_height);
    let mut at = x;
    let mut room = max;
    if let Some(dot) = tab.dot.filter(|_| room > 4) {
        let d = (ch * 0.36).round();
        scene
            .back
            .push(Rect::fill(at + (cw - d) / 2.0, y + (ch - d) / 2.0, d, d, dot).rounded(d / 2.0));
        at += 2.0 * cw;
        room -= 2;
    }
    let name = fit(&tab.title, room);
    at += label(r, &name, at, y, title, Style::Bold, scene);
    room = room.saturating_sub(name.chars().count() + 1);
    if !tab.detail.is_empty() && room > 3 {
        at += cw;
        at += label(r, &fit(&tab.detail, room), at, y, detail, Style::Regular, scene);
    }
    at - x
}

/// How many cells a tab's text takes in at most `max`.
fn tab_width(tab: &Tab, max: usize) -> usize {
    let dot = if tab.dot.is_some() && max > 4 { 2 } else { 0 };
    let title = tab.title.chars().count().min(max - dot);
    let room = max.saturating_sub(dot + title + 1);
    let detail = if tab.detail.is_empty() || room <= 3 {
        0
    } else {
        tab.detail.chars().count().min(room) + 1
    };
    dot + title + detail
}

/// Tabs of one width across the bar, as pills or under a line.
fn even(r: &mut Renderer, bar: &Bar, scene: &mut Scene) -> Vec<Hit> {
    let m = r.metrics();
    let s = bar.scale;
    let n = bar.tabs.len() as f32;
    let gap = 4.0 * s;
    let tab_w = ((bar.w - gap * (n - 1.0)) / n).min(240.0 * s).floor();
    let pills = bar.style == BarStyle::Pills;
    if !pills {
        let line = bar.muted(0.88);
        scene
            .back
            .push(Rect::fill(bar.x, bar.y + bar.h - s, bar.w, s.max(1.0), line));
    }
    let mut hits = Vec::new();
    for (i, tab) in bar.tabs.iter().enumerate() {
        let x = bar.x + i as f32 * (tab_w + gap);
        let active = i == bar.active;
        if pills {
            let fill = bar.muted(if active { 0.88 } else { 0.95 });
            scene
                .back
                .push(Rect::fill(x, bar.y, tab_w, bar.h, fill).rounded(bar.radius * s));
        }
        let max = ((tab_w - 28.0 * s) / m.cell_width).floor().max(1.0) as usize;
        let text_w = tab_width(tab, max) as f32 * m.cell_width;
        let color = if active { bar.fg } else { bar.muted(0.4) };
        let tx = (x + (tab_w - text_w) / 2.0).round();
        let ty = (bar.y + (bar.h - m.cell_height) / 2.0).round();
        tab_text(r, tab, tx, ty, max, color, bar.muted(0.55), scene);
        if active {
            let (w, h) = if pills { (16.0 * s, 2.0 * s) } else { (tab_w, 2.0 * s) };
            let under = Rect::fill(
                x + (tab_w - w) / 2.0,
                bar.y + bar.h - h - if pills { s } else { 0.0 },
                w,
                h,
                bar.accent,
            );
            scene.back.push(under.rounded(s));
        }
        hits.push((x, bar.y, tab_w, bar.h));
    }
    hits
}

/// Segments packed left to right with arrows between them, one text row tall.
fn powerline(r: &mut Renderer, bar: &Bar, scene: &mut Scene) -> Vec<Hit> {
    let m = r.metrics();
    let cw = m.cell_width;
    let y = (bar.y + (bar.h - m.cell_height) / 2.0).round();
    let room = (bar.w / bar.tabs.len() as f32 / cw).floor() as usize;
    let mut x = bar.x;
    let mut hits = Vec::new();
    for (i, tab) in bar.tabs.iter().enumerate() {
        let active = i == bar.active;
        let fill = if active {
            bar.accent
        } else {
            bar.muted(0.82 + 0.06 * (i % 2) as f32)
        };
        let max = room.saturating_sub(4).max(1);
        // Past the first, a segment starts under the previous one's arrow.
        let lead = if i == 0 { 0.0 } else { cw };
        let w = (tab_width(tab, max) + 2) as f32 * cw + lead;
        scene.back.push(Rect::fill(x, y, w, m.cell_height, fill));
        let color = if active { bar.bg } else { bar.fg };
        let quiet = if active { bar.bg } else { bar.muted(0.4) };
        tab_text(r, tab, x + lead + cw, y, max, color, quiet, scene);
        let end = x + w;
        label(r, "\u{e0b0}", end, y, fill, Style::Regular, scene);
        hits.push((x, bar.y, w, bar.h));
        x = end;
    }
    hits
}

/// Just the titles, the current one in the accent.
fn plain(r: &mut Renderer, bar: &Bar, scene: &mut Scene) -> Vec<Hit> {
    let m = r.metrics();
    let cw = m.cell_width;
    let y = (bar.y + (bar.h - m.cell_height) / 2.0).round();
    let room = (bar.w / bar.tabs.len() as f32 / cw).floor() as usize;
    let mut x = bar.x;
    let mut hits = Vec::new();
    for (i, tab) in bar.tabs.iter().enumerate() {
        let active = i == bar.active;
        let color = if active { bar.accent } else { bar.muted(0.45) };
        let w = tab_text(
            r,
            tab,
            x,
            y,
            room.saturating_sub(3).max(1),
            color,
            bar.muted(0.6),
            scene,
        );
        hits.push((x, bar.y, w, bar.h));
        x += w + 3.0 * cw;
    }
    hits
}

fn fit(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    text.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
}

/// Draws a line of text from (x, y); returns its width.
pub fn label(r: &mut Renderer, text: &str, x: f32, y: f32, color: [f32; 4], style: Style, scene: &mut Scene) -> f32 {
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
    r.draw_line(&cells, x, y, scene);
    cells.len() as f32 * r.metrics().cell_width
}
