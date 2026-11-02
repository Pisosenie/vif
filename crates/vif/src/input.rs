use alacritty_terminal::term::TermMode;
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, KeyCode, KeyLocation, ModifiersState, NamedKey, PhysicalKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

/// What a key press sends to the program in the terminal.
pub fn encode(event: &KeyEvent, mods: ModifiersState, alt_meta: bool, mode: TermMode) -> Option<Vec<u8>> {
    if mode.intersects(TermMode::KITTY_KEYBOARD_PROTOCOL) {
        return kitty(event, mods, mode);
    }
    if event.state == ElementState::Released {
        return None;
    }
    legacy(event, mods, alt_meta, mode)
}

fn modifier_param(mods: ModifiersState) -> u32 {
    1 + u32::from(mods.shift_key())
        + 2 * u32::from(mods.alt_key())
        + 4 * u32::from(mods.control_key())
        + 8 * u32::from(mods.super_key())
}

fn legacy(event: &KeyEvent, mods: ModifiersState, alt_meta: bool, mode: TermMode) -> Option<Vec<u8>> {
    let m = 1 + u32::from(mods.shift_key()) + 2 * u32::from(mods.alt_key()) + 4 * u32::from(mods.control_key());
    let app = mode.contains(TermMode::APP_CURSOR);
    let esc = |rest: &[u8]| {
        let mut out = Vec::with_capacity(rest.len() + 1);
        if alt_meta {
            out.push(0x1b);
        }
        out.extend_from_slice(rest);
        out
    };
    if let Key::Named(named) = &event.logical_key {
        let cursor = |letter: u8| -> Vec<u8> {
            if m > 1 {
                format!("\x1b[1;{m}{}", letter as char).into_bytes()
            } else if app {
                vec![0x1b, b'O', letter]
            } else {
                vec![0x1b, b'[', letter]
            }
        };
        let tilde = |n: u32| -> Vec<u8> {
            if m > 1 {
                format!("\x1b[{n};{m}~").into_bytes()
            } else {
                format!("\x1b[{n}~").into_bytes()
            }
        };
        let ss3 = |letter: u8| -> Vec<u8> {
            if m > 1 {
                format!("\x1b[1;{m}{}", letter as char).into_bytes()
            } else {
                vec![0x1b, b'O', letter]
            }
        };
        let bytes = match named {
            NamedKey::Enter => esc(b"\r"),
            NamedKey::Tab if mods.shift_key() => b"\x1b[Z".to_vec(),
            NamedKey::Tab => esc(b"\t"),
            NamedKey::Backspace if mods.control_key() => esc(b"\x08"),
            NamedKey::Backspace => esc(b"\x7f"),
            NamedKey::Escape => esc(b"\x1b"),
            NamedKey::Space if mods.control_key() => esc(b"\0"),
            NamedKey::Space => esc(b" "),
            NamedKey::ArrowUp => cursor(b'A'),
            NamedKey::ArrowDown => cursor(b'B'),
            NamedKey::ArrowRight => cursor(b'C'),
            NamedKey::ArrowLeft => cursor(b'D'),
            NamedKey::Home => cursor(b'H'),
            NamedKey::End => cursor(b'F'),
            NamedKey::Insert => tilde(2),
            NamedKey::Delete => tilde(3),
            NamedKey::PageUp => tilde(5),
            NamedKey::PageDown => tilde(6),
            NamedKey::F1 => ss3(b'P'),
            NamedKey::F2 => ss3(b'Q'),
            NamedKey::F3 => ss3(b'R'),
            NamedKey::F4 => ss3(b'S'),
            NamedKey::F5 => tilde(15),
            NamedKey::F6 => tilde(17),
            NamedKey::F7 => tilde(18),
            NamedKey::F8 => tilde(19),
            NamedKey::F9 => tilde(20),
            NamedKey::F10 => tilde(21),
            NamedKey::F11 => tilde(23),
            NamedKey::F12 => tilde(24),
            _ => return event.text.as_ref().map(|t| esc(t.as_bytes())),
        };
        return Some(bytes);
    }
    if mods.control_key() {
        let base = base_char(event);
        if let Some(code) = base.and_then(control_code) {
            return Some(esc(&[code]));
        }
    }
    event.text.as_deref().map(|text| esc(text.as_bytes()))
}

/// The character the key types without modifiers, in the US layout when the current one isn't Latin.
fn base_char(event: &KeyEvent) -> Option<char> {
    let typed = match event.key_without_modifiers() {
        Key::Character(s) => s.chars().next(),
        _ => None,
    };
    match typed {
        Some(c) if c.is_ascii() => Some(c.to_ascii_lowercase()),
        _ => latin(event.physical_key).or(typed),
    }
}

