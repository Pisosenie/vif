mod gallery;
mod greeting;
mod more;
mod preview;
mod style;

use std::collections::{BTreeSet, HashMap};
use std::ops::Range;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use iced::widget::{
    Column, Row, button, column, container, pick_list, responsive, row, scrollable, shader, slider, space, text,
    text_input, toggler,
};
use iced::{Alignment, Element, Fill, Font, Length, Subscription, Task, Theme, keyboard, window};
use vif_config::{
    ACTIONS, BadgeShow, Config, CursorShape, Decorations, Diagnostic, Divider, Editor, OptionAsAlt, Osc52, Palette,
    Rgb, TabPosition, TabShow, TabStyle, themes,
};

use preview::Preview;

const SANS: Font = Font::with_name("Geist");
const MONO: Font = Font::with_name("Geist Mono");

fn main() -> iced::Result {
    let args: Vec<String> = std::env::args().collect();
    let page = args
        .iter()
        .position(|a| a == "--page")
        .and_then(|i| args.get(i + 1))
        .and_then(|name| {
            let name = name.to_lowercase();
            Page::ALL
                .into_iter()
                .find(|p| p.section() == name || p.title().to_lowercase() == name)
        })
        .unwrap_or(Page::Themes);
    iced::application(move || Settings::new(page), Settings::update, Settings::view)
        .title("vif settings")
        .subscription(Settings::subscription)
        .theme(|_: &Settings| Theme::Dark)
        .font(include_bytes!("../../../assets/fonts/Geist-Regular.ttf").as_slice())
        .font(include_bytes!("../../../assets/fonts/Geist-Medium.ttf").as_slice())
        .font(include_bytes!("../../../assets/fonts/Geist-SemiBold.ttf").as_slice())
        .font(include_bytes!("../../../assets/fonts/GeistMono.ttf").as_slice())
        .default_font(SANS)
        .window(window::Settings {
            size: iced::Size::new(1180.0, 780.0),
            min_size: Some(iced::Size::new(760.0, 560.0)),
            ..window::Settings::default()
        })
        .run()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Themes,
    Font,
    Colors,
    Effects,
    Window,
    Cursor,
    Tabs,
    Quick,
    Greeting,
    Rules,
    Keys,
    Terminal,
}

impl Page {
    const ALL: [Page; 12] = [
        Page::Themes,
        Page::Font,
        Page::Colors,
        Page::Effects,
        Page::Window,
        Page::Cursor,
        Page::Tabs,
        Page::Quick,
        Page::Greeting,
        Page::Rules,
        Page::Keys,
        Page::Terminal,
    ];

    fn title(self) -> &'static str {
        match self {
            Page::Themes => "Themes",
            Page::Font => "Font",
            Page::Colors => "Colors",
            Page::Effects => "Picture and effects",
            Page::Window => "Window",
            Page::Cursor => "Cursor",
            Page::Tabs => "Tabs and splits",
            Page::Quick => "Quick terminal",
            Page::Greeting => "Greeting",
            Page::Rules => "Rules",
            Page::Keys => "Keys",
            Page::Terminal => "Terminal",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Page::Themes => "The whole look in one file. Pick one, or make it yours.",
            Page::Font => "The face, its size and the space between lines.",
            Page::Colors => "The colors you change on top of the theme.",
            Page::Effects => "A picture behind the text and shaders over it.",
            Page::Window => "Padding, see-through glass and the window frame.",
            Page::Cursor => "How the cursor looks when nothing asks for another shape.",
            Page::Tabs => "Where the tabs sit and how they look, and how splits part.",
            Page::Quick => "One window that drops in from the top of the screen, its shell always running.",
            Page::Greeting => "A picture played in text when a shell starts, with this machine beside it.",
            Page::Rules => {
                "Hosts and folders that change how a pane looks, so production never looks like your laptop."
            }
            Page::Keys => "Shortcuts for vif itself. Everything else goes to the program.",
            Page::Terminal => "The shell, the scrollback and what programs may do.",
        }
    }

    fn section(self) -> &'static str {
        match self {
            Page::Themes => "",
            Page::Font => "font",
            Page::Colors => "colors",
            Page::Effects => "background",
            Page::Window => "window",
            Page::Cursor => "cursor",
            Page::Tabs => "tabs",
            Page::Quick => "quick",
            Page::Greeting => "greeting",
            Page::Rules => "rules",
            Page::Keys => "keys",
            Page::Terminal => "terminal",
        }
    }
}

/// A named choice shown in a pick list.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Choice(&'static str);

impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

const COLOR_KEYS: [(&str, &str); 5] = [
    ("foreground", "Text"),
    ("background", "Background"),
    ("cursor", "Cursor"),
    ("selection", "Selection"),
    ("accent", "Accent"),
];

