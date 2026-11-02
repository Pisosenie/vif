use iced::font::Weight;
use iced::widget::overlay::menu;
use iced::widget::{button, container, pick_list, slider, text_input, toggler};
use iced::{Background, Border, Color, Element, Font, Shadow, Theme, Vector};
use vif_config::Rgb;

const TEXT: Color = Color::from_rgb8(0xf4, 0xf3, 0xf0);
const INK: Color = Color::from_rgb8(0x0c, 0x0d, 0x10);
const WINDOW: Color = Color::from_rgb8(0x11, 0x13, 0x16);
const SIDEBAR: Color = Color::from_rgb8(0x16, 0x18, 0x1c);
const WARN: Color = Color::from_rgb8(0xf2, 0xb3, 0x5c);
const GOOD: Color = Color::from_rgb8(0x5d, 0xd3, 0x9e);

fn white(a: f32) -> Color {
    Color { a, ..Color::WHITE }
}

fn black(a: f32) -> Color {
    Color { a, ..Color::BLACK }
}

fn rounded(radius: f32) -> Border {
    Border {
        radius: radius.into(),
        ..Border::default()
    }
}

fn outlined(color: Color, radius: f32) -> Border {
    Border {
        color,
        width: 1.0,
        radius: radius.into(),
    }
}

pub fn rgb(c: Rgb) -> Color {
    Color::from_rgb8(c.r, c.g, c.b)
}

fn alpha(c: Rgb, a: f32) -> Color {
    Color { a, ..rgb(c) }
}

pub fn text() -> Color {
    TEXT
}

pub fn faint() -> Color {
    Color { a: 0.55, ..TEXT }
}

pub fn warn() -> Color {
    WARN
}

pub fn good() -> Color {
    GOOD
}

fn on(c: Rgb) -> Color {
    if c.luma() > 0.55 { INK } else { TEXT }
}

pub fn medium() -> Font {
    Font {
        weight: Weight::Medium,
        ..Font::with_name("Geist")
    }
}

pub fn semibold() -> Font {
    Font {
        weight: Weight::Semibold,
        ..Font::with_name("Geist")
    }
}

pub fn mono_bold() -> Font {
    Font {
        weight: Weight::Bold,
        ..Font::with_name("Geist Mono")
    }
}

pub fn window(_: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TEXT),
        background: Some(Background::Color(WINDOW)),
        ..container::Style::default()
    }
}

pub fn sidebar(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(SIDEBAR)),
        border: Border {
            color: white(0.06),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..container::Style::default()
    }
}

pub fn writes(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(black(0.28))),
        border: Border {
            color: white(0.06),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..container::Style::default()
    }
}

pub fn frame(_: &Theme) -> container::Style {
    container::Style {
        border: outlined(white(0.08), 12.0),
        shadow: Shadow {
            color: black(0.35),
            offset: Vector::new(0.0, 8.0),
            blur_radius: 24.0,
        },
        ..container::Style::default()
    }
}

pub fn card(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(white(0.03))),
        border: outlined(white(0.07), 12.0),
        ..container::Style::default()
    }
}

pub fn keycap(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(white(0.07))),
        border: outlined(white(0.1), 6.0),
        ..container::Style::default()
    }
}

pub fn swatch(c: Rgb) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(rgb(c))),
        border: outlined(white(0.18), 6.0),
        ..container::Style::default()
    }
}

/// A theme's own background, for its card in the gallery.
pub fn tile(bg: Rgb, radius: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(rgb(bg))),
        border: outlined(white(0.08), radius),
        ..container::Style::default()
    }
}

pub fn theme_card(active: bool, accent: Rgb) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        background: Some(Background::Color(match status {
            button::Status::Hovered | button::Status::Pressed => white(0.06),
            _ => white(0.03),
        })),
        text_color: TEXT,
        border: Border {
            color: if active { rgb(accent) } else { white(0.07) },
            width: if active { 2.0 } else { 1.0 },
            radius: 14.0.into(),
        },
        ..button::Style::default()
    }
}

pub fn line(hit: bool, accent: Rgb) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: hit.then(|| Background::Color(alpha(accent, 0.16))),
        border: rounded(4.0),
        ..container::Style::default()
    }
}

pub fn banner<'a, M: 'a>(content: Element<'a, M>) -> Element<'a, M> {
    container(content)
        .padding([14, 16])
        .width(iced::Fill)
        .style(|_| container::Style {
            background: Some(Background::Color(Color { a: 0.09, ..WARN })),
            border: outlined(Color { a: 0.3, ..WARN }, 12.0),
            ..container::Style::default()
        })
        .into()
}

pub fn nav(active: bool, accent: Rgb) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        background: match (active, status) {
            (true, _) => Some(Background::Color(alpha(accent, 0.16))),
            (false, button::Status::Hovered | button::Status::Pressed) => Some(Background::Color(white(0.06))),
            _ => None,
        },
        text_color: if active { rgb(accent) } else { Color { a: 0.8, ..TEXT } },
        border: rounded(8.0),
        ..button::Style::default()
    }
}