fn control_code(c: char) -> Option<u8> {
    Some(match c {
        'a'..='z' => c as u8 - b'a' + 1,
        '@' | ' ' | '2' => 0,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '/' | '7' | '-' => 0x1f,
        '8' | '?' => 0x7f,
        _ => return None,
    })
}

fn latin(key: PhysicalKey) -> Option<char> {
    let PhysicalKey::Code(code) = key else {
        return None;
    };
    Some(match code {
        KeyCode::KeyA => 'a',
        KeyCode::KeyB => 'b',
        KeyCode::KeyC => 'c',
        KeyCode::KeyD => 'd',
        KeyCode::KeyE => 'e',
        KeyCode::KeyF => 'f',
        KeyCode::KeyG => 'g',
        KeyCode::KeyH => 'h',
        KeyCode::KeyI => 'i',
        KeyCode::KeyJ => 'j',
        KeyCode::KeyK => 'k',
        KeyCode::KeyL => 'l',
        KeyCode::KeyM => 'm',
        KeyCode::KeyN => 'n',
        KeyCode::KeyO => 'o',
        KeyCode::KeyP => 'p',
        KeyCode::KeyQ => 'q',
        KeyCode::KeyR => 'r',
        KeyCode::KeyS => 's',
        KeyCode::KeyT => 't',
        KeyCode::KeyU => 'u',
        KeyCode::KeyV => 'v',
        KeyCode::KeyW => 'w',
        KeyCode::KeyX => 'x',
        KeyCode::KeyY => 'y',
        KeyCode::KeyZ => 'z',
        KeyCode::Digit0 => '0',
        KeyCode::Digit1 => '1',
        KeyCode::Digit2 => '2',
        KeyCode::Digit3 => '3',
        KeyCode::Digit4 => '4',
        KeyCode::Digit5 => '5',
        KeyCode::Digit6 => '6',
        KeyCode::Digit7 => '7',
        KeyCode::Digit8 => '8',
        KeyCode::Digit9 => '9',
        KeyCode::BracketLeft => '[',
        KeyCode::BracketRight => ']',
        KeyCode::Backslash => '\\',
        KeyCode::Slash => '/',
        KeyCode::Minus => '-',
        KeyCode::Equal => '=',
        KeyCode::Semicolon => ';',
        KeyCode::Quote => '\'',
        KeyCode::Comma => ',',
        KeyCode::Period => '.',
        KeyCode::Backquote => '`',
        _ => return None,
    })
}

/// The kitty keyboard protocol: https://sw.kovidgoyal.net/kitty/keyboard-protocol/
fn kitty(event: &KeyEvent, mods: ModifiersState, mode: TermMode) -> Option<Vec<u8>> {
    let all = mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC);
    let events = mode.contains(TermMode::REPORT_EVENT_TYPES);
    let alternates = mode.contains(TermMode::REPORT_ALTERNATE_KEYS);
    let with_text = mode.contains(TermMode::REPORT_ASSOCIATED_TEXT);
    let kind = match (event.state, event.repeat) {
        (ElementState::Released, _) => 3,
        (_, true) => 2,
        _ => 1,
    };
    if kind == 3 && !events {
        return None;
    }
    let m = modifier_param(mods);
    let plain = m & !1 == 0;

    let (code, end) = match &event.logical_key {
        Key::Named(named) => match functional(*named, event.location) {
            Some(found) => found,
            None if *named == NamedKey::Space => (32, b'u'),
            None => return None,
        },
        Key::Character(_) => {
            let c = match event.key_without_modifiers() {
                Key::Character(s) => s.chars().next()?,
                _ => return None,
            };
            // Shortcuts in a non-Latin layout mean the Latin key, or ctrl+w in vim would never fire.
            let shortcut = mods.control_key() || mods.alt_key() || mods.super_key();
            let c = match latin(event.physical_key) {
                Some(l) if shortcut && !c.is_ascii() => l,
                _ => c,
            };
            (c.to_lowercase().next().unwrap_or(c) as u32, b'u')
        }
        _ => return None,
    };

    let is_text = matches!(event.logical_key, Key::Character(_)) || code == 32;
    let modifier_key = (57441..=57452).contains(&code);
    if modifier_key && !all {
        return None;
    }
    if !all {
        if is_text && m & !1 == 0 {
            return if kind == 3 {
                None
            } else {
                event.text.as_ref().map(|t| t.as_bytes().to_vec())
            };
        }
        if matches!(code, 13 | 9 | 127) && plain {
            if kind == 3 {
                return None;
            }
            return Some(match code {
                13 => b"\r".to_vec(),
                9 => b"\t".to_vec(),
                _ => b"\x7f".to_vec(),
            });
        }
    }

    let mut key = code.to_string();
    if alternates && end == b'u' && is_text {
        let shifted = mods
            .shift_key()
            .then(|| event.text.as_ref().and_then(|t| t.chars().next()))
            .flatten()
            .map(|c| c as u32)
            .filter(|&c| c != code);
        let base = latin(event.physical_key).map(|c| c as u32).filter(|&c| c != code);
        if shifted.is_some() || base.is_some() {
            key.push(':');
            if let Some(s) = shifted {
                key.push_str(&s.to_string());
            }
            if let Some(b) = base {
                key.push(':');
                key.push_str(&b.to_string());
            }
        }
    }
    let text = (with_text && all && kind != 3)
        .then_some(event.text.as_ref())
        .flatten()
        .filter(|t| t.chars().all(|c| !c.is_control()))
        .map(|t| t.chars().map(|c| (c as u32).to_string()).collect::<Vec<_>>().join(":"));

    let mut params = String::new();
    if m > 1 || kind > 1 || text.is_some() {
        params.push_str(&m.to_string());
        if kind > 1 {
            params.push(':');
            params.push_str(&kind.to_string());
        }
    }
    if let Some(text) = &text {
        params.push(';');
        params.push_str(text);
    }
    let out = if end == b'u' || end == b'~' {
        if params.is_empty() {
            format!("\x1b[{key}{}", end as char)
        } else {
            format!("\x1b[{key};{params}{}", end as char)
        }
    } else if params.is_empty() {
        format!("\x1b[{}", end as char)
    } else {
        format!("\x1b[1;{params}{}", end as char)
    };
    Some(out.into_bytes())
}