#[derive(Clone, Debug)]
enum Message {
    Page(Page),
    Family(String),
    Size(f32),
    LineHeight(f32),
    Ligatures(bool),
    UseTheme(String),
    NewTheme,
    OpenThemes,
    TabPlace(Choice),
    TabLook(Choice),
    TabShowing(Choice),
    TabTitle(String),
    Divide(Choice),
    Dim(f32),
    Radius(f32),
    BadgesShow(Choice),
    BadgeMark(&'static str, String),
    Alphabet(String),
    Apolline(bool),
    ColorDraft(&'static str, String),
    ColorReset(&'static str),
    PadX(f32),
    PadY(f32),
    Opacity(f32),
    Decorations(Choice),
    Columns(f32),
    Lines(f32),
    Shape(CursorShape),
    Blink(bool),
    ComboDraft(String),
    Record,
    Slot(usize),
    SlotDraft(String),
    SlotReset,
    ActionDraft(Choice),
    AddKey,
    RemoveKey(String),
    Shell(String),
    Scrollback(f32),
    Term(Choice),
    Osc52(Choice),
    Kitty(bool),
    Integration(bool),
    NotifyAfter(f32),
    ArchiveDays(f32),
    Sessions(bool),
    OptionAsAlt(Choice),
    Fix(usize),
    FixAll,
    Undo,
    Redo,
    Open,
    Tick,
    Families(Vec<String>),
    Key(keyboard::Event),
    More(more::Change),
    Greet(greeting::Change),
}

struct Settings {
    path: PathBuf,
    editor: Result<Editor, String>,
    config: Config,
    diagnostics: Vec<Diagnostic>,
    page: Page,
    changed: Option<Range<usize>>,
    undo: Vec<String>,
    redo: Vec<String>,
    families: Vec<String>,
    drafts: HashMap<&'static str, String>,
    combo: String,
    action: Option<Choice>,
    recording: bool,
    slot: usize,
    slot_draft: Option<String>,
    outside: Option<Rgb>,
    modified: Option<SystemTime>,
    error: Option<String>,
    gallery: Vec<gallery::Card>,
    gallery_stamp: Option<SystemTime>,
}

fn modified(path: &PathBuf) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl Settings {
    fn new(page: Page) -> (Self, Task<Message>) {
        let path = vif_config::path();
        let editor = Editor::open(&path);
        let mut settings = Self {
            modified: modified(&path),
            path,
            editor,
            config: Config::default(),
            diagnostics: Vec::new(),
            page,
            changed: None,
            undo: Vec::new(),
            redo: Vec::new(),
            families: vec![vif_config::DEFAULT_FAMILY.to_owned()],
            drafts: HashMap::new(),
            combo: String::new(),
            action: None,
            recording: false,
            slot: 0,
            slot_draft: None,
            outside: vif_config::apolline_accent(),
            error: None,
            gallery: gallery::cards(),
            gallery_stamp: gallery::stamp(),
        };
        settings.reparse();
        let families = Task::perform(async { monospace_families() }, Message::Families);
        (settings, families)
    }

    fn reparse(&mut self) {
        if let Ok(editor) = &self.editor {
            let (config, diagnostics) = Config::parse(&editor.text());
            self.config = config;
            self.diagnostics = diagnostics;
        }
        self.drafts.clear();
    }

    fn edit(&mut self, change: impl FnOnce(&mut Editor)) {
        let Ok(editor) = &mut self.editor else {
            return;
        };
        let before = editor.text();
        change(editor);
        if editor.text() == before {
            return;
        }
        match editor.save() {
            Ok(range) => {
                self.undo.push(before);
                self.redo.clear();
                self.changed = Some(range);
                self.error = None;
            }
            Err(err) => self.error = Some(format!("can't save: {err}")),
        }
        self.modified = modified(&self.path);
        self.reparse();
    }

    fn replace(&mut self, text: String) {
        match Editor::parse(&text) {
            Ok(mut editor) => {
                editor.set_path(&self.path);
                if let Err(err) = editor.save() {
                    self.error = Some(format!("can't save: {err}"));
                }
                self.editor = Ok(editor);
                self.changed = None;
                self.modified = modified(&self.path);
                self.reparse();
            }
            Err(err) => self.error = Some(err),
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Page(page) => self.page = page,
            Message::More(change) => self.change(change),
            Message::Greet(change) => return self.greet(change),
            Message::Family(family) => self.edit(|e| e.set("font", "family", family)),
            Message::Size(v) => self.edit(|e| e.set_float("font", "size", v)),
            Message::LineHeight(v) => self.edit(|e| e.set_float("font", "line_height", v)),
            Message::Ligatures(v) => self.edit(|e| e.set("font", "ligatures", v)),
            Message::UseTheme(name) => self.edit(|e| e.set_top("theme", name)),
            Message::NewTheme => {
                let from = self.config.theme.clone();
                let base = format!("{}-mine", from.trim_end_matches("-mine"));
                let name = (1..)
                    .map(|n| if n == 1 { base.clone() } else { format!("{base}-{n}") })
                    .find(|n| !themes::dir().join(format!("{n}.toml")).exists())
                    .unwrap_or(base);
                match themes::create(&name, &from) {
                    Ok(path) => {
                        self.edit(|e| e.set_top("theme", name.as_str()));
                        self.gallery = gallery::cards();
                        let _ = open::that_detached(path);
                    }
                    Err(err) => self.error = Some(err),
                }
            }
            Message::OpenThemes => {
                let _ = std::fs::create_dir_all(themes::dir());
                let _ = open::that_detached(themes::dir());
            }
            Message::TabPlace(Choice(v)) => self.edit(|e| e.set("tabs", "position", v)),
            Message::TabLook(Choice(v)) => self.edit(|e| e.set("tabs", "style", v)),
            Message::TabShowing(Choice(v)) => self.edit(|e| e.set("tabs", "show", v)),
            Message::TabTitle(v) => {
                self.drafts.insert("tab_title", v.clone());
                self.edit(|e| e.set("tabs", "title", v));
            }
            Message::Divide(Choice(v)) => self.edit(|e| e.set("panes", "divider", v)),
            Message::Dim(v) => self.edit(|e| e.set_float("panes", "dim", v)),
            Message::Radius(v) => self.edit(|e| e.set_float("window", "radius", v)),
            Message::BadgesShow(Choice(v)) => self.edit(|e| e.set("badges", "show", v)),
            Message::BadgeMark(key, v) => {
                if v.chars().count() <= 4 {
                    self.edit(|e| e.set("badges", key, v));
                }
            }
            Message::Alphabet(v) => {
                self.drafts.insert("alphabet", v.clone());
                let mut letters: Vec<char> = v.chars().collect();
                letters.sort_unstable();
                letters.dedup();
                let valid = letters.len() == v.chars().count()
                    && letters.len() >= 2
                    && v.chars().all(|c| c.is_ascii_lowercase());
                if valid {
                    self.edit(|e| e.set("hints", "alphabet", v));
                }
            }
            Message::Apolline(v) => self.edit(|e| e.set("colors", "apolline", v)),
            Message::ColorDraft(key, draft) => {
                if let Some(color) = Rgb::parse(draft.trim()) {
                    self.edit(|e| e.set_color("colors", key, Some(color)));
                }
                self.drafts.insert(key, draft);
            }
            Message::ColorReset(key) => self.edit(|e| e.remove("colors", key)),
            Message::PadX(v) => {
                let y = self.config.window.padding[1];
                self.edit(|e| e.set_padding(v.round() as u16, y));
            }
            Message::PadY(v) => {
                let x = self.config.window.padding[0];
                self.edit(|e| e.set_padding(x, v.round() as u16));
            }
            Message::Opacity(v) => self.edit(|e| e.set_float("window", "opacity", v)),
            Message::Decorations(Choice(v)) => self.edit(|e| e.set("window", "decorations", v)),
            Message::Columns(v) => self.edit(|e| e.set("window", "columns", v.round() as i64)),
            Message::Lines(v) => self.edit(|e| e.set("window", "lines", v.round() as i64)),
            Message::Shape(shape) => {
                let name = match shape {
                    CursorShape::Block => "block",
                    CursorShape::Beam => "beam",
                    CursorShape::Underline => "underline",
                };
                self.edit(|e| e.set("cursor", "shape", name));
            }
            Message::Blink(v) => self.edit(|e| e.set("cursor", "blink", v)),
            Message::ComboDraft(v) => self.combo = v,
            Message::Record => self.recording = !self.recording,
            Message::Slot(i) => {
                self.slot = i;
                self.slot_draft = None;
            }
            Message::SlotDraft(draft) => {
                if let Some(color) = Rgb::parse(draft.trim()) {
                    let mut colors = self.config.palette().ansi;
                    colors[self.slot] = color;
                    self.edit(|e| e.set_palette(Some(&colors)));
                }
                self.slot_draft = Some(draft);
            }
            Message::SlotReset => {
                let theme = self.config.colors.base.ansi;
                let mut colors = self.config.palette().ansi;
                colors[self.slot] = theme[self.slot];
                let custom = colors != theme;
                self.edit(|e| e.set_palette(custom.then_some(&colors)));
                self.slot_draft = None;
            }
            Message::ActionDraft(v) => self.action = Some(v),
            Message::AddKey => {
                let combo = vif_config::normalize_combo(&self.combo);
                if let Some(Choice(action)) = self.action.clone()
                    && combo.split('+').next_back().is_some_and(|k| !k.is_empty())
                {
                    self.edit(|e| e.set("keys", &combo, action));
                    self.combo.clear();
                }
            }
            Message::RemoveKey(combo) => {
                let default = vif_config::default_keys()
                    .iter()
                    .any(|(c, _)| vif_config::normalize_combo(c) == combo);
                let own = self
                    .config
                    .keys
                    .iter()
                    .find(|(c, _)| vif_config::normalize_combo(c) == combo)
                    .map(|(c, _)| c.clone());
                self.edit(|e| {
                    if let Some(own) = &own {
                        e.remove("keys", own);
                    }
                    if default {
                        e.set("keys", &combo, "none");
                    }
                });
            }
            Message::Shell(v) => {
                if v.trim().is_empty() {
                    self.edit(|e| e.remove("terminal", "shell"));
                } else {
                    self.edit(|e| e.set("terminal", "shell", v));
                }
            }
            Message::Scrollback(v) => self.edit(|e| e.set("terminal", "scrollback", v.round() as i64)),
            Message::Term(Choice(v)) => self.edit(|e| e.set("terminal", "term", v)),
            Message::Osc52(Choice(v)) => self.edit(|e| e.set("terminal", "osc52", v)),
            Message::Kitty(v) => self.edit(|e| e.set("terminal", "kitty_keyboard", v)),
            Message::Integration(v) => self.edit(|e| e.set("terminal", "shell_integration", v)),
            Message::NotifyAfter(v) => self.edit(|e| e.set("terminal", "notify_after", v.round() as i64)),
            Message::Sessions(v) => self.edit(|e| e.set("terminal", "sessions", v)),
            Message::ArchiveDays(v) => self.edit(|e| e.set("terminal", "archive_days", v.round() as i64)),
            Message::OptionAsAlt(Choice(v)) => self.edit(|e| e.set("terminal", "option_as_alt", v)),
            Message::Fix(i) => {
                if let (Some(fix), Ok(editor)) = (self.diagnostics.get(i).and_then(|d| d.fix.clone()), &self.editor) {
                    let before = editor.text();
                    if let Some(text) = fix.apply(&before) {
                        self.undo.push(before);
                        self.replace(text);
                    }
                }
            }
            Message::FixAll => {
                if let Ok(editor) = &self.editor {
                    let before = editor.text();
                    let mut fixes: Vec<_> = self.diagnostics.iter().filter_map(|d| d.fix.clone()).collect();
                    fixes.sort_by_key(|f| std::cmp::Reverse(f.span.start));
                    let text = fixes.iter().try_fold(before.clone(), |text, fix| fix.apply(&text));
                    if let Some(text) = text {
                        self.undo.push(before);
                        self.replace(text);
                    }
                }
            }
            Message::Undo => {
                if let (Some(text), Ok(editor)) = (self.undo.pop(), &self.editor) {
                    self.redo.push(editor.text());
                    self.replace(text);
                }
            }
            Message::Redo => {
                if let (Some(text), Ok(editor)) = (self.redo.pop(), &self.editor) {
                    self.undo.push(editor.text());
                    self.replace(text);
                }
            }
            Message::Open => {
                if !self.path.exists() {
                    let _ = std::fs::create_dir_all(self.path.parent().unwrap_or(&self.path));
                    let _ = std::fs::write(&self.path, "");
                }
                let _ = open::that_detached(&self.path);
            }
            Message::Tick => {
                self.outside = vif_config::apolline_accent();
                let stamp = gallery::stamp();
                if stamp != self.gallery_stamp {
                    self.gallery_stamp = stamp;
                    self.gallery = gallery::cards();
                    self.reparse();
                }
                let now = modified(&self.path);
                if now != self.modified {
                    self.modified = now;
                    self.editor = Editor::open(&self.path);
                    self.changed = None;
                    self.reparse();
                }
            }
            Message::Families(list) => self.families = list,
            Message::Key(keyboard::Event::KeyPressed {
                key,
                physical_key,
                modifiers,
                ..
            }) if self.recording => {
                if key == keyboard::Key::Named(keyboard::key::Named::Escape) {
                    self.recording = false;
                } else if let Some(combo) = recorded(&key, physical_key, modifiers) {
                    self.combo = combo;
                    self.recording = false;
                }
            }
            Message::Key(keyboard::Event::KeyPressed { key, modifiers, .. }) => {
                let undo = matches!(key.as_ref(), keyboard::Key::Character("z" | "я")) && modifiers.command();
                if undo && modifiers.shift() {
                    return self.update(Message::Redo);
                }
                if undo {
                    return self.update(Message::Undo);
                }
            }
            Message::Key(_) => {}
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            iced::time::every(Duration::from_millis(500)).map(|_| Message::Tick),
            keyboard::listen().map(Message::Key),
        ])
    }

    fn palette(&self) -> Palette {
        self.config.palette_with(self.outside)
    }

    fn accent(&self) -> Rgb {
        self.palette().accent
    }

    fn view(&self) -> Element<'_, Message> {
        responsive(move |size| self.layout(size.width)).into()
    }

    /// Three columns when there is room, the written lines under the form when there isn't.
    fn layout(&self, width: f32) -> Element<'_, Message> {
        let accent = self.accent();
        let wide = width >= 1120.0;
        let narrow = width < 860.0;
        let mut nav = Column::new()
            .spacing(2)
            .padding([18, 10])
            .width(if narrow { 150 } else { 196 });
        nav = nav
            .push(container(text("vif").size(20).font(style::mono_bold()).color(style::rgb(accent))).padding([4, 12]));
        nav = nav.push(space::vertical().height(14));
        for page in Page::ALL {
            nav = nav.push(
                button(text(page.title()).size(14).font(style::medium()))
                    .width(Fill)
                    .padding([9, 12])
                    .style(style::nav(page == self.page, accent))
                    .on_press(Message::Page(page)),
            );
        }
        let sidebar = container(nav).height(Fill).style(style::sidebar);

        let title = column![
            text(self.page.title()).size(26).font(style::semibold()),
            text(self.page.subtitle()).size(14).color(style::faint()),
        ]
        .spacing(4)
        .width(Fill);
        let header: Element<'_, Message> = if narrow {
            column![title, self.history()].spacing(10).into()
        } else {
            row![title, self.history()].align_y(Alignment::Start).into()
        };

