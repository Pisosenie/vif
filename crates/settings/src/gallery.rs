//! The Themes page: every theme as a card in its own colors, one click to switch.

use std::path::Path;
use std::time::SystemTime;

use iced::widget::{Column, Row, button, column, container, row, space, text};
use iced::{Alignment, Element, Fill};
use vif_config::themes::{self, ThemeFile};
use vif_config::{Config, Palette, Rgb, TabPosition, TabStyle};

use crate::{MONO, Message, Settings, style};

pub struct Card {
    theme: ThemeFile,
    palette: Palette,
    looks: String,
}

/// The themes as they come out of their files, before vif.toml changes anything.
pub fn cards() -> Vec<Card> {
    themes::all()
        .into_iter()
        .map(|theme| {
            let (config, _) = Config::parse(&format!("theme = \"{}\"\n", theme.name));
            Card {
                palette: config.palette(),
                looks: looks(&config),
                theme,
            }
        })
        .collect()
}

fn looks(config: &Config) -> String {
    let style = match config.tabs.style {
        TabStyle::Pills => "pills",
        TabStyle::Underline => "underline",
        TabStyle::Powerline => "powerline",
        TabStyle::Plain => "plain",
    };
    let place = match config.tabs.position {
        TabPosition::Top => "top",
        TabPosition::Bottom => "bottom",
    };
    format!("{style} tabs · {place} · corners {}", config.window.radius)
}

/// When the themes folder last changed, so the gallery knows to read it again.
pub fn stamp() -> Option<SystemTime> {
    let dir = themes::dir();
    let mut newest = std::fs::metadata(&dir).and_then(|m| m.modified()).ok();
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let time = entry.metadata().and_then(|m| m.modified()).ok();
        newest = newest.max(time);
    }
    newest
}

impl Settings {
    pub(crate) fn themes(&self) -> Element<'_, Message> {
        let accent = self.accent();
        let mut grid = Column::new().spacing(12);
        for pair in self.gallery.chunks(2) {
            let mut line = Row::new().spacing(12);
            for card in pair {
                line = line.push(self.card(card, accent));
            }
            if pair.len() == 1 {
                line = line.push(space::horizontal().width(Fill));
            }
            grid = grid.push(line);
        }
        let actions = row![
            button(text("Make my own from this one").size(13))
                .padding([8, 14])
                .style(style::choice(true, accent))
                .on_press(Message::NewTheme),
            button(text("Open themes folder").size(13))
                .padding([8, 14])
                .style(style::ghost)
                .on_press(Message::OpenThemes),
        ]
        .spacing(8);
        let note = text(
            "A theme is one file: colors, tabs, splits, corners, command badges and hint letters. \
             Make your own and every window follows as you save it. What you change on the other \
             pages goes in vif.toml and wins over the theme.",
        )
        .size(13)
        .color(style::faint());
        column![actions, note, grid].spacing(16).into()
    }

    fn card<'a>(&self, card: &'a Card, accent: Rgb) -> Element<'a, Message> {
        let p = &card.palette;
        let mono = |s: &'a str, c: Rgb| text(s).font(MONO).size(13).color(style::rgb(c));
        let mut colors = Row::new().spacing(4);
        for c in &p.ansi[1..8] {
            colors = colors.push(
                container(space::horizontal().width(14))
                    .height(10)
                    .style(style::tile(*c, 3.0)),
            );
        }
        let sample = container(
            column![
                row![
                    mono("~/src/vif", p.ansi[4]),
                    mono(" main ", p.ansi[5]),
                    mono("❯ ", p.ansi[2]),
                    mono("ls", p.foreground),
                ],
                row![
                    mono("crates  ", p.ansi[6]),
                    mono("README.md  ", p.ansi[3]),
                    mono("vif.toml", p.foreground)
                ],
                colors,
            ]
            .spacing(6),
        )
        .padding([10, 12])
        .width(Fill)
        .style(style::tile(p.background, 8.0));
        let place = match &card.theme.path {
            Some(path) => shown(path),
            None => "built-in".to_owned(),
        };
        let byline = if card.theme.author.is_empty() {
            place
        } else {
            format!("{} · {place}", card.theme.author)
        };
        let active = card.theme.name == self.config.theme;
        let content = column![
            row![
                text(card.theme.title.clone())
                    .size(15)
                    .font(style::semibold())
                    .width(Fill),
                text(card.theme.name.clone()).size(12).font(MONO).color(style::faint()),
            ]
            .align_y(Alignment::Center),
            text(byline).size(12).color(style::faint()),
            sample,
            text(card.looks.clone()).size(12).color(style::faint()),
        ]
        .spacing(8);
        button(content)
            .width(Fill)
            .padding(12)
            .style(style::theme_card(active, accent))
            .on_press(Message::UseTheme(card.theme.name.clone()))
            .into()
    }
}

fn shown(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    match home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}
