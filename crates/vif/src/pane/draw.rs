//! Drawing a pane: its rows built into quads and glyphs, only those that changed, with the badges,
//! images, selection and search matches over them.

use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{self, Term, TermDamage};
use alacritty_terminal::vte::ansi::CursorShape;
use vif_config::{BadgeShow, Badges};
use vif_render::{LineCell, RECT_CURL, RECT_DASHED, RECT_DOTTED, RECT_HOLLOW, Rect, Renderer, Scene, Style};

use super::*;
use crate::colors::{Rgba, Scheme, mix};
use crate::fold::{Shown, folded_row};
use crate::graphics;
use crate::hints;
use crate::marks::{self, Kind};

impl Pane {
    /// Draws each image whole, anchored at its topmost visible cell: text may reflow around
    /// it on resize, the picture stays in one piece.
    pub(super) fn draw_images(&self, r: &mut Renderer, scene: &mut Scene) {
        let mut anchors: std::collections::HashMap<u32, Mark> = std::collections::HashMap::new();
        // Starts of image rows first: after a reflow the tail of a row sits at another column.
        let rank = |m: &Mark| (m.col != 0, m.row, m.col);
        for mark in self.rows.iter().flatten().flat_map(|row| &row.marks) {
            let better = anchors.get(&mark.id).is_none_or(|a| rank(mark) < rank(a));
            if better {
                anchors.insert(mark.id, *mark);
            }
        }
        if anchors.is_empty() {
            return;
        }
        let m = r.metrics();
        let a = self.area;
        let placements = self.graphics.placements.lock().expect("placements");
        for (id, mark) in anchors {
            let Some(p) = placements.get(&id) else {
                continue;
            };
            let (iw, ih) = (p.image.width as f32, p.image.height as f32);
            let (bw, bh) = (p.cols as f32 * m.cell_width, p.rows as f32 * m.cell_height);
            let scale = if p.scaled { (bw / iw).min(bh / ih) } else { 1.0 };
            let (dw, dh) = (iw * scale, ih * scale);
            let x = mark.x - mark.col as f32 * m.cell_width;
            let y = mark.y - mark.row as f32 * m.cell_height;
            let (left, top) = (x.max(a.x), y.max(a.y));
            let (right, bottom) = ((x + dw).min(a.x + a.w), (y + dh).min(a.y + a.h));
            if right <= left || bottom <= top {
                continue;
            }
            let key = (self.id << 32) | u64::from(id);
            r.image(key, &p.image.rgba, p.image.width, p.image.height);
            scene.images.push((
                key,
                vif_render::Sprite {
                    pos: [left, top],
                    size: [right - left, bottom - top],
                    uv: [
                        (left - x) / dw,
                        (top - y) / dh,
                        (right - left) / dw,
                        (bottom - top) / dh,
                    ],
                    color: [1.0, 1.0, 1.0, 1.0],
                },
            ));
        }
    }

    /// Exit status and time at the end of each command line.
    pub(super) fn draw_badges(&mut self, r: &mut Renderer, look: &Look, scene: &mut Scene) {
        let m = r.metrics();
        let shown: Vec<(usize, usize, marks::Block)> = self
            .rows
            .iter()
            .enumerate()
            .filter_map(|(line, row)| {
                let row = row.as_ref()?;
                Some((line, row.end, self.blocks.get(row.block?)?))
            })
            .collect();
        self.ticking = shown.iter().any(|(_, _, b)| b.ended.is_none());
        for (line, end, block) in shown {
            let Some((text, color)) = badge(&block, look.scheme, look.badges) else {
                continue;
            };
            let len = text.chars().count();
            if end + len + 2 > self.cols {
                continue;
            }
            let chars: Vec<String> = text.chars().map(String::from).collect();
            let cells: Vec<LineCell> = chars
                .iter()
                .map(|t| LineCell {
                    text: t,
                    style: Style::Regular,
                    color,
                    wide: false,
                })
                .collect();
            let x = self.area.x + (self.cols - len - 1) as f32 * m.cell_width;
            let y = self.area.y + line as f32 * m.cell_height;
            r.draw_line(&cells, x, y, scene);
        }
    }

