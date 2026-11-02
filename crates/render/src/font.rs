use std::collections::HashMap;
use std::sync::Arc;

use swash::shape::ShapeContext;
use swash::text::Script;
use swash::{CacheKey, FontRef, GlyphId};

pub(crate) const BUNDLED: &[u8] = include_bytes!("../../../assets/fonts/GeistMono.ttf");
const BUNDLED_FAMILY: &str = "Geist Mono";

#[derive(Clone, Debug, PartialEq)]
pub struct FontConfig {
    pub family: String,
    /// Points; one point is one logical pixel on macOS and 4/3 of one elsewhere.
    pub size: f32,
    pub line_height: f32,
    pub ligatures: bool,
    pub scale: f32,
    pub faces: Faces,
}

/// The faces beyond the family: weights, other families for bold and italic, OpenType features.
#[derive(Clone, Debug, PartialEq)]
pub struct Faces {
    pub weight: u16,
    pub bold_weight: u16,
    pub bold_family: Option<String>,
    pub italic_family: Option<String>,
    /// Tags with their values, like ("ss01", 1) or ("calt", 0).
    pub features: Vec<(String, u16)>,
}

impl Default for Faces {
    fn default() -> Self {
        Self {
            weight: 400,
            bold_weight: 700,
            bold_family: None,
            italic_family: None,
            features: Vec::new(),
        }
    }
}

impl FontConfig {
    pub fn px(&self) -> f32 {
        let dpi = if cfg!(target_os = "macos") { 1.0 } else { 4.0 / 3.0 };
        (self.size * dpi * self.scale).max(4.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub cell_width: f32,
    pub cell_height: f32,
    pub baseline: f32,
    pub underline: f32,
    pub stroke: f32,
    pub strikeout: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Style {
    #[default]
    Regular,
    Bold,
    Italic,
    BoldItalic,
}

impl Style {
    pub fn new(bold: bool, italic: bool) -> Self {
        match (bold, italic) {
            (false, false) => Style::Regular,
            (true, false) => Style::Bold,
            (false, true) => Style::Italic,
            (true, true) => Style::BoldItalic,
        }
    }

    fn bold(self) -> bool {
        matches!(self, Style::Bold | Style::BoldItalic)
    }
}

pub(crate) struct Face {
    data: Arc<dyn AsRef<[u8]> + Send + Sync>,
    offset: u32,
    key: CacheKey,
    pub(crate) weight: Option<f32>,
    pub(crate) fake_bold: bool,
    pub(crate) fake_italic: bool,
}

impl Face {
    fn load(data: Arc<dyn AsRef<[u8]> + Send + Sync>, index: u32) -> Option<Self> {
        let font = FontRef::from_index((*data).as_ref(), index as usize)?;
        let (offset, key) = (font.offset, font.key);
        Some(Self {
            data,
            offset,
            key,
            weight: None,
            fake_bold: false,
            fake_italic: false,
        })
    }

    pub(crate) fn font(&self) -> FontRef<'_> {
        FontRef {
            data: (*self.data).as_ref(),
            offset: self.offset,
            key: self.key,
        }
    }

    fn styled(&self, weight: Option<f32>, fake_bold: bool, fake_italic: bool) -> Self {
        Self {
            data: self.data.clone(),
            offset: self.offset,
            key: self.key,
            weight,
            fake_bold,
            fake_italic,
        }
    }

    fn glyph(&self, c: char) -> GlyphId {
        self.font().charmap().map(c)
    }

    fn has_weight_axis(&self) -> bool {
        self.font()
            .variations()
            .find_by_tag(swash::tag_from_bytes(b"wght"))
            .is_some()
    }
}

/// A glyph placed relative to the left edge of the cell it starts in.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Placed {
    pub cell: usize,
    pub face: u16,
    pub glyph: GlyphId,
    pub x: f32,
    pub y: f32,
}

pub(crate) struct Fonts {
    pub faces: Vec<Face>,
    pub metrics: Metrics,
    pub px: f32,
    ligatures: bool,
    /// What the shaper is told: the config's features, and ligatures off when they are.
    features: Vec<(String, u16)>,
    fallback: HashMap<char, Option<u16>>,
    loaded: HashMap<(fontdb::ID, bool), u16>,
    index: Coverage,
    system: Option<fontdb::Database>,
    shaper: ShapeContext,
    shaped: HashMap<(Style, String), Arc<[Placed]>>,
}

impl Fonts {
    pub fn new(config: &FontConfig) -> Self {
        let px = config.px();
        let faces = &config.faces;
        let mut system = None;
        let bundled: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(BUNDLED);
        let ours = config.family.eq_ignore_ascii_case(BUNDLED_FAMILY) || config.family.is_empty();
        let regular = if ours {
            None
        } else {
            let db = system.get_or_insert_with(load_system);
            find(db, &config.family, faces.weight, false)
        };
        let regular = regular.unwrap_or_else(|| {
            if !ours {
                log::warn!("font “{}” not found, using {BUNDLED_FAMILY}", config.family);
            }
            Face::load(bundled.clone(), 0).expect("bundled font parses")
        });
        let variable = regular.has_weight_axis();
        let styled = |bold: bool, italic: bool, system: &mut Option<fontdb::Database>| {
            let weight = if bold { faces.bold_weight } else { faces.weight };
            let other = if italic {
                faces.italic_family.as_ref()
            } else if bold {
                faces.bold_family.as_ref()
            } else {
                None
            };
            let families = other.into_iter().chain((!variable && !ours).then_some(&config.family));
            for family in families {
                let db = system.get_or_insert_with(load_system);
                if let Some(face) = find(db, family, weight, italic) {
                    return face;
                }
            }
            regular.styled(variable.then_some(f32::from(weight)), bold && !variable, italic)
        };
        let bold = styled(true, false, &mut system);
        let italic = styled(false, true, &mut system);
        let bold_italic = styled(true, true, &mut system);
        let regular = if variable {
            regular.styled(Some(f32::from(faces.weight)), false, false)
        } else {
            regular
        };
        let mut features = faces.features.clone();
        if !config.ligatures && !features.is_empty() {
            features.extend(["liga", "calt", "dlig"].map(|t| (t.to_owned(), 0)));
        }
        let metrics = measure(&regular, px, config.line_height);
        Self {
            faces: vec![regular, bold, italic, bold_italic],
            metrics,
            px,
            ligatures: config.ligatures,
            features,
            fallback: HashMap::new(),
            loaded: HashMap::new(),
            index: Coverage::default(),
            system,
            shaper: ShapeContext::new(),
            shaped: HashMap::new(),
        }
    }

