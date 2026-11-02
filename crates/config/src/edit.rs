use std::ops::Range;
use std::path::{Path, PathBuf};
use std::{fs, io};

use toml_edit::{DocumentMut, Item, Table, Value};

use crate::Rgb;

/// Edits vif.toml in place: comments, order and formatting of untouched lines stay.
#[derive(Debug)]
pub struct Editor {
    doc: DocumentMut,
    path: PathBuf,
    saved: String,
}

impl Editor {
    pub fn open(path: &Path) -> Result<Self, String> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
            Err(err) => return Err(err.to_string()),
        };
        let mut editor = Self::parse(&text)?;
        editor.path = path.to_owned();
        Ok(editor)
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        text.parse::<DocumentMut>()
            .map(|doc| Self {
                doc,
                path: PathBuf::new(),
                saved: text.to_owned(),
            })
            .map_err(|err| format!("fix the syntax error first: {}", err.message()))
    }

    pub fn set(&mut self, section: &str, key: &str, value: impl Into<Value>) {
        let table = table(self.doc.as_table_mut(), section);
        put(table, key, value.into());
    }

    /// A key at the top of the file, above every section.
    pub fn set_top(&mut self, key: &str, value: impl Into<Value>) {
        put(self.doc.as_table_mut(), key, value.into());
    }

    pub fn remove_top(&mut self, key: &str) {
        self.doc.as_table_mut().remove(key);
    }

    pub fn set_float(&mut self, section: &str, key: &str, value: f32) {
        let value = (f64::from(value) * 100.0).round() / 100.0;
        if value.fract() == 0.0 {
            self.set(section, key, value as i64);
        } else {
            self.set(section, key, value);
        }
    }

    pub fn set_color(&mut self, section: &str, key: &str, color: Option<Rgb>) {
        match color {
            Some(color) => self.set(section, key, color.to_hex()),
            None => self.remove(section, key),
        }
    }

    pub fn set_padding(&mut self, x: u16, y: u16) {
        if x == y {
            self.set("window", "padding", i64::from(x));
        } else {
            let pair: toml_edit::Array = [i64::from(x), i64::from(y)].into_iter().collect();
            self.set("window", "padding", pair);
        }
    }

    /// Writes all 16 palette colors, or removes the key to go back to the theme's.
    pub fn set_palette(&mut self, colors: Option<&[Rgb; 16]>) {
        match colors {
            Some(colors) => {
                let list: toml_edit::Array = colors.iter().map(|c| c.to_hex()).collect();
                self.set("colors", "palette", list);
            }
            None => self.remove("colors", "palette"),
        }
    }

    pub fn set_path(&mut self, path: &Path) {
        self.path = path.to_owned();
    }

    pub fn remove(&mut self, section: &str, key: &str) {
        if let Some(table) = self.doc.get_mut(section).and_then(Item::as_table_mut) {
            table.remove(key);
        }
    }

    pub fn text(&self) -> String {
        self.doc.to_string()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes the file and returns the 1-based lines that changed since the last save.
    pub fn save(&mut self) -> io::Result<Range<usize>> {
        let text = self.text();
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(&self.path, &text)?;
        let changed = changed(&self.saved, &text);
        self.saved = text;
        Ok(changed)
    }

    /// The lines of one section with their 1-based numbers, as they are in the file.
    pub fn section(&self, name: &str) -> Vec<(usize, String)> {
        let text = self.text();
        let header = format!("[{name}]");
        let mut out = Vec::new();
        // An empty name means the lines above the first section, like `theme`.
        let mut inside = name.is_empty();
        for (i, line) in text.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                inside = !name.is_empty() && trimmed == header;
            }
            if inside {
                out.push((i + 1, line.to_owned()));
            }
        }
        while out.last().is_some_and(|(_, l)| l.trim().is_empty()) {
            out.pop();
        }
        out
    }
}

fn table<'a>(root: &'a mut Table, name: &str) -> &'a mut Table {
    let item = root.entry(name).or_insert_with(|| {
        let mut table = Table::new();
        table.set_implicit(false);
        Item::Table(table)
    });
    if !item.is_table() {
        *item = Item::Table(Table::new());
    }
    item.as_table_mut().expect("just made a table")
}

fn put(table: &mut Table, key: &str, value: Value) {
    match table.get_mut(key).and_then(Item::as_value_mut) {
        Some(old) => {
            let decor = old.decor().clone();
            *old = value;
            *old.decor_mut() = decor;
        }
        None => {
            table.insert(key, Item::Value(value));
        }
    }
}

fn changed(old: &str, new: &str) -> Range<usize> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    prefix + 1..b.len() - suffix + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_round_trips() {
        let mut editor = Editor::parse("theme = \"day\"\n[colors]\n").unwrap();
        let mut colors = crate::Palette::NIGHT.ansi;
        colors[1] = Rgb::hex(0xff0000);
        editor.set_palette(Some(&colors));
        let (config, diagnostics) = crate::Config::parse(&editor.text());
        assert!(diagnostics.is_empty());
        assert_eq!(config.palette().ansi[1], Rgb::hex(0xff0000));
        editor.set_palette(None);
        assert!(!editor.text().contains("palette"));
    }

    #[test]
    fn keeps_comments_and_order() {
        let src = "# mine\n[font]\n# the face\nfamily = \"Geist Mono\"\nsize = 13 # small\n\n[window]\nblur = 0\n";
        let mut editor = Editor::parse(src).unwrap();
        editor.set_float("font", "size", 14.5);
        assert_eq!(changed(src, &editor.text()), 5..6);
        editor.set("cursor", "shape", "beam");
        let text = editor.text();
        assert!(text.starts_with("# mine\n[font]\n# the face\nfamily = \"Geist Mono\"\nsize = 14.5 # small\n"));
        assert!(text.ends_with("[cursor]\nshape = \"beam\"\n"));
        let lines = editor.section("font");
        assert_eq!(lines[3], (5, "size = 14.5 # small".to_owned()));
        editor.set_top("theme", "nord");
        assert!(
            editor.text().starts_with("theme = \"nord\"\n# mine\n[font]"),
            "{}",
            editor.text()
        );
    }
}
