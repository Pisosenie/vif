#![warn(clippy::undocumented_unsafe_blocks)]

mod ai;
mod gallery;
mod greeting;
mod more;
mod pages;
mod preview;
mod search;
mod style;
mod tabs;
mod update;

use std::collections::{BTreeSet, HashMap};
use std::ops::Range;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use iced::widget::{
    Column, Row, button, column, container, responsive, row, scrollable, shader, space, text, text_input,
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
    Ai,
    Rules,
    Keys,
    Terminal,
}

impl Page {
    const ALL: [Page; 13] = [
        Page::Themes,
        Page::Font,
        Page::Colors,
        Page::Effects,
        Page::Window,
        Page::Cursor,
        Page::Tabs,
        Page::Quick,
        Page::Greeting,
        Page::Ai,
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
            Page::Ai => "AI",
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
            Page::Ai => {
                "A model that says why a command failed, and turns words into a command. Off until you want it."
            }
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
            Page::Ai => "ai",
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
    KeysLike(Choice),
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
    Query(String),
    Greet(greeting::Change),
    Ai(ai::Change),
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
    /// What the search field above the pages holds.
    query: String,
    /// The setting the last change made, and what it was before.
    was: Option<(String, Option<String>)>,
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
            query: String::new(),
            was: None,
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
        let was = vif_config::setting_changed(&before, &editor.text());
        match editor.save() {
            Ok(range) => {
                self.was = was;
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
        self.was = None;
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
        nav = nav.push(space::vertical().height(10));
        nav = nav.push(
            text_input("Search settings", &self.query)
                .on_input(Message::Query)
                .size(13)
                .padding([7, 10])
                .style(style::input),
        );
        nav = nav.push(space::vertical().height(8));
        if self.query.trim().is_empty() {
            for page in Page::ALL {
                nav = nav.push(
                    button(text(page.title()).size(14).font(style::medium()))
                        .width(Fill)
                        .padding([9, 12])
                        .style(style::nav(page == self.page, accent))
                        .on_press(Message::Page(page)),
                );
            }
        } else {
            let hits = search::find(&self.query, 14);
            if hits.is_empty() {
                nav = nav.push(container(text("Nothing by that name").size(13).color(style::faint())).padding([9, 12]));
            }
            for hit in hits {
                let label = column![
                    text(hit.label).size(13).font(MONO),
                    text(hit.page.title()).size(11).color(style::faint()),
                ]
                .spacing(1);
                nav = nav.push(
                    button(label)
                        .width(Fill)
                        .padding([6, 12])
                        .style(style::nav(false, accent))
                        .on_press(Message::Page(hit.page)),
                );
            }
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
        let was = self.was.as_ref().map(|(key, old)| {
            let old = old.as_deref().unwrap_or("not set");
            text(format!("{key} was {old}"))
                .size(12)
                .font(MONO)
                .color(style::faint())
        });
        row![
            container(was).padding([9, 6]),
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
        match self.page {
            Page::Themes => self.themes(),
            Page::Tabs => self.tabs(),
            Page::Effects => self.effects_page(),
            Page::Quick => self.quick_page(),
            Page::Greeting => self.greeting_page(),
            Page::Ai => self.ai_page(),
            Page::Rules => self.rules_page(),
            Page::Font => self.font_page(),
            Page::Colors => self.colors_page(),
            Page::Window => self.window_page(),
            Page::Cursor => self.cursor_page(),
            Page::Keys => self.keys(),
            Page::Terminal => self.terminal_page(),
        }
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
