//! Other terminals' keys, for hands that learned them there: `[keys] like = "kitty"` lays a
//! terminal's defaults for the actions vif shares with it over vif's own. Each list follows that
//! terminal's documented defaults; actions vif doesn't have are left out.

pub const PRESETS: [&str; 3] = ["kitty", "wezterm", "ghostty"];

type Keys = &'static [(&'static str, &'static str)];

/// The keys `name` lays over vif's defaults on this system.
pub fn preset(name: &str) -> Keys {
    let mac = cfg!(target_os = "macos");
    match (name, mac) {
        ("kitty", false) => KITTY,
        ("kitty", true) => KITTY_MAC,
        ("wezterm", false) => WEZTERM,
        ("wezterm", true) => WEZTERM_MAC,
        ("ghostty", false) => GHOSTTY,
        ("ghostty", true) => GHOSTTY_MAC,
        _ => &[],
    }
}

const KITTY: Keys = &[
    ("ctrl+shift+z", "prompt_previous"),
    ("ctrl+shift+x", "prompt_next"),
    ("ctrl+shift+enter", "split_right"),
    ("ctrl+shift+right", "next_tab"),
    ("ctrl+shift+left", "previous_tab"),
    ("ctrl+shift+=", "font_bigger"),
    ("ctrl+shift+-", "font_smaller"),
    ("ctrl+shift+backspace", "font_reset"),
    ("ctrl+shift+pageup", "scroll_page_up"),
    ("ctrl+shift+pagedown", "scroll_page_down"),
    ("ctrl+shift+/", "search"),
    ("ctrl+shift+e", "hints"),
    ("ctrl+shift+f2", "settings"),
];

const KITTY_MAC: Keys = &[
    ("cmd+enter", "split_right"),
    ("ctrl+tab", "next_tab"),
    ("ctrl+shift+tab", "previous_tab"),
    ("cmd+pageup", "scroll_page_up"),
    ("cmd+pagedown", "scroll_page_down"),
];

const WEZTERM: Keys = &[
    ("ctrl+shift+1", "tab_1"),
    ("ctrl+shift+2", "tab_2"),
    ("ctrl+shift+3", "tab_3"),
    ("ctrl+shift+4", "tab_4"),
    ("ctrl+shift+5", "tab_5"),
    ("ctrl+shift+6", "tab_6"),
    ("ctrl+shift+7", "tab_7"),
    ("ctrl+shift+8", "tab_8"),
    ("ctrl+shift+9", "tab_9"),
    ("ctrl+pageup", "previous_tab"),
    ("ctrl+pagedown", "next_tab"),
    ("ctrl+shift+alt+'", "split_down"),
    ("ctrl+shift+alt+5", "split_right"),
    ("ctrl+shift+left", "focus_left"),
    ("ctrl+shift+right", "focus_right"),
    ("ctrl+shift+up", "focus_up"),
    ("ctrl+shift+down", "focus_down"),
    ("ctrl+shift+space", "hints"),
];

const WEZTERM_MAC: Keys = &[
    ("ctrl+shift+alt+'", "split_down"),
    ("ctrl+shift+alt+5", "split_right"),
    ("ctrl+shift+alt+left", "resize_left"),
    ("ctrl+shift+alt+right", "resize_right"),
    ("ctrl+shift+alt+up", "resize_up"),
    ("ctrl+shift+alt+down", "resize_down"),
    ("ctrl+shift+left", "focus_left"),
    ("ctrl+shift+right", "focus_right"),
    ("ctrl+shift+up", "focus_up"),
    ("ctrl+shift+down", "focus_down"),
    ("ctrl+shift+z", "zoom"),
    ("ctrl+shift+p", "palette"),
    ("ctrl+shift+space", "hints"),
    ("ctrl+tab", "next_tab"),
    ("ctrl+shift+tab", "previous_tab"),
];

const GHOSTTY: Keys = &[
    ("ctrl+shift+o", "split_right"),
    ("ctrl+shift+e", "split_down"),
    ("ctrl+cmd+[", "focus_previous"),
    ("ctrl+cmd+]", "focus_next"),
    ("ctrl+shift+cmd+left", "resize_left"),
    ("ctrl+shift+cmd+right", "resize_right"),
    ("ctrl+shift+cmd+up", "resize_up"),
    ("ctrl+shift+cmd+down", "resize_down"),
    ("ctrl+shift+enter", "zoom"),
    ("ctrl+shift+left", "previous_tab"),
    ("ctrl+shift+right", "next_tab"),
    ("ctrl+pageup", "previous_tab"),
    ("ctrl+pagedown", "next_tab"),
    ("shift+home", "scroll_top"),
    ("shift+end", "scroll_bottom"),
];

const GHOSTTY_MAC: Keys = &[
    ("cmd+pageup", "scroll_page_up"),
    ("cmd+pagedown", "scroll_page_down"),
    ("ctrl+tab", "next_tab"),
    ("ctrl+shift+tab", "previous_tab"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ACTIONS;

    #[test]
    fn every_preset_names_real_actions() {
        for list in [KITTY, KITTY_MAC, WEZTERM, WEZTERM_MAC, GHOSTTY, GHOSTTY_MAC] {
            for (combo, action) in list {
                assert!(ACTIONS.contains(action), "{combo} = {action}");
            }
        }
    }
}