    pub fn draw(&mut self, r: &mut Renderer, look: &Look, scene: &mut Scene) {
        let m = r.metrics();
        let mut term = self.term.lock();
        if self.generation != r.generation() || self.rows.len() != self.lines {
            self.rows = vec![None; self.lines];
            self.generation = r.generation();
        }
        match term.damage() {
            TermDamage::Full => self.rows.iter_mut().for_each(|row| *row = None),
            TermDamage::Partial(lines) => {
                for damage in lines {
                    if let Some(row) = self.rows.get_mut(damage.line) {
                        *row = None;
                    }
                }
            }
        }
        term.reset_damage();
        // Folds move lines around the screen, so every row is drawn again while one shows.
        let view = self.fold_view(&term);
        if view.is_some() || self.view.is_some() {
            self.rows.iter_mut().for_each(|row| *row = None);
        }
        self.view = view;

        let content = term.renderable_content();
        let offset = content.display_offset as i32;
        let cursor = self
            .screen_of(offset, content.cursor.point.line.0)
            .map(|line| (content.cursor.point.column.0, line))
            .filter(|_| content.cursor.shape != CursorShape::Hidden);
        let shape = if look.focused {
            content.cursor.shape
        } else {
            CursorShape::HollowBlock
        };
        let solid = cursor.filter(|_| look.focused && look.cursor_on && shape == CursorShape::Block);
        let selection = content.selection;
        let colors = *content.colors;

        for line in [self.cursor_line, cursor.map(|c| c.1)].into_iter().flatten() {
            if let Some(row) = self.rows.get_mut(line) {
                *row = None;
            }
        }
        self.cursor_line = cursor.map(|c| c.1);

        // Copy what changed and let the PTY thread go before shaping and rasterizing.
        let cols = self.cols;
        let dirty: Vec<(usize, Shown, Vec<Cell>)> = (0..self.lines)
            .filter(|&line| self.rows[line].is_none())
            .map(|line| {
                let shown = self.shown(offset, line);
                let cells = match shown {
                    Shown::Line(grid_line) => {
                        let row = &term.grid()[Line(grid_line)];
                        (0..cols.min(row.len())).map(|c| row[Column(c)].clone()).collect()
                    }
                    _ => Vec::new(),
                };
                (line, shown, cells)
            })
            .collect();
        let wide_cursor = cursor.is_some_and(|(col, _)| {
            term.grid()[content.cursor.point.line][Column(col)]
                .flags
                .contains(Flags::WIDE_CHAR)
        });
        if let Some(hints) = &mut self.hints
            && !dirty.is_empty()
        {
            let (rows, links) = screen_text(&term, self.lines, self.cols);
            hints.refresh(&rows, &links);
        }
        drop(term);

        let (x0, y0) = (self.area.x, self.area.y);
        let o = self.outer;
        for (line, shown, cells) in dirty {
            let invert = solid.filter(|c| c.1 == line).map(|c| c.0);
            let y = y0 + line as f32 * m.cell_height;
            if let Shown::Folded { lines, .. } = shown {
                self.rows[line] = Some(Row {
                    scene: folded_row(r, look, lines, x0, y),
                    marks: Vec::new(),
                    block: None,
                    end: 0,
                });
                continue;
            }
            let bleed = Bleed {
                left: o.x,
                right: o.x + o.w,
                top: (line == 0).then_some(o.y),
                bottom: (line + 1 == self.lines).then_some(o.y + o.h),
            };
            self.rows[line] = Some(build_row(r, &cells, &colors, look, invert, x0, y, bleed));
        }
        for row in self.rows.iter().flatten() {
            scene.extend(&row.scene);
        }
        self.draw_images(r, scene);
        self.draw_badges(r, look, scene);

        if let Some(range) = selection {
            let color = rgba_alpha(look.scheme.palette.selection, 1.0);
            let last = self.cols.saturating_sub(1);
            for line in 0..self.lines {
                let Shown::Line(grid_line) = self.shown(offset, line) else {
                    continue;
                };
                if grid_line < range.start.line.0 || grid_line > range.end.line.0 {
                    continue;
                }
                let (from, to) = if range.is_block {
                    (range.start.column.0, range.end.column.0)
                } else {
                    let from = if grid_line == range.start.line.0 {
                        range.start.column.0
                    } else {
                        0
                    };
                    let to = if grid_line == range.end.line.0 {
                        range.end.column.0
                    } else {
                        last
                    };
                    (from, to)
                };
                if from > to {
                    continue;
                }
                scene.back.push(Rect::fill(
                    x0 + from as f32 * m.cell_width,
                    y0 + line as f32 * m.cell_height,
                    (to - from + 1) as f32 * m.cell_width,
                    m.cell_height,
                    color,
                ));
            }
        }

        if let Some((col, line)) = cursor.filter(|_| look.cursor_on || !look.focused) {
            let w = m.cell_width * if wide_cursor { 2.0 } else { 1.0 };
            let (x, y) = (x0 + col as f32 * m.cell_width, y0 + line as f32 * m.cell_height);
            let color = colors[alacritty_terminal::vte::ansi::NamedColor::Cursor]
                .map(|c| {
                    [
                        f32::from(c.r) / 255.0,
                        f32::from(c.g) / 255.0,
                        f32::from(c.b) / 255.0,
                        1.0,
                    ]
                })
                .unwrap_or_else(|| look.scheme.cursor());
            let beam = (m.cell_width / 8.0).round().max(2.0);
            match shape {
                CursorShape::Block => scene.back.push(Rect::fill(x, y, w, m.cell_height, color)),
                CursorShape::Beam => scene.front.push(Rect::fill(x, y, beam, m.cell_height, color)),
                CursorShape::Underline => scene
                    .front
                    .push(Rect::fill(x, y + m.cell_height - beam, w, beam, color)),
                CursorShape::HollowBlock => scene
                    .front
                    .push(Rect::fill(x, y, w, m.cell_height, color).kind(RECT_HOLLOW, m.stroke.max(1.0))),
                CursorShape::Hidden => {}
            }
        }
    }
}

