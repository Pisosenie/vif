//! The greeting page, and the file chooser it shares with the picture behind the text.

use std::path::{Path, PathBuf};

use iced::widget::{button, column, pick_list, row, slider, text, toggler};
use iced::{Alignment, Element, Task};
use vif_config::GreetingShow;

use crate::{Choice, Message, Settings, field, ranged, style};

const SHOWS: [(&str, GreetingShow); 3] = [
    ("Off", GreetingShow::Off),
    ("Each new window", GreetingShow::Window),
    ("Every new shell", GreetingShow::Always),
];

#[derive(Clone, Debug)]
pub enum Change {
    Show(Choice),
    Blocks(bool),
    Info(bool),
    Height(f32),
    Loops(f32),
    /// Opens the system's file chooser for `section.key`.
    Pick(&'static str, &'static [&'static str]),
    Picked(&'static str, Option<PathBuf>),
}

/// A path the way vif.toml likes it: under home as `~/`.
fn written(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// A Choose… button that fills `key` with a file of one of `kinds`.
pub(crate) fn chooser<'a>(key: &'static str, kinds: &'static [&'static str]) -> Element<'a, Message> {
    button(text("Choose…").size(13))
        .padding([6, 14])
        .style(style::ghost)
        .on_press(Message::Greet(Change::Pick(key, kinds)))
        .into()
}

impl Settings {
    pub(crate) fn greet(&mut self, change: Change) -> Task<Message> {
        match change {
            Change::Show(Choice(name)) => {
                let value = match SHOWS.iter().find(|(n, _)| *n == name).map(|(_, s)| *s) {
                    Some(GreetingShow::Window) => "window",
                    Some(GreetingShow::Always) => "always",
                    _ => "off",
                };
                self.edit(|e| e.set("greeting", "show", value));
            }
            Change::Blocks(v) => self.edit(|e| e.set("greeting", "blocks", v)),
            Change::Info(v) => self.edit(|e| e.set("greeting", "info", v)),
            Change::Height(v) => self.edit(|e| e.set("greeting", "height", v.round() as i64)),
            Change::Loops(v) => self.edit(|e| e.set("greeting", "loops", v.round() as i64)),
            Change::Pick(key, kinds) => {
                let dialog = rfd::AsyncFileDialog::new()
                    .set_title("Choose a file for vif")
                    .add_filter("Pictures", kinds);
                let dialog = match std::env::var_os("HOME") {
                    Some(home) => dialog.set_directory(PathBuf::from(home).join("Pictures")),
                    None => dialog,
                };
                return Task::perform(dialog.pick_file(), move |file| {
                    Message::Greet(Change::Picked(key, file.map(|f| f.path().to_path_buf())))
                });
            }
            Change::Picked(key, Some(path)) => {
                let (section, name) = key.split_once('.').unwrap_or(("", key));
                self.edit(|e| e.set(section, name, written(&path)));
            }
            Change::Picked(_, None) => {}
        }
        Task::none()
    }

    pub(crate) fn greeting_page(&self) -> Element<'_, Message> {
        let g = &self.config.greeting;
        let accent = self.accent();
        let shown = SHOWS.iter().find(|(_, s)| *s == g.show).map_or("Off", |(n, _)| n);
        let toggle = |on: bool, change: fn(bool) -> Change| {
            toggler(on)
                .on_toggle(move |v| Message::Greet(change(v)))
                .style(style::toggle(accent))
                .into()
        };
        column![
            field(
                "Show it",
                pick_list(SHOWS.map(|(n, _)| Choice(n)), Some(Choice(shown)), |v| Message::Greet(
                    Change::Show(v)
                ))
                .width(220)
                .style(style::pick)
                .menu_style(style::menu)
                .into(),
                Some("before the first prompt, the way neofetch does"),
            ),
            field(
                "Picture",
                row![
                    self.typed(
                        "greeting.file",
                        g.file.clone().unwrap_or_default(),
                        "~/Pictures/cat.gif",
                        320.0
                    ),
                    chooser("greeting.file", &["gif", "png", "jpg", "jpeg"]),
                ]
                .spacing(10)
                .align_y(Alignment::Center)
                .into(),
                Some("a GIF plays, a PNG or JPEG stays; Enter sets a typed path"),
            ),
            field(
                "Blocks",
                toggle(g.blocks, Change::Blocks),
                Some("▀ halves in color; off draws it in characters")
            ),
            field(
                "About this machine",
                toggle(g.info, Change::Info),
                Some("who, the system, the shell, the colors")
            ),
            field(
                "Height",
                ranged(
                    slider(3.0f32..=60.0, f32::from(g.height), |v| Message::Greet(Change::Height(
                        v
                    ))),
                    format!("{} lines", g.height),
                    accent
                ),
                None,
            ),
            field(
                "Plays",
                ranged(
                    slider(1.0f32..=10.0, f32::from(g.loops), |v| Message::Greet(Change::Loops(v))),
                    if g.loops == 1 {
                        "once".to_owned()
                    } else {
                        format!("{} times", g.loops)
                    },
                    accent
                ),
                Some("ctrl-c cuts it short; the prompt waits 15 s at most"),
            ),
            text("Try it in any terminal with vif greet, or vif greet FILE for another picture.")
                .size(12)
                .color(style::faint()),
        ]
        .spacing(18)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_under_home_are_written_from_it() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            written(&Path::new(&home).join("Pictures/cat.gif")),
            "~/Pictures/cat.gif"
        );
        assert_eq!(written(Path::new("/tmp/cat.gif")), "/tmp/cat.gif");
    }
}
