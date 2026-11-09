//! The tabs and keys pages: where tabs sit and how splits part, and vif's own shortcuts.

use iced::widget::{Column, button, column, container, pick_list, row, slider, text, text_input};

use super::*;

impl Settings {
    pub(crate) fn tabs(&self) -> Element<'_, Message> {
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

    pub(crate) fn keys(&self) -> Element<'_, Message> {
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
        let [kitty, wezterm, ghostty] = vif_config::PRESETS;
        let like = field(
            "Keys like",
            pick_list(
                [Choice("vif"), Choice(kitty), Choice(wezterm), Choice(ghostty)],
                Some(Choice(self.config.keys_like.unwrap_or("vif"))),
                Message::KeysLike,
            )
            .width(200)
            .style(style::pick)
            .menu_style(style::menu)
            .into(),
            Some("another terminal's shortcuts for what vif shares with it; yours still win"),
        );
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
        column![like, add, container(list).padding(14).style(style::card)]
            .spacing(16)
            .into()
    }
}