    pub fn face(&self, index: u16) -> &Face {
        &self.faces[index as usize]
    }

    fn style_face(style: Style) -> u16 {
        match style {
            Style::Regular => 0,
            Style::Bold => 1,
            Style::Italic => 2,
            Style::BoldItalic => 3,
        }
    }

    /// Face index and glyph for a character, falling back to system fonts.
    pub fn lookup(&mut self, c: char, style: Style) -> (u16, GlyphId) {
        let index = Self::style_face(style);
        let glyph = self.faces[index as usize].glyph(c);
        if glyph != 0 {
            return (index, glyph);
        }
        match self.fallback_face(c, style) {
            Some(face) => (face, self.faces[face as usize].glyph(c)),
            None => (index, 0),
        }
    }

    fn fallback_face(&mut self, c: char, style: Style) -> Option<u16> {
        if let Some(found) = self.fallback.get(&c) {
            return *found;
        }
        let db = self.system.get_or_insert_with(load_system);
        let covers = |id: fontdb::ID| {
            db.with_face_data(id, |data, index| {
                FontRef::from_index(data, index as usize).is_some_and(|f| f.charmap().map(c) != 0 && drawable(&f))
            })
            .unwrap_or(false)
        };
        let mut found = preferred(c).iter().find_map(|name| {
            let query = fontdb::Query {
                families: &[fontdb::Family::Name(name)],
                ..Default::default()
            };
            db.query(&query).filter(|id| covers(*id))
        });
        if found.is_none() {
            found = self.index.find(db, c);
        }
        let index = found.and_then(|id| {
            let fake_bold = style.bold() && db.face(id).is_some_and(|i| i.weight.0 < 600);
            if let Some(&index) = self.loaded.get(&(id, fake_bold)) {
                return Some(index);
            }
            let info = db.face(id)?;
            log::debug!("fallback for {c:?}: {:?}", info.families.first());
            let index = info.index;
            let face = Face::load(face_data(db, id)?, index)?.styled(None, fake_bold, false);
            self.faces.push(face);
            let index = (self.faces.len() - 1) as u16;
            self.loaded.insert((id, fake_bold), index);
            Some(index)
        });
        self.fallback.insert(c, index);
        index
    }

    /// Shapes a run of cells that share a style. `text` holds one entry per cell:
    /// the cell's characters, or nothing for the right half of a wide character.
    pub fn shape(&mut self, style: Style, cells: &[&str]) -> Arc<[Placed]> {
        let mut text = String::new();
        let mut starts = Vec::with_capacity(cells.len());
        for cell in cells {
            starts.push(text.len());
            text.push_str(cell);
        }
        let key = (style, text);
        if let Some(hit) = self.shaped.get(&key) {
            return hit.clone();
        }
        let text = &key.1;
        let plain = text.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ');
        let simple = self.features.is_empty() && (!self.ligatures || plain);
        let placed: Arc<[Placed]> = if simple {
            self.place_simple(style, cells).into()
        } else {
            self.place_shaped(style, text, &starts, cells).into()
        };
        if self.shaped.len() > 8192 {
            self.shaped.clear();
        }
        self.shaped.insert(key, placed.clone());
        placed
    }

