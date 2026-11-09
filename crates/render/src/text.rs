//! Text into the scene: a line of cells as glyphs from the atlas, rasterized once, and the box
//! and block characters drawn to fit the cell.

use super::*;

impl Renderer {
    /// Lays out one row of cells starting at (x, y), the row's top-left corner.
    pub fn draw_line(&mut self, cells: &[LineCell], x: f32, y: f32, scene: &mut Scene) {
        let m = self.fonts.metrics;
        let mut start = 0;
        let mut texts: Vec<&str> = Vec::with_capacity(cells.len());
        while start < cells.len() {
            let style = cells[start].style;
            let mut end = start;
            while end < cells.len() && cells[end].style == style {
                end += 1;
            }
            texts.clear();
            for (i, cell) in cells[start..end].iter().enumerate() {
                let mut chars = cell.text.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) if builtin::covers(c) => {
                        let cell_x = x + (start + i) as f32 * m.cell_width;
                        self.draw_builtin(c, cell_x, y, cell.color, scene);
                        texts.push(" ");
                    }
                    _ => texts.push(cell.text),
                }
            }
            if texts.iter().any(|t| !t.trim().is_empty()) {
                let placed = self.fonts.shape(style, &texts);
                for p in placed.iter() {
                    let index = start + p.cell;
                    let cell = &cells[index];
                    let Some(glyph) = self.glyph(p.face, p.glyph) else {
                        continue;
                    };
                    let cell_x = x + index as f32 * m.cell_width;
                    let span = if cell.wide { 2.0 } else { 1.0 } * m.cell_width;
                    let uv = [
                        glyph.origin[0] as f32,
                        glyph.origin[1] as f32,
                        glyph.size[0] as f32,
                        glyph.size[1] as f32,
                    ];
                    if glyph.color {
                        let (w, h) = (glyph.size[0] as f32, glyph.size[1] as f32);
                        let fit = (span / w).min(m.cell_height / h).min(1.0);
                        let (w, h) = (w * fit, h * fit);
                        scene.color.push(Sprite {
                            pos: [
                                (cell_x + (span - w) / 2.0).round(),
                                (y + (m.cell_height - h) / 2.0).round(),
                            ],
                            size: [w, h],
                            uv,
                            color: [1.0, 1.0, 1.0, cell.color[3]],
                        });
                    } else {
                        scene.mask.push(Sprite {
                            pos: [
                                (cell_x + p.x).round() + glyph.left as f32,
                                (y + m.baseline + p.y).round() - glyph.top as f32,
                            ],
                            size: [glyph.size[0] as f32, glyph.size[1] as f32],
                            uv,
                            color: cell.color,
                        });
                    }
                }
            }
            start = end;
        }
    }

    pub(super) fn draw_builtin(&mut self, c: char, x: f32, y: f32, color: [f32; 4], scene: &mut Scene) {
        let m = self.fonts.metrics;
        let cached = match self.drawn.get(&c) {
            Some(cached) => *cached,
            None => {
                let (w, h) = (m.cell_width as u32, m.cell_height as u32);
                let cached = builtin::draw(c, w, h, m.stroke).and_then(|pixels| {
                    let origin = self.insert(false, w, h, &pixels)?;
                    Some(Cached {
                        color: false,
                        origin,
                        size: [w, h],
                        left: 0,
                        top: 0,
                    })
                });
                self.drawn.insert(c, cached);
                cached
            }
        };
        if let Some(g) = cached {
            scene.mask.push(Sprite {
                pos: [x.round(), y.round()],
                size: [g.size[0] as f32, g.size[1] as f32],
                uv: [
                    g.origin[0] as f32,
                    g.origin[1] as f32,
                    g.size[0] as f32,
                    g.size[1] as f32,
                ],
                color,
            });
        }
    }

    pub(super) fn glyph(&mut self, face: u16, glyph: u16) -> Option<Cached> {
        if let Some(cached) = self.glyphs.get(&(face, glyph)) {
            return *cached;
        }
        let cached = self.rasterize(face, glyph);
        self.glyphs.insert((face, glyph), cached);
        cached
    }

    pub(super) fn rasterize(&mut self, face_index: u16, glyph: u16) -> Option<Cached> {
        let face = self.fonts.face(face_index);
        let px = self.fonts.px;
        let mut builder = self
            .scale
            .builder(face.font())
            .size(px)
            .hint(!cfg!(target_os = "macos"));
        if let Some(weight) = face.weight {
            builder = builder.variations(&[("wght", weight)][..]);
        }
        let mut scaler = builder.build();
        let mut render = Render::new(&[
            Source::ColorOutline(0),
            Source::ColorBitmap(StrikeWith::BestFit),
            Source::Outline,
        ]);
        render.format(Format::Alpha);
        if face.fake_bold {
            render.embolden((px / 28.0).max(0.5));
        }
        if face.fake_italic {
            render.transform(Some(Transform::skew(
                Angle::from_degrees(12.0),
                Angle::from_degrees(0.0),
            )));
        }
        let image = render.render(&mut scaler, glyph)?;
        let (w, h) = (image.placement.width, image.placement.height);
        if w == 0 || h == 0 {
            return None;
        }
        let color = matches!(image.content, Content::Color);
        let origin = self.insert(color, w, h, &image.data)?;
        Some(Cached {
            color,
            origin,
            size: [w, h],
            left: image.placement.left,
            top: image.placement.top,
        })
    }

    /// Places an image in an atlas, growing the atlas up to 4096² before starting it over.
    pub(super) fn insert(&mut self, color: bool, w: u32, h: u32, data: &[u8]) -> Option<[u32; 2]> {
        let atlas = if color { &mut self.color } else { &mut self.mask };
        if let Some(origin) = atlas.insert(&self.queue, w, h, data) {
            return Some(origin);
        }
        let size = atlas.size();
        if size < 4096 {
            let format = atlas.texture.format();
            *atlas = Atlas::new(&self.device, format, size * 2);
            self.bind_group = bind_group(
                &self.device,
                &self.layout,
                &self.globals,
                &self.mask,
                &self.color,
                &self.samplers[0],
                &self.samplers[1],
            );
        }
        self.reset_atlas();
        let atlas = if color { &mut self.color } else { &mut self.mask };
        atlas.insert(&self.queue, w, h, data)
    }
}
