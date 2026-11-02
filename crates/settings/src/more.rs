//! The pages for the picture and effects, the quick terminal and rules, and the rows the font,
//! colors, window, tabs and terminal pages gain from them.

use iced::widget::{Column, button, column, pick_list, row, slider, text, text_input, toggler};
use iced::{Alignment, Element};
use vif_config::{Fit, themes};

use crate::{Choice, MONO, Message, Settings, field, ranged, style};

/// Built-in shaders that are looks; the cursor trail has a toggle of its own.
const LOOKS: [(&str, &str); 2] = [("crt", "Old monitor"), ("bloom", "Glow")];

#[derive(Clone, Debug)]
pub enum Change {
    /// Text typed into a field, kept until Enter writes it.
    Draft(&'static str, String),
    Commit(&'static str),
    Fit(Choice),
    Dim(f32),
    PictureBlur(f32),
    Look(&'static str, bool),
    Trail(bool),
    QuickHeight(f32),
    QuickBlur(bool),
    Weight(f32),
    BoldWeight(f32),
    Italic(String),
    Detail(bool),
    Plus(bool),
    Clock(bool),
    Blur(f32),
    Rule(usize, &'static str, String),
    AddRule,
    RemoveRule(usize),
}

fn fit_name(fit: Fit) -> &'static str {
    match fit {
        Fit::Cover => "cover",
        Fit::Contain => "contain",
        Fit::Stretch => "stretch",
        Fit::Center => "center",
        Fit::Tile => "tile",
    }
}

impl Settings {
    pub(crate) fn change(&mut self, change: Change) {
        match change {
            Change::Draft(key, value) => {
                self.drafts.insert(key, value);
            }
            Change::Commit(key) => {
                let Some(value) = self.drafts.remove(key) else {
                    return;
                };
                let (section, name) = key.split_once('.').unwrap_or(("", key));
                let value = value.trim().to_owned();
                if matches!(key, "window.shader" | "font.features") {
                    let mut list: Vec<String> = value
                        .split([',', ' '])
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .collect();
                    if key == "window.shader" {
                        let looks = self
                            .config
                            .window
                            .shaders
                            .iter()
                            .filter(|s| LOOKS.iter().any(|(l, _)| l == s));
                        list = looks.cloned().chain(list).collect();
                    }
                    return self.edit(|e| e.set_list(section, name, &list));
                }
                if value.is_empty() {
                    self.edit(|e| e.remove(section, name));
                } else {
                    self.edit(|e| e.set(section, name, value));
                }
            }
            Change::Fit(Choice(v)) => self.edit(|e| e.set("background", "fit", v)),
            Change::Dim(v) => self.edit(|e| e.set_float("background", "dim", v)),
            Change::PictureBlur(v) => self.edit(|e| e.set("background", "blur", v.round() as i64)),
            Change::Look(name, on) => {
                let mut list: Vec<String> = self
                    .config
                    .window
                    .shaders
                    .iter()
                    .filter(|s| *s != name)
                    .cloned()
                    .collect();
                if on {
                    list.insert(0, name.to_owned());
                }
                self.edit(|e| e.set_list("window", "shader", &list));
            }
            Change::Trail(v) => self.edit(|e| e.set("cursor", "trail", v)),
            Change::QuickHeight(v) => self.edit(|e| e.set_float("quick", "height", v)),
            Change::QuickBlur(v) => self.edit(|e| e.set("quick", "hide_on_blur", v)),
            Change::Weight(v) => self.edit(|e| e.set("font", "weight", v.round() as i64)),
            Change::BoldWeight(v) => self.edit(|e| e.set("font", "bold_weight", v.round() as i64)),
            Change::Italic(family) if family == SAME => self.edit(|e| e.remove("font", "italic_family")),
            Change::Italic(family) => self.edit(|e| e.set("font", "italic_family", family)),
            Change::Detail(v) => self.edit(|e| e.set("tabs", "detail", v)),
            Change::Plus(v) => self.edit(|e| e.set("tabs", "plus", v)),
            Change::Clock(v) => self.edit(|e| e.set("badges", "clock", v)),
            Change::Blur(v) => self.edit(|e| e.set("window", "blur", v.round() as i64)),
            Change::Rule(i, key, value) => self.edit(|e| e.set_rule(i, key, &value)),
            Change::AddRule => self.edit(|e| e.add_rule("prod-*")),
            Change::RemoveRule(i) => self.edit(|e| e.remove_rule(i)),
        }
    }

    /// A text field that writes on Enter: its draft while typing, else what the config says.
    fn typed<'a>(&'a self, key: &'static str, now: String, hint: &'a str, width: f32) -> Element<'a, Message> {
        let value = self.drafts.get(key).cloned().unwrap_or(now);
        text_input(hint, &value)
            .on_input(move |v| Message::More(Change::Draft(key, v)))
            .on_submit(Message::More(Change::Commit(key)))
            .font(MONO)
            .size(13)
            .width(width)
            .style(style::input)
            .into()
    }

    pub(crate) fn effects_page(&self) -> Element<'_, Message> {
        let c = &self.config;
        let accent = self.accent();
        let fits = ["cover", "contain", "stretch", "center", "tile"].map(Choice);
        let mut looks = Column::new().spacing(8);
        for (name, title) in LOOKS {
            let on = c.window.shaders.iter().any(|s| s == name);
            looks = looks.push(
                row![
                    toggler(on)
                        .on_toggle(move |v| Message::More(Change::Look(name, v)))
                        .style(style::toggle(accent)),
                    text(title).size(13),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            );
        }
        let own: Vec<String> = c
            .window
            .shaders
            .iter()
            .filter(|s| !LOOKS.iter().any(|(l, _)| l == s))
            .cloned()
            .collect();
        column![
            field(
                "Picture",
                self.typed(
                    "background.image",
                    c.background.image.clone().unwrap_or_default(),
                    "~/Pictures/wall.jpg",
                    320.0
                ),
                Some("PNG or JPEG behind the text; Enter sets it"),
            ),
            field(
                "Fit",
                pick_list(fits, Some(Choice(fit_name(c.background.fit))), |v| Message::More(
                    Change::Fit(v)
                ))
                .width(200)
                .style(style::pick)
                .menu_style(style::menu)
                .into(),
                None,
            ),
            field(
                "Fade",
                ranged(
                    slider(0.0f32..=1.0, c.background.dim, |v| Message::More(Change::Dim(v))).step(0.05f32),
                    format!("{:.0}%", c.background.dim * 100.0),
                    accent
                ),
                Some("toward the background color, so text reads"),
            ),
            field(
                "Blur",
                ranged(
                    slider(0.0f32..=64.0, f32::from(c.background.blur), |v| Message::More(
                        Change::PictureBlur(v)
                    )),
                    format!("{} px", c.background.blur),
                    accent
                ),
                None,
            ),
            field("Looks", looks.into(), Some("shaders over the whole window")),
            field(
                "Your shaders",
                self.typed("window.shader", own.join(", "), "names in shaders/, or files", 320.0),
                Some("Shadertoy GLSL, the same as Ghostty's"),
            ),
            field(
                "Cursor trail",
                toggler(c.cursor.trail)
                    .on_toggle(|v| Message::More(Change::Trail(v)))
                    .style(style::toggle(accent))
                    .into(),
                Some("a streak where the cursor jumps from"),
            ),
        ]
        .spacing(18)
        .into()
    }

    pub(crate) fn quick_page(&self) -> Element<'_, Message> {
        let c = &self.config;
        let accent = self.accent();
        let key_hint = if cfg!(target_os = "macos") {
            "from any app; none until you set one"
        } else {
            "bind `vif quick` in your compositor instead"
        };
        column![
            field(
                "Key",
                self.typed("quick.key", c.quick.key.clone(), "ctrl+`", 200.0),
                Some(key_hint)
            ),
            field(
                "Height",
                ranged(
                    slider(0.15f32..=1.0, c.quick.height, |v| Message::More(Change::QuickHeight(v))).step(0.05f32),
                    format!("{:.0}% of the screen", c.quick.height * 100.0),
                    accent
                ),
                None,
            ),
            field(
                "Hide when left",
                toggler(c.quick.hide_on_blur)
                    .on_toggle(|v| Message::More(Change::QuickBlur(v)))
                    .style(style::toggle(accent))
                    .into(),
                Some("when another window takes the focus"),
            ),
        ]
        .spacing(18)
        .into()
    }

