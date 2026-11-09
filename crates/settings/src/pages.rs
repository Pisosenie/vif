//! The settings pages that are a plain list of fields: font, colors, window, cursor and
//! terminal.

use iced::widget::{Column, Row, button, column, container, pick_list, row, slider, space, text, text_input, toggler};

use super::*;

impl Settings {
    pub(crate) fn font_page(&self) -> Element<'_, Message> {
        let c = &self.config;
        let accent = self.accent();
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

    pub(crate) fn colors_page(&self) -> Element<'_, Message> {
        let c = &self.config;
        let accent = self.accent();
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

    pub(crate) fn window_page(&self) -> Element<'_, Message> {
        let c = &self.config;
        let accent = self.accent();
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

    pub(crate) fn cursor_page(&self) -> Element<'_, Message> {
        let c = &self.config;
        let accent = self.accent();
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

    pub(crate) fn terminal_page(&self) -> Element<'_, Message> {
        let c = &self.config;
        let accent = self.accent();
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
                    slider(0.0f32..=100_000.0, c.terminal.scrollback as f32, Message::Scrollback).step(1000.0f32),
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