    fn place_simple(&mut self, style: Style, cells: &[&str]) -> Vec<Placed> {
        let mut out = Vec::with_capacity(cells.len());
        for (cell, text) in cells.iter().enumerate() {
            for c in text.chars().filter(|c| *c != ' ') {
                let (face, glyph) = self.lookup(c, style);
                if glyph != 0 || face as usize >= 4 {
                    out.push(Placed {
                        cell,
                        face,
                        glyph,
                        x: 0.0,
                        y: 0.0,
                    });
                }
            }
        }
        out
    }

    fn place_shaped(&mut self, style: Style, text: &str, starts: &[usize], cells: &[&str]) -> Vec<Placed> {
        let index = Self::style_face(style);
        let face = &self.faces[index as usize];
        let font = face.font();
        let weight = face.weight;
        let mut builder = self
            .shaper
            .builder(font)
            .script(Script::Latin)
            .size(self.px)
            .features(self.features.iter().map(|(tag, v)| (tag.as_str(), *v)));
        if let Some(weight) = weight {
            builder = builder.variations(&[("wght", weight)][..]);
        }
        let mut shaper = builder.build();
        shaper.add_str(text);
        let mut clusters = Vec::new();
        shaper.shape_with(|cluster| {
            let mut pen = 0.0;
            for glyph in cluster.glyphs {
                clusters.push((cluster.source.start as usize, glyph.id, pen + glyph.x, glyph.y));
                pen += glyph.advance;
            }
        });
        let mut out = Vec::with_capacity(clusters.len());
        let mut missing = Vec::new();
        for (start, glyph, x, y) in clusters {
            let cell = starts.partition_point(|&s| s <= start).saturating_sub(1);
            if glyph == 0 {
                missing.push(cell);
                continue;
            }
            out.push(Placed {
                cell,
                face: index,
                glyph,
                x,
                y: -y,
            });
        }
        missing.dedup();
        for cell in missing {
            for c in cells[cell].chars() {
                let (face, glyph) = self.lookup(c, style);
                out.push(Placed {
                    cell,
                    face,
                    glyph,
                    x: 0.0,
                    y: 0.0,
                });
            }
        }
        out
    }
}

/// Which installed face draws each character, worked out one Unicode block at a time.
#[derive(Default)]
struct Coverage {
    candidates: Option<Vec<fontdb::ID>>,
    blocks: HashMap<u32, Box<[Option<u16>; 128]>>,
}

impl Coverage {
    fn find(&mut self, db: &fontdb::Database, c: char) -> Option<fontdb::ID> {
        let candidates = self.candidates.get_or_insert_with(|| candidates(db));
        let block = c as u32 >> 7;
        let table = self.blocks.entry(block).or_insert_with(|| {
            let mut table = Box::new([None; 128]);
            let mut left = 128;
            for (i, id) in candidates.iter().enumerate() {
                db.with_face_data(*id, |data, index| {
                    let Some(font) = FontRef::from_index(data, index as usize) else {
                        return;
                    };
                    let charmap = font.charmap();
                    for (slot, entry) in table.iter_mut().enumerate() {
                        if entry.is_none()
                            && let Some(ch) = char::from_u32(block << 7 | slot as u32)
                            && charmap.map(ch) != 0
                        {
                            *entry = Some(i as u16);
                            left -= 1;
                        }
                    }
                });
                if left == 0 {
                    break;
                }
            }
            table
        });
        table[(c as u32 & 127) as usize].map(|i| candidates[i as usize])
    }
}

/// Installed faces worth falling back to, the likeliest first.
fn candidates(db: &fontdb::Database) -> Vec<fontdb::ID> {
    let mut list: Vec<(u8, fontdb::ID)> = db
        .faces()
        .filter(|info| {
            let name = info.families.first().map_or("", |(n, _)| n.as_str());
            !name.starts_with('.') && name != "LastResort" && !name.contains("Bitmap")
        })
        .filter(|info| {
            db.with_face_data(info.id, |data, index| {
                FontRef::from_index(data, index as usize).is_some_and(|f| drawable(&f))
            })
            .unwrap_or(false)
        })
        .map(|info| {
            let rank =
                u8::from(info.monospaced) * 2 + u8::from(info.style == fontdb::Style::Normal && info.weight.0 == 400);
            (3 - rank, info.id)
        })
        .collect();
    list.sort_by_key(|(rank, _)| *rank);
    list.into_iter().map(|(_, id)| id).collect()
}

fn measure(face: &Face, px: f32, line_height: f32) -> Metrics {
    let font = face.font();
    let coords: Vec<_> = match face.weight {
        Some(w) => font.variations().normalized_coords([("wght", w)]).collect(),
        None => Vec::new(),
    };
    let m = font.metrics(&coords).scale(px);
    let gid = font.charmap().map('0');
    let advance = font.glyph_metrics(&coords).scale(px).advance_width(gid);
    let natural = m.ascent + m.descent + m.leading;
    let cell_height = (natural * line_height).round().max(1.0);
    let baseline = ((cell_height - natural) / 2.0 + m.ascent).round();
    let stroke = m.stroke_size.max(1.0).round();
    Metrics {
        cell_width: advance.round().max(1.0),
        cell_height,
        baseline,
        underline: (baseline - m.underline_offset + stroke / 2.0)
            .round()
            .min(cell_height - stroke),
        stroke,
        strikeout: (baseline - m.strikeout_offset).round(),
    }
}

fn load_system() -> fontdb::Database {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    db
}

fn find(db: &mut fontdb::Database, family: &str, weight: u16, italic: bool) -> Option<Face> {
    let query = fontdb::Query {
        families: &[fontdb::Family::Name(family)],
        weight: fontdb::Weight(weight),
        stretch: fontdb::Stretch::Normal,
        style: if italic {
            fontdb::Style::Italic
        } else {
            fontdb::Style::Normal
        },
    };
    let id = db.query(&query)?;
    let info = db.face(id)?;
    if !info.families.iter().any(|(name, _)| name.eq_ignore_ascii_case(family)) {
        return None;
    }
    let fake_italic = italic && info.style == fontdb::Style::Normal;
    let (found, index) = (info.weight.0, info.index);
    let face = Face::load(face_data(db, id)?, index)?;
    let variable = face.has_weight_axis();
    let fake_bold = weight >= 600 && found < 600 && !variable;
    Some(face.styled(variable.then_some(f32::from(weight)), fake_bold, fake_italic))
}

/// The face's file, mapped rather than read: Apple Color Emoji is 190 MB and a terminal touches a few pages of it.
fn face_data(db: &mut fontdb::Database, id: fontdb::ID) -> Option<Arc<dyn AsRef<[u8]> + Send + Sync>> {
    // SAFETY: installed fonts don't change under a running process.
    unsafe { db.make_shared_face_data(id) }.map(|(data, _)| data)
}

/// Fonts known to draw a script well, tried before searching every installed face.
fn preferred(c: char) -> &'static [&'static str] {
    match c as u32 {
        0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2B00..=0x2BFF | 0xFE0F => {
            &["Apple Color Emoji", "Noto Color Emoji", "Twemoji"]
        }
        0x1100..=0x11FF | 0xAC00..=0xD7AF => &["Apple SD Gothic Neo", "Noto Sans CJK KR", "Noto Sans KR"],
        0x2E80..=0x9FFF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x3FFFF => &[
            "PingFang SC",
            "Hiragino Sans",
            "Noto Sans Mono CJK SC",
            "Noto Sans CJK SC",
            "Source Han Sans SC",
        ],
        0xE000..=0xF8FF | 0xF0000..=0x10FFFF => &["Symbols Nerd Font Mono", "Symbols Nerd Font"],
        _ => &[
            "Menlo",
            "SF Mono",
            "DejaVu Sans Mono",
            "Noto Sans Mono",
            "Apple Symbols",
            "Noto Sans Symbols 2",
            "Noto Sans Math",
        ],
    }
}