        let preview = container(
            shader(Preview::new(&self.config, self.palette()))
                .width(Fill)
                .height(Length::Fixed(Preview::height(&self.config))),
        )
        .style(style::frame);

        let mut body = column![header].spacing(22);
        if let Err(err) = &self.editor {
            body = body.push(style::banner(text(err.clone()).size(13).color(style::warn()).into()));
        }
        if !self.diagnostics.is_empty() {
            body = body.push(self.problems());
        }
        body = body.push(preview).push(self.form());
        if !wide {
            body = body.push(self.writes(false));
        }
        let pad = if narrow { [22, 20] } else { [28, 32] };
        let content = scrollable(container(body).padding(pad).width(Fill))
            .height(Fill)
            .width(Fill);

        let mut columns = row![sidebar, content].height(Fill);
        if wide {
            columns = columns.push(self.writes(true));
        }
        container(columns).style(style::window).width(Fill).height(Fill).into()
    }

    fn history(&self) -> Element<'_, Message> {
        let item = |label, enabled: bool, message| {
            button(text(label).size(13))
                .padding([7, 12])
                .style(style::ghost)
                .on_press_maybe(enabled.then_some(message))
        };
        row![
            item("Undo", !self.undo.is_empty(), Message::Undo),
            item("Redo", !self.redo.is_empty(), Message::Redo),
            item("Open file", true, Message::Open),
        ]
        .spacing(4)
        .into()
    }

    fn problems(&self) -> Element<'_, Message> {
        let n = self.diagnostics.len();
        let fixable = self.diagnostics.iter().filter(|d| d.fix.is_some()).count();
        let mut list = Column::new().spacing(8);
        let mut head = row![
            text(format!(
                "vif.toml has {n} problem{}. Everything else is applied.",
                if n == 1 { "" } else { "s" }
            ))
            .size(14)
            .font(style::semibold())
            .color(style::warn())
            .width(Fill)
        ]
        .align_y(Alignment::Center);
        if fixable > 1 {
            head = head.push(
                button(text("Fix all").size(13))
                    .padding([6, 12])
                    .style(style::warn_button)
                    .on_press(Message::FixAll),
            );
        }
        list = list.push(head);
        for (i, d) in self.diagnostics.iter().enumerate() {
            let mut line = row![
                text(format!("line {}", d.line))
                    .size(13)
                    .font(MONO)
                    .color(style::faint())
                    .width(64),
                text(d.to_string()).size(13).width(Fill),
            ]
            .spacing(10)
            .align_y(Alignment::Center);
            if d.fix.is_some() {
                line = line.push(
                    button(text("Fix").size(13))
                        .padding([4, 12])
                        .style(style::ghost)
                        .on_press(Message::Fix(i)),
                );
            }
            list = list.push(line);
        }
        style::banner(list.into())
    }

    fn form(&self) -> Element<'_, Message> {
        let c = &self.config;
        let accent = self.accent();
        match self.page {
            Page::Themes => self.themes(),
            Page::Tabs => self.tabs(),
            Page::Effects => self.effects_page(),
            Page::Quick => self.quick_page(),
            Page::Greeting => self.greeting_page(),
            Page::Rules => self.rules_page(),
            Page::Font => {
                let family = pick_list(self.families.clone(), Some(c.font.family.clone()), Message::Family)
                    .width(300)
                    .style(style::pick)
                    .menu_style(style::menu);
                column![
                    field("Family", family.into(), Some("Geist Mono ships with vif")),
                    field(
                        "Size",
                        ranged(
                            slider(6.0f32..=36.0, c.font.size, Message::Size).step(0.5f32),
                            format!("{:.1}", c.font.size),
                            accent
                        ),
                        None,
                    ),
                    field(
                        "Line height",
                        ranged(
                            slider(0.8f32..=2.0, c.font.line_height, Message::LineHeight).step(0.05f32),
                            format!("{:.2}", c.font.line_height),
                            accent,
                        ),
                        None,
                    ),
                    field(
                        "Ligatures",
                        toggler(c.font.ligatures)
                            .on_toggle(Message::Ligatures)
                            .style(style::toggle(accent))
                            .into(),
                        Some("joins != => >= into one glyph when the font has them"),
                    ),
                ]
                .extend(self.font_rows())
                .spacing(18)
                .into()
            }
            Page::Colors => {
                let mut col = column![field(
                    "Theme",
                    row![
                        text(c.theme.clone()).size(14).font(MONO),
                        button(text("All themes").size(12))
                            .padding([5, 10])
                            .style(style::ghost)
                            .on_press(Message::Page(Page::Themes)),
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center)
                    .into(),
                    None,
                )]
                .spacing(14);
                col = col.push(self.colors_from());
                let mut follow = row![
                    toggler(c.colors.apolline)
                        .on_toggle(Message::Apolline)
                        .style(style::toggle(accent))
                ]
                .spacing(12)
                .align_y(Alignment::Center);
                follow = match self.outside {
                    Some(color) => follow
                        .push(
                            container(space::horizontal().width(14))
                                .height(14)
                                .style(style::swatch(color)),
                        )
                        .push(text(color.to_hex()).size(13).font(MONO).color(style::faint())),
                    None => follow.push(text("Apolline isn't coloring apps now").size(13).color(style::faint())),
                };
                col = col.push(field(
                    "Follow Apolline",
                    follow.into(),
                    Some("its accent while it runs; colors you set here still win"),
                ));
                let palette = self.palette();
                for (key, label) in COLOR_KEYS {
                    let set = match key {
                        "foreground" => c.colors.foreground,
                        "background" => c.colors.background,
                        "cursor" => c.colors.cursor,
                        "selection" => c.colors.selection,
                        _ => c.colors.accent,
                    };
                    let shown = match key {
                        "foreground" => palette.foreground,
                        "background" => palette.background,
                        "cursor" => palette.cursor,
                        "selection" => palette.selection,
                        _ => palette.accent,
                    };
                    let draft = self
                        .drafts
                        .get(key)
                        .cloned()
                        .unwrap_or_else(|| set.map(Rgb::to_hex).unwrap_or_default());
                    let mut line = row![
                        container(space::horizontal().width(22))
                            .height(22)
                            .style(style::swatch(shown)),
                        text_input(&shown.to_hex(), &draft)
                            .on_input(move |v| Message::ColorDraft(key, v))
                            .font(MONO)
                            .size(13)
                            .width(120)
                            .style(style::input),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center);
                    if set.is_some() {
                        line = line.push(
                            button(text("From theme").size(12))
                                .padding([5, 10])
                                .style(style::ghost)
                                .on_press(Message::ColorReset(key)),
                        );
                    }
                    col = col.push(field(label, line.into(), None));
                }
                let theme = c.colors.base.ansi;
                let mut swatches = Column::new().spacing(6);
                for half in [0..8, 8..16] {
                    let mut line = Row::new().spacing(6);
                    for i in half {
                        line = line.push(
                            button(space::horizontal().width(18).height(18))
                                .padding(0)
                                .style(style::slot(palette.ansi[i], i == self.slot, accent))
                                .on_press(Message::Slot(i)),
                        );
                    }
                    swatches = swatches.push(line);
                }
                let slot = palette.ansi[self.slot];
                let draft = self.slot_draft.clone().unwrap_or_else(|| slot.to_hex());
                let mut edit = row![
                    text(format!("{:>2}", self.slot))
                        .size(13)
                        .font(MONO)
                        .color(style::faint()),
                    text_input(&slot.to_hex(), &draft)
                        .on_input(Message::SlotDraft)
                        .font(MONO)
                        .size(13)
                        .width(120)
                        .style(style::input),
                ]
                .spacing(10)
                .align_y(Alignment::Center);
                if slot != theme[self.slot] {
                    edit = edit.push(
                        button(text("From theme").size(12))
                            .padding([5, 10])
                            .style(style::ghost)
                            .on_press(Message::SlotReset),
                    );
                }
                col = col.push(field(
                    "Palette",
                    column![swatches, edit].spacing(12).into(),
                    Some("the 16 colors programs ask for by number"),
                ));
                col.into()
            }
            Page::Window => {
                let decorations = [Choice("full"), Choice("transparent"), Choice("none")];
                let current = match c.window.decorations {
                    Decorations::Full => "full",
                    Decorations::Transparent => "transparent",
                    Decorations::None => "none",
                };
                column![
                    field(
                        "Padding left and right",
                        ranged(
                            slider(0.0f32..=64.0, f32::from(c.window.padding[0]), Message::PadX),
                            format!("{} px", c.window.padding[0]),
                            accent
                        ),
                        None,
                    ),
                    field(
                        "Padding top and bottom",
                        ranged(
                            slider(0.0f32..=64.0, f32::from(c.window.padding[1]), Message::PadY),
                            format!("{} px", c.window.padding[1]),
                            accent
                        ),
                        None,
                    ),
                    field(
                        "Opacity",
                        ranged(
                            slider(0.3f32..=1.0, c.window.opacity, Message::Opacity).step(0.01f32),
                            format!("{:.0}%", c.window.opacity * 100.0),
                            accent,
                        ),
                        Some("the background only; text stays solid"),
                    ),
                    self.blur_row(),
                    field(
                        "Frame",
                        pick_list(decorations, Some(Choice(current)), Message::Decorations)
                            .width(200)
                            .style(style::pick)
                            .menu_style(style::menu)
                            .into(),
                        Some("transparent puts the tabs in the title bar"),
                    ),
                    field(
                        "New window width",
                        ranged(
                            slider(40.0f32..=300.0, f32::from(c.window.columns), Message::Columns),
                            format!("{} columns", c.window.columns),
                            accent
                        ),
                        None,
                    ),
                    field(
                        "New window height",
                        ranged(
                            slider(10.0f32..=120.0, f32::from(c.window.lines), Message::Lines),
                            format!("{} lines", c.window.lines),
                            accent
                        ),
                        None,
                    ),
                ]
                .spacing(18)
                .into()
            }
            Page::Cursor => {
                let shape = |label, value: CursorShape| {
                    button(text(label).size(13))
                        .padding([8, 16])
                        .style(style::choice(c.cursor.shape == value, accent))
                        .on_press(Message::Shape(value))
                };
                column![
                    field(
                        "Shape",
                        row![
                            shape("Block", CursorShape::Block),
                            shape("Beam", CursorShape::Beam),
                            shape("Underline", CursorShape::Underline)
                        ]
                        .spacing(8)
                        .into(),
                        Some("programs like vim may change it while they run"),
                    ),
                    field(
                        "Blink",
                        toggler(c.cursor.blink)
                            .on_toggle(Message::Blink)
                            .style(style::toggle(accent))
                            .into(),
                        None
                    ),
                ]
                .spacing(18)
                .into()
            }
            Page::Keys => self.keys(),
            Page::Terminal => {
                let terms = [Choice("xterm-vif"), Choice("xterm-256color")];
                let osc = [Choice("off"), Choice("copy"), Choice("paste"), Choice("both")];
                let alt = [Choice("none"), Choice("left"), Choice("right"), Choice("both")];
                let term = if c.terminal.term == "xterm-256color" {
                    "xterm-256color"
                } else {
                    "xterm-vif"
                };
                let osc_now = match c.terminal.osc52 {
                    Osc52::Off => "off",
                    Osc52::Copy => "copy",
                    Osc52::Paste => "paste",
                    Osc52::Both => "both",
                };
                let alt_now = match c.terminal.option_as_alt {
                    OptionAsAlt::None => "none",
                    OptionAsAlt::Left => "left",
                    OptionAsAlt::Right => "right",
                    OptionAsAlt::Both => "both",
                };
                column![
                    field(
                        "Shell",
                        text_input("your login shell", c.terminal.shell.as_deref().unwrap_or(""))
                            .on_input(Message::Shell)
                            .font(MONO)
                            .size(13)
                            .width(300)
                            .style(style::input)
                            .into(),
                        None,
                    ),
                    field(
                        "Scrollback",
                        ranged(
                            slider(0.0f32..=100_000.0, c.terminal.scrollback as f32, Message::Scrollback)
                                .step(1000.0f32),
                            format!("{} lines", c.terminal.scrollback),
                            accent,
                        ),
                        None,
                    ),
                    field(
                        "TERM",
                        pick_list(terms, Some(Choice(term)), Message::Term)
                            .width(200)
                            .style(style::pick)
                            .menu_style(style::menu)
                            .into(),
                        Some("xterm-256color is safer over ssh")
                    ),
                    field(
                        "Clipboard from programs",
                        pick_list(osc, Some(Choice(osc_now)), Message::Osc52)
                            .width(200)
                            .style(style::pick)
                            .menu_style(style::menu)
                            .into(),
                        Some("OSC 52: copy is safe, paste lets programs read it")
                    ),
                    field(
                        "Kitty keyboard protocol",
                        toggler(c.terminal.kitty_keyboard)
                            .on_toggle(Message::Kitty)
                            .style(style::toggle(accent))
                            .into(),
                        None
                    ),
                    field(
                        "Shell integration",
                        toggler(c.terminal.shell_integration)
                            .on_toggle(Message::Integration)
                            .style(style::toggle(accent))
                            .into(),
                        Some("marks prompts in zsh and bash; new panes pick it up")
                    ),
                    field(
                        "Notify after",
                        ranged(
                            slider(0.0f32..=120.0, c.terminal.notify_after as f32, Message::NotifyAfter).step(5.0f32),
                            if c.terminal.notify_after == 0 {
                                "never".to_owned()
                            } else {
                                format!("{} s", c.terminal.notify_after)
                            },
                            accent,
                        ),
                        Some("when a command this long ends out of sight")
                    ),
                    field(
                        "Sessions outlive windows",
                        toggler(c.terminal.sessions)
                            .on_toggle(Message::Sessions)
                            .style(style::toggle(accent))
                            .into(),
                        Some("closing a window keeps its shells running; the next vif brings them back")
                    ),
                    field(
                        "Keep commands",
                        ranged(
                            slider(0.0f32..=90.0, c.terminal.archive_days as f32, Message::ArchiveDays).step(1.0f32),
                            if c.terminal.archive_days == 0 {
                                "off".to_owned()
                            } else {
                                format!("{} days", c.terminal.archive_days)
                            },
                            accent,
                        ),
                        Some("each command and its output, on this machine only, for ⇧⌘F and vif history")
                    ),
                    field(
                        "Command badges",
                        row![
                            pick_list(
                                [Choice("all"), Choice("failures"), Choice("none")],
                                Some(Choice(match c.badges.show {
                                    BadgeShow::All => "all",
                                    BadgeShow::Failures => "failures",
                                    BadgeShow::None => "none",
                                })),
                                Message::BadgesShow
                            )
                            .width(130)
                            .style(style::pick)
                            .menu_style(style::menu),
                            text_input("✓", &c.badges.ok)
                                .on_input(|v| Message::BadgeMark("ok", v))
                                .font(MONO)
                                .size(13)
                                .width(52)
                                .style(style::input),
                            text_input("✗", &c.badges.failed)
                                .on_input(|v| Message::BadgeMark("failed", v))
                                .font(MONO)
                                .size(13)
                                .width(52)
                                .style(style::input),
                            text_input("●", &c.badges.running)
                                .on_input(|v| Message::BadgeMark("running", v))
                                .font(MONO)
                                .size(13)
                                .width(52)
                                .style(style::input),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center)
                        .into(),
                        Some("exit code and time at the end of a command line; marks for done, failed, running")
                    ),
                    field(
                        "Hint letters",
                        text_input(
                            "asdfghjkl",
                            &self
                                .drafts
                                .get("alphabet")
                                .cloned()
                                .unwrap_or_else(|| c.hints.alphabet.clone())
                        )
                        .on_input(Message::Alphabet)
                        .font(MONO)
                        .size(13)
                        .width(300)
                        .style(style::input)
                        .into(),
                        Some("the easiest first; small latin letters, none twice")
                    ),
                    field(
                        "Option key as Alt",
                        pick_list(alt, Some(Choice(alt_now)), Message::OptionAsAlt)
                            .width(200)
                            .style(style::pick)
                            .menu_style(style::menu)
                            .into(),
                        Some("macOS only")
                    ),
                ]
                .push(self.clock_row())
                .spacing(18)
                .into()
            }
        }
    }

    fn tabs(&self) -> Element<'_, Message> {
        let c = &self.config;
        let accent = self.accent();
        let pick = |options: Vec<Choice>, now: &'static str, message: fn(Choice) -> Message| -> Element<'_, Message> {
            pick_list(options, Some(Choice(now)), message)
                .width(200)
                .style(style::pick)
                .menu_style(style::menu)
                .into()
        };
        let place = match c.tabs.position {
            TabPosition::Top => "top",
            TabPosition::Bottom => "bottom",
        };
        let look = match c.tabs.style {
            TabStyle::Pills => "pills",
            TabStyle::Underline => "underline",
            TabStyle::Powerline => "powerline",
            TabStyle::Plain => "plain",
        };
        let showing = match c.tabs.show {
            TabShow::Many => "many",
            TabShow::Always => "always",
            TabShow::Never => "never",
        };
        let divider = match c.panes.divider {
            Divider::Line => "line",
            Divider::Gap => "gap",
        };
        let title = self
            .drafts
            .get("tab_title")
            .cloned()
            .unwrap_or_else(|| c.tabs.title.clone());
        column![
            field(
                "Tabs sit",
                pick(vec![Choice("top"), Choice("bottom")], place, Message::TabPlace),
                None
            ),
            field(
                "Tab style",
                pick(
                    vec![
                        Choice("pills"),
                        Choice("underline"),
                        Choice("powerline"),
                        Choice("plain")
                    ],
                    look,
                    Message::TabLook
                ),
                None
            ),
            field(
                "Show the bar",
                pick(
                    vec![Choice("many"), Choice("always"), Choice("never")],
                    showing,
                    Message::TabShowing
                ),
                Some("many: only once there is a second tab")
            ),
            field(
                "Tab title",
                text_input("{index} {title}", &title)
                    .on_input(Message::TabTitle)
                    .font(MONO)
                    .size(13)
                    .width(260)
                    .style(style::input)
                    .into(),
                Some("{index}, {title}, {cwd} and {dir} are filled in")
            ),
            field(
                "Corners",
                ranged(
                    slider(0.0f32..=16.0, c.window.radius, Message::Radius).step(1.0f32),
                    format!("{}", c.window.radius),
                    accent,
                ),
                Some("tabs, panels and hint labels; 0 is square")
            ),
            field(
                "Between splits",
                pick(vec![Choice("line"), Choice("gap")], divider, Message::Divide),
                None
            ),
            field(
                "Fade other splits",
                ranged(
                    slider(0.0f32..=0.9, c.panes.dim, Message::Dim).step(0.01f32),
                    format!("{:.0}%", c.panes.dim * 100.0),
                    accent,
                ),
                None
            ),
        ]
        .extend(self.tab_rows())
        .spacing(18)
        .into()
    }

    fn keys(&self) -> Element<'_, Message> {
        let accent = self.accent();
        let mut bindings = self.config.bindings();
        bindings.sort_by(|a, b| {
            let rank = |action: &str| ACTIONS.iter().position(|x| *x == action).unwrap_or(usize::MAX);
            rank(&a.1).cmp(&rank(&b.1))
        });
        let own: BTreeSet<String> = self
            .config
            .keys
            .iter()
            .map(|(c, _)| vif_config::normalize_combo(c))
            .collect();
        let mut list = Column::new().spacing(4);
        for (combo, action) in bindings {
            let mine = own.contains(&combo);
            list = list.push(
                row![
                    text(action.replace('_', " ")).size(13).width(Fill),
                    container(text(pretty_combo(&combo)).size(13).font(MONO).color(if mine {
                        style::rgb(accent)
                    } else {
                        style::text()
                    }))
                    .padding([4, 10])
                    .style(style::keycap),
                    button(text("Remove").size(12))
                        .padding([4, 10])
                        .style(style::ghost)
                        .on_press(Message::RemoveKey(combo.clone())),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            );
        }
        let actions: Vec<Choice> = ACTIONS.iter().map(|a| Choice(a)).collect();
        let add = row![
            button(text(if self.recording { "Press keys…" } else { "Record" }).size(13))
                .padding([7, 12])
                .style(style::choice(self.recording, accent))
                .on_press(Message::Record),
            text_input(
                if cfg!(target_os = "macos") {
                    "cmd+shift+k"
                } else {
                    "ctrl+shift+k"
                },
                &self.combo
            )
            .on_input(Message::ComboDraft)
            .on_submit(Message::AddKey)
            .font(MONO)
            .size(13)
            .width(180)
            .style(style::input),
            pick_list(actions, self.action.clone(), Message::ActionDraft)
                .placeholder("action")
                .width(200)
                .style(style::pick)
                .menu_style(style::menu),
            button(text("Add").size(13))
                .padding([7, 14])
                .style(style::choice(true, accent))
                .on_press(Message::AddKey),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        column![add, container(list).padding(14).style(style::card)]
            .spacing(16)
            .into()
    }

    fn writes(&self, side: bool) -> Element<'_, Message> {
        let accent = self.accent();
        let section = self.page.section();
        let lines = match &self.editor {
            Ok(editor) => editor.section(section),
            Err(_) => Vec::new(),
        };
        let mut code = Column::new().spacing(2);
        if lines.is_empty() {
            code = code.push(
                text("# nothing set yet, defaults apply")
                    .size(13)
                    .font(MONO)
                    .color(style::faint()),
            );
        }
        for (n, line) in &lines {
            let hit = self.changed.as_ref().is_some_and(|r| r.contains(n));
            let content = row![
                text(format!("{n:>3}"))
                    .size(12)
                    .font(MONO)
                    .color(if hit { style::rgb(accent) } else { style::faint() })
                    .width(30),
                text(line.clone()).size(13).font(MONO).color(syntax(line)),
            ]
            .spacing(10);
            code = code.push(
                container(content)
                    .padding([2, 8])
                    .width(Fill)
                    .style(style::line(hit, accent)),
            );
        }
        let mut panel = column![
            row![
                text("vif.toml").size(13).font(style::medium()).width(Fill),
                text("live").size(11).color(style::good()),
            ]
            .align_y(Alignment::Center),
            code,
            text("Saved as you change it, and every open vif window reloads. Your comments and order stay.")
                .size(12)
                .color(style::faint()),
            text(self.path.display().to_string())
                .size(11)
                .font(MONO)
                .color(style::faint()),
        ]
        .spacing(14);
        if let Some(err) = &self.error {
            panel = panel.push(text(err.clone()).size(12).color(style::warn()));
        }
        if side {
            container(scrollable(panel))
                .width(Length::Fixed(340.0))
                .height(Fill)
                .padding(20)
                .style(style::writes)
                .into()
        } else {
            container(panel).width(Fill).padding(18).style(style::card).into()
        }
    }
}