fn functional(key: NamedKey, location: KeyLocation) -> Option<(u32, u8)> {
    let right = location == KeyLocation::Right;
    Some(match key {
        NamedKey::Escape => (27, b'u'),
        NamedKey::Enter => (13, b'u'),
        NamedKey::Tab => (9, b'u'),
        NamedKey::Backspace => (127, b'u'),
        NamedKey::Insert => (2, b'~'),
        NamedKey::Delete => (3, b'~'),
        NamedKey::ArrowLeft => (1, b'D'),
        NamedKey::ArrowRight => (1, b'C'),
        NamedKey::ArrowUp => (1, b'A'),
        NamedKey::ArrowDown => (1, b'B'),
        NamedKey::PageUp => (5, b'~'),
        NamedKey::PageDown => (6, b'~'),
        NamedKey::Home => (1, b'H'),
        NamedKey::End => (1, b'F'),
        NamedKey::CapsLock => (57358, b'u'),
        NamedKey::ScrollLock => (57359, b'u'),
        NamedKey::NumLock => (57360, b'u'),
        NamedKey::PrintScreen => (57361, b'u'),
        NamedKey::Pause => (57362, b'u'),
        NamedKey::ContextMenu => (57363, b'u'),
        NamedKey::F1 => (1, b'P'),
        NamedKey::F2 => (1, b'Q'),
        NamedKey::F3 => (13, b'~'),
        NamedKey::F4 => (1, b'S'),
        NamedKey::F5 => (15, b'~'),
        NamedKey::F6 => (17, b'~'),
        NamedKey::F7 => (18, b'~'),
        NamedKey::F8 => (19, b'~'),
        NamedKey::F9 => (20, b'~'),
        NamedKey::F10 => (21, b'~'),
        NamedKey::F11 => (23, b'~'),
        NamedKey::F12 => (24, b'~'),
        NamedKey::Shift => (if right { 57447 } else { 57441 }, b'u'),
        NamedKey::Control => (if right { 57448 } else { 57442 }, b'u'),
        NamedKey::Alt => (if right { 57449 } else { 57443 }, b'u'),
        NamedKey::Super | NamedKey::Meta => (if right { 57450 } else { 57444 }, b'u'),
        _ => return None,
    })
}

/// A mouse report in whichever encoding the program asked for; `button` uses xterm numbering.
pub fn mouse(
    button: u8,
    col: usize,
    line: usize,
    pressed: bool,
    mods: ModifiersState,
    mode: TermMode,
) -> Option<Vec<u8>> {
    let mut b = u32::from(button);
    if mods.shift_key() {
        b += 4;
    }
    if mods.alt_key() {
        b += 8;
    }
    if mods.control_key() {
        b += 16;
    }
    let (x, y) = (col + 1, line + 1);
    if mode.contains(TermMode::SGR_MOUSE) {
        let end = if pressed { 'M' } else { 'm' };
        return Some(format!("\x1b[<{b};{x};{y}{end}").into_bytes());
    }
    let b = if pressed { b } else { 3 + (b & !3) };
    if mode.contains(TermMode::UTF8_MOUSE) {
        let mut out = String::from("\x1b[M");
        for v in [32 + b as usize, 32 + x, 32 + y] {
            out.push(char::from_u32(v as u32)?);
        }
        return Some(out.into_bytes());
    }
    if x > 223 || y > 223 {
        return None;
    }
    Some(vec![0x1b, b'[', b'M', 32 + b as u8, 32 + x as u8, 32 + y as u8])
}