pub fn slot(c: Rgb, selected: bool, accent: Rgb) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        background: Some(Background::Color(rgb(c))),
        border: Border {
            color: if selected {
                rgb(accent)
            } else if matches!(status, button::Status::Hovered) {
                white(0.4)
            } else {
                white(0.15)
            },
            width: if selected { 2.0 } else { 1.0 },
            radius: 5.0.into(),
        },
        ..button::Style::default()
    }
}

pub fn ghost(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: match status {
            button::Status::Hovered => Some(Background::Color(white(0.07))),
            button::Status::Pressed => Some(Background::Color(white(0.11))),
            _ => None,
        },
        text_color: match status {
            button::Status::Disabled => faint(),
            _ => TEXT,
        },
        border: rounded(8.0),
        ..button::Style::default()
    }
}

pub fn warn_button(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: Some(Background::Color(match status {
            button::Status::Hovered | button::Status::Pressed => Color { a: 0.85, ..WARN },
            _ => WARN,
        })),
        text_color: INK,
        border: rounded(8.0),
        ..button::Style::default()
    }
}

pub fn choice(active: bool, accent: Rgb) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(match (active, hovered) {
                (true, false) => rgb(accent),
                (true, true) => Color { a: 0.85, ..rgb(accent) },
                (false, false) => white(0.06),
                (false, true) => white(0.1),
            })),
            text_color: if active { on(accent) } else { TEXT },
            border: rounded(8.0),
            ..button::Style::default()
        }
    }
}

pub fn toggle(accent: Rgb) -> impl Fn(&Theme, toggler::Status) -> toggler::Style {
    move |_, status| {
        let (is_on, hovered) = match status {
            toggler::Status::Active { is_toggled } | toggler::Status::Disabled { is_toggled } => (is_toggled, false),
            toggler::Status::Hovered { is_toggled } => (is_toggled, true),
        };
        let track = match (is_on, hovered) {
            (true, false) => rgb(accent),
            (true, true) => Color { a: 0.85, ..rgb(accent) },
            (false, false) => white(0.16),
            (false, true) => white(0.22),
        };
        toggler::Style {
            background: Background::Color(track),
            background_border_width: 0.0,
            background_border_color: Color::TRANSPARENT,
            foreground: Background::Color(Color::WHITE),
            foreground_border_width: 0.0,
            foreground_border_color: Color::TRANSPARENT,
            text_color: None,
            border_radius: None,
            padding_ratio: 0.12,
        }
    }
}

pub fn rail(accent: Rgb) -> impl Fn(&Theme, slider::Status) -> slider::Style {
    move |_, status| slider::Style {
        rail: slider::Rail {
            backgrounds: (Background::Color(rgb(accent)), Background::Color(white(0.14))),
            width: 4.0,
            border: rounded(999.0),
        },
        handle: slider::Handle {
            shape: slider::HandleShape::Circle {
                radius: if matches!(status, slider::Status::Active) {
                    7.0
                } else {
                    8.0
                },
            },
            background: Background::Color(Color::WHITE),
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        },
    }
}

pub fn input(_: &Theme, status: text_input::Status) -> text_input::Style {
    let edge = match status {
        text_input::Status::Focused { .. } => white(0.35),
        text_input::Status::Hovered => white(0.16),
        _ => white(0.08),
    };
    text_input::Style {
        background: Background::Color(black(0.28)),
        border: outlined(edge, 8.0),
        icon: faint(),
        placeholder: faint(),
        value: TEXT,
        selection: white(0.25),
    }
}

pub fn pick(_: &Theme, status: pick_list::Status) -> pick_list::Style {
    let edge = match status {
        pick_list::Status::Opened { .. } => white(0.35),
        pick_list::Status::Hovered => white(0.16),
        pick_list::Status::Active => white(0.08),
    };
    pick_list::Style {
        text_color: TEXT,
        placeholder_color: faint(),
        handle_color: faint(),
        background: Background::Color(white(0.06)),
        border: outlined(edge, 8.0),
    }
}

pub fn menu(_: &Theme) -> menu::Style {
    menu::Style {
        background: Background::Color(Color::from_rgb8(0x1b, 0x1d, 0x22)),
        border: outlined(white(0.08), 8.0),
        text_color: TEXT,
        selected_text_color: TEXT,
        selected_background: Background::Color(white(0.12)),
        shadow: Shadow {
            color: black(0.42),
            offset: Vector::new(0.0, 8.0),
            blur_radius: 24.0,
        },
    }
}