/// Bitmap-only faces have nothing to draw at most sizes.
fn drawable(font: &FontRef) -> bool {
    [b"glyf", b"CFF ", b"CFF2", b"sbix", b"COLR", b"CBDT"]
        .iter()
        .any(|tag| font.table(swash::tag_from_bytes(tag)).is_some())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn falls_back_to_faces_that_draw() {
        let mut fonts = Fonts::new(&FontConfig {
            family: BUNDLED_FAMILY.into(),
            size: 13.0,
            line_height: 1.2,
            ligatures: true,
            scale: 2.0,
            faces: Faces::default(),
        });
        let mut scale = swash::scale::ScaleContext::new();
        for c in ['漢', 'か', '⣿', '🚀'] {
            let (face, glyph) = fonts.lookup(c, Style::Regular);
            assert!(face >= 4 && glyph != 0, "{c:?}");
            let mut scaler = scale.builder(fonts.face(face).font()).size(26.0).build();
            let image = swash::scale::Render::new(&[
                swash::scale::Source::ColorBitmap(swash::scale::StrikeWith::BestFit),
                swash::scale::Source::Outline,
            ])
            .render(&mut scaler, glyph);
            assert!(image.is_some_and(|i| i.placement.width > 0), "{c:?}");
        }
        assert_eq!(
            fonts.lookup('か', Style::Regular).0,
            fonts.lookup('漢', Style::Regular).0
        );
    }
}