fn field<'a>(label: &'a str, control: Element<'a, Message>, hint: Option<&'a str>) -> Element<'a, Message> {
    let mut left = Column::new().spacing(2).width(190);
    left = left.push(text(label).size(14).font(style::medium()));
    if let Some(hint) = hint {
        left = left.push(text(hint).size(12).color(style::faint()));
    }
    Row::new()
        .push(left)
        .push(control)
        .spacing(20)
        .align_y(Alignment::Center)
        .wrap()
        .vertical_spacing(8)
        .into()
}

fn ranged<'a>(slider: iced::widget::Slider<'a, f32, Message>, value: String, accent: Rgb) -> Element<'a, Message> {
    row![
        slider.width(260).style(style::rail(accent)),
        text(value).size(13).font(MONO).width(110),
    ]
    .spacing(14)
    .align_y(Alignment::Center)
    .into()
}

/// A key press with at least one modifier, written the way vif.toml spells it.
fn recorded(key: &keyboard::Key, physical: keyboard::key::Physical, mods: keyboard::Modifiers) -> Option<String> {
    use keyboard::key::Named;
    let name = match key {
        keyboard::Key::Named(named) => match named {
            Named::ArrowLeft => "left".to_owned(),
            Named::ArrowRight => "right".to_owned(),
            Named::ArrowUp => "up".to_owned(),
            Named::ArrowDown => "down".to_owned(),
            Named::PageUp => "pageup".to_owned(),
            Named::PageDown => "pagedown".to_owned(),
            Named::Home => "home".to_owned(),
            Named::End => "end".to_owned(),
            Named::Tab => "tab".to_owned(),
            Named::Enter => "enter".to_owned(),
            Named::Backspace => "backspace".to_owned(),
            Named::Delete => "delete".to_owned(),
            Named::Space => "space".to_owned(),
            other => {
                let debug = format!("{other:?}");
                if debug.starts_with('F') && debug[1..].parse::<u8>().is_ok() {
                    debug.to_lowercase()
                } else {
                    return None;
                }
            }
        },
        keyboard::Key::Character(_) => key.to_latin(physical)?.to_ascii_lowercase().to_string(),
        keyboard::Key::Unidentified => return None,
    };
    let mut parts = Vec::new();
    if mods.alt() {
        parts.push("alt");
    }
    if mods.logo() {
        parts.push("cmd");
    }
    if mods.control() {
        parts.push("ctrl");
    }
    if mods.shift() {
        parts.push("shift");
    }
    (!parts.is_empty() && !(parts == ["shift"] && name.len() == 1)).then(|| format!("{}+{name}", parts.join("+")))
}