pub(super) fn rgba_alpha(c: vif_config::Rgb, a: f32) -> Rgba {
    crate::colors::rgba(c, a)
}

/// `✓ 41.2s · 16:39`, `✗ exit 101 · 3.1s · 16:40`, `● running · 1:12`; with `show = "failures"`
/// only the failures.
pub(super) fn badge(block: &marks::Block, scheme: &Scheme, style: &Badges) -> Option<(String, Rgba)> {
    let took = block.running();
    let all = style.show == BadgeShow::All;
    let mark = |glyph: &str, rest: String| {
        if glyph.is_empty() {
            rest
        } else {
            format!("{glyph} {rest}")
        }
    };
    let ended = |text: String| {
        if style.clock {
            format!("{text} · {}", marks::time_of_day(block.finished_at()))
        } else {
            text
        }
    };
    match block.ended {
        _ if style.show == BadgeShow::None => None,
        None => all.then(|| {
            let text = format!("running · {}", marks::clock(took));
            (mark(&style.running, text), scheme.accent())
        }),
        Some((_, 0)) => all.then(|| {
            let text = ended(marks::clock(took));
            (mark(&style.ok, text), mix(scheme.fg(), scheme.bg(), 0.5))
        }),
        Some((_, code)) => {
            let text = ended(format!("exit {code} · {}", marks::clock(took)));
            Some((mark(&style.failed, text), rgba_alpha(scheme.palette.ansi[1], 1.0)))
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_row(
    r: &mut Renderer,
    row: &[Cell],
    colors: &term::color::Colors,
    look: &Look,
    invert: Option<usize>,
    x0: f32,
    y: f32,
    bleed: Bleed,
) -> Row {
    let m = r.metrics();
    let scheme = look.scheme;
    let mut scene = Scene::default();
    let cols = row.len();
    let mut text = String::with_capacity(cols);
    let mut spans = Vec::with_capacity(cols);
    let mut meta: Vec<(Style, Rgba, bool)> = Vec::with_capacity(cols);
    let mut run: Option<(usize, Rgba)> = None;
    let mut marks = Vec::new();
    let (mut block, mut end) = (None, 0);
    let flush = |run: &mut Option<(usize, Rgba)>, end: usize, scene: &mut Scene| {
        if let Some((start, color)) = run.take() {
            let left = if start == 0 {
                bleed.left
            } else {
                x0 + start as f32 * m.cell_width
            };
            let right = if end == cols {
                bleed.right
            } else {
                x0 + end as f32 * m.cell_width
            };
            let top = bleed.top.unwrap_or(y);
            let bottom = bleed.bottom.unwrap_or(y + m.cell_height);
            scene
                .back
                .push(Rect::fill(left, top, right - left, bottom - top, color));
        }
    };
    for (col, cell) in row.iter().enumerate() {
        let flags = cell.flags;
        let (mut fg, bg, plain_bg) = scheme.cell(colors, cell.fg, cell.bg, flags);
        match run {
            Some((_, color)) if !plain_bg && color == bg => {}
            _ => {
                flush(&mut run, col, &mut scene);
                if !plain_bg {
                    run = Some((col, bg));
                }
            }
        }
        if invert == Some(col) {
            fg = if plain_bg { scheme.bg() } else { bg };
        }

        let start = text.len();
        if cell.c == graphics::PLACEHOLDER {
            if let Some(mark) = Mark::read(cell, x0 + col as f32 * m.cell_width, y)
                && !marks.iter().any(|m: &Mark| m.id == mark.id)
            {
                marks.push(mark);
            }
        } else if !flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            text.push(cell.c);
            if let Some(extra) = cell.zerowidth() {
                text.extend(extra.iter().filter(|c| !crate::marks::is_tag(**c)));
                block = block.or_else(|| crate::marks::find(extra, Kind::Command));
            }
            if cell.c != ' ' {
                end = col + 1;
            }
        }
        spans.push(start..text.len());
        let style = Style::new(flags.contains(Flags::BOLD), flags.contains(Flags::ITALIC));
        meta.push((style, fg, flags.contains(Flags::WIDE_CHAR)));

        let x = x0 + col as f32 * m.cell_width;
        let w = m.cell_width * if flags.contains(Flags::WIDE_CHAR) { 2.0 } else { 1.0 };
        let underline = cell.underline_color().map(|c| scheme.resolve(colors, c)).unwrap_or(fg);
        let s = m.stroke;
        let linked = look
            .link
            .is_some_and(|id| cell.hyperlink().is_some_and(|h| h.id() == id));
        if flags.contains(Flags::UNDERCURL) {
            scene
                .front
                .push(Rect::fill(x, y + m.underline - s, w, s * 3.0, underline).kind(RECT_CURL, s));
        } else if flags.contains(Flags::DOUBLE_UNDERLINE) {
            scene.front.push(Rect::fill(x, y + m.underline - s, w, s, underline));
            scene.front.push(Rect::fill(x, y + m.underline + s, w, s, underline));
        } else if flags.contains(Flags::DOTTED_UNDERLINE) {
            scene
                .front
                .push(Rect::fill(x, y + m.underline, w, s, underline).kind(RECT_DOTTED, s));
        } else if flags.contains(Flags::DASHED_UNDERLINE) {
            scene
                .front
                .push(Rect::fill(x, y + m.underline, w, s, underline).kind(RECT_DASHED, s));
        } else if flags.contains(Flags::UNDERLINE) || linked {
            scene.front.push(Rect::fill(x, y + m.underline, w, s, underline));
        } else if cell.hyperlink().is_some() {
            let faint = mix(underline, scheme.bg(), 0.6);
            scene
                .front
                .push(Rect::fill(x, y + m.underline, w, s, faint).kind(RECT_DOTTED, s));
        }
        if flags.contains(Flags::STRIKEOUT) {
            scene.front.push(Rect::fill(x, y + m.strikeout, w, s, fg));
        }
    }
    flush(&mut run, cols, &mut scene);

    let cells: Vec<LineCell> = spans
        .iter()
        .zip(&meta)
        .map(|(span, (style, color, wide))| LineCell {
            text: &text[span.clone()],
            style: *style,
            color: *color,
            wide: *wide,
        })
        .collect();
    r.draw_line(&cells, x0, y, &mut scene);
    Row {
        scene,
        marks,
        block,
        end,
    }
}

/// The visible rows as text, and the OSC 8 links on them.
pub(super) fn screen_text(term: &Term<Proxy>, lines: usize, cols: usize) -> (Vec<hints::Row>, Vec<hints::Link>) {
    let offset = term.grid().display_offset() as i32;
    let mut rows = Vec::with_capacity(lines);
    let mut links = Vec::new();
    for line in 0..lines {
        let row = &term.grid()[Line(line as i32 - offset)];
        let n = cols.min(row.len());
        let (mut text, mut at) = (Vec::with_capacity(n), Vec::with_capacity(n));
        let mut link: Option<(String, hints::Link)> = None;
        for col in 0..n {
            let cell = &row[Column(col)];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            text.push(if cell.c == graphics::PLACEHOLDER { ' ' } else { cell.c });
            at.push(col);
            let h = cell.hyperlink();
            match (&h, &mut link) {
                (Some(h), Some((id, l))) if id == h.id() => l.end = col,
                _ => {
                    links.extend(link.take().map(|(_, l)| l));
                    link = h.map(|h| {
                        let l = hints::Link {
                            line,
                            start: col,
                            end: col,
                            uri: h.uri().to_owned(),
                        };
                        (h.id().to_owned(), l)
                    });
                }
            }
        }
        links.extend(link.map(|(_, l)| l));
        let wraps = n > 0 && row[Column(n - 1)].flags.contains(Flags::WRAPLINE);
        rows.push(hints::Row { text, cols: at, wraps });
    }
    (rows, links)
}