    pub(crate) fn rules_page(&self) -> Element<'_, Message> {
        let mut col = Column::new().spacing(14);
        let themes: Vec<String> = std::iter::once(String::new())
            .chain(themes::all().into_iter().map(|t| t.name))
            .collect();
        for (i, rule) in self.config.rules.iter().enumerate() {
            let input = |key: &'static str, hint: &'static str, value: Option<String>, width: f32| {
                text_input(hint, &value.unwrap_or_default())
                    .on_input(move |v| Message::More(Change::Rule(i, key, v)))
                    .font(MONO)
                    .size(13)
                    .width(width)
                    .style(style::input)
            };
            let theme = pick_list(themes.clone(), rule.theme.clone().or(Some(String::new())), move |v| {
                Message::More(Change::Rule(i, "theme", v))
            })
            .width(150)
            .style(style::pick)
            .menu_style(style::menu);
            col = col.push(
                row![
                    input("host", "host, like prod-*", rule.host.clone(), 170.0),
                    input("dir", "or a folder", rule.dir.clone(), 170.0),
                    theme,
                    input("accent", "#rrggbb", rule.accent.map(|c| c.to_hex()), 100.0),
                    input("label", "label", rule.label.clone(), 90.0),
                    button(text("Remove").size(13))
                        .style(style::ghost)
                        .on_press(Message::More(Change::RemoveRule(i))),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
        }
        col.push(
            button(text("Add a rule").size(13))
                .padding([8, 16])
                .style(style::choice(false, self.accent()))
                .on_press(Message::More(Change::AddRule)),
        )
        .push(text("A pane ssh'd into a host that fits, or sitting in a folder that fits, takes the rule's theme, accent and label. The first rule that fits wins.").size(12).color(style::faint()))
        .into()
    }

    /// Rows the font page gains: weights, an italic face, OpenType features.
    pub(crate) fn font_rows(&self) -> Vec<Element<'_, Message>> {
        let c = &self.config;
        let accent = self.accent();
        let italics: Vec<String> = std::iter::once(SAME.to_owned())
            .chain(self.families.iter().cloned())
            .collect();
        vec![
            field(
                "Weight",
                ranged(
                    slider(100.0f32..=900.0, f32::from(c.font.weight), |v| {
                        Message::More(Change::Weight(v))
                    })
                    .step(100.0f32),
                    c.font.weight.to_string(),
                    accent,
                ),
                Some("400 is regular, 500 medium"),
            ),
            field(
                "Bold weight",
                ranged(
                    slider(100.0f32..=900.0, f32::from(c.font.bold_weight), |v| {
                        Message::More(Change::BoldWeight(v))
                    })
                    .step(100.0f32),
                    c.font.bold_weight.to_string(),
                    accent,
                ),
                None,
            ),
            field(
                "Italic from",
                pick_list(
                    italics,
                    Some(c.font.italic_family.clone().unwrap_or_else(|| SAME.to_owned())),
                    |v| Message::More(Change::Italic(v)),
                )
                .width(300)
                .style(style::pick)
                .menu_style(style::menu)
                .into(),
                Some("a cursive italic from another face"),
            ),
            field(
                "Features",
                self.typed("font.features", c.font.features.join(", "), "ss01, cv02, -calt", 300.0),
                Some("OpenType features the font has"),
            ),
        ]
    }