fn pretty_combo(combo: &str) -> String {
    combo
        .split('+')
        .map(|p| match p {
            "cmd" if cfg!(target_os = "macos") => "⌘".to_owned(),
            "shift" if cfg!(target_os = "macos") => "⇧".to_owned(),
            "alt" if cfg!(target_os = "macos") => "⌥".to_owned(),
            "ctrl" if cfg!(target_os = "macos") => "⌃".to_owned(),
            other if other.len() == 1 => other.to_uppercase(),
            other => {
                let mut c = other.chars();
                c.next()
                    .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                    .unwrap_or_default()
            }
        })
        .collect::<Vec<_>>()
        .join(if cfg!(target_os = "macos") { "" } else { "+" })
}

fn syntax(line: &str) -> iced::Color {
    let t = line.trim_start();
    if t.starts_with('#') {
        style::faint()
    } else if t.starts_with('[') {
        style::warn()
    } else {
        style::text()
    }
}

fn monospace_families() -> Vec<String> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let mut names: BTreeSet<String> = db
        .faces()
        .filter(|f| f.monospaced)
        .filter_map(|f| f.families.first().map(|(n, _)| n.clone()))
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.remove(vif_config::DEFAULT_FAMILY);
    let mut list = vec![vif_config::DEFAULT_FAMILY.to_owned()];
    list.extend(names);
    list
}
