//! `vif niri`: a fragment for niri's config that floats the quick terminal along the top of the
//! screen and binds a key to it. It goes next to the one Apolline writes, by `include`.

use vif_config::Config;

pub fn fragment(config: &Config) -> String {
    let height = (config.quick.height * 100.0).round() / 100.0;
    format!(
        r#"// vif for niri. Save it with `vif niri > ~/.config/niri/vif.kdl` and add
// `include "vif.kdl"` to config.kdl; vif writes it again from vif.toml whenever you ask.

// The quick terminal floats over the windows along the top of the screen.
window-rule {{
    match app-id="^vif-quick$"
    open-floating true
    open-focused true
    default-column-width {{ proportion 1.0; }}
    default-window-height {{ proportion {height}; }}
    default-floating-position x=0 y=0 relative-to="top"
}}

binds {{
    Mod+Grave hotkey-overlay-title="Quick terminal" {{ spawn "vif" "quick"; }}
}}
"#
    )
}