    pub(crate) fn colors_from(&self) -> Element<'_, Message> {
        field(
            "Follow a file",
            self.typed(
                "colors.from",
                self.config.colors.from.clone().unwrap_or_default(),
                "wal",
                300.0,
            ),
            Some("\"wal\" for pywal, or colors from kitty, Ghostty, Alacritty, iTerm…"),
        )
    }

    pub(crate) fn blur_row(&self) -> Element<'_, Message> {
        let blur = self.config.window.blur;
        field(
            "Blur behind",
            ranged(
                slider(0.0f32..=64.0, f32::from(blur), |v| Message::More(Change::Blur(v))),
                if blur == 0 { "off".to_owned() } else { format!("{blur}") },
                self.accent(),
            ),
            Some("shows when the window is see-through"),
        )
    }

    pub(crate) fn tab_rows(&self) -> Vec<Element<'_, Message>> {
        let c = &self.config;
        let accent = self.accent();
        vec![
            field(
                "What runs there",
                toggler(c.tabs.detail)
                    .on_toggle(|v| Message::More(Change::Detail(v)))
                    .style(style::toggle(accent))
                    .into(),
                Some("the command or folder after the title"),
            ),
            field(
                "A + button",
                toggler(c.tabs.plus)
                    .on_toggle(|v| Message::More(Change::Plus(v)))
                    .style(style::toggle(accent))
                    .into(),
                None,
            ),
        ]
    }

    pub(crate) fn clock_row(&self) -> Element<'_, Message> {
        field(
            "Time of day on badges",
            toggler(self.config.badges.clock)
                .on_toggle(|v| Message::More(Change::Clock(v)))
                .style(style::toggle(self.accent()))
                .into(),
            Some("when each command ended"),
        )
    }
}

const SAME: &str = "the same face";
