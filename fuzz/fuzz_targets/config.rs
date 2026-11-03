#![no_main]

use vif_config::{Config, Editor};

// vif.toml as the window and the settings read it, every fix it offers applied, and an edit.
libfuzzer_sys::fuzz_target!(|text: &str| {
    let (_, diagnostics) = Config::parse(text);
    for fix in diagnostics.iter().filter_map(|d| d.fix.as_ref()) {
        if let Some(fixed) = fix.apply(text) {
            let _ = Config::parse(&fixed);
        }
    }
    if let Ok(mut editor) = Editor::parse(text) {
        editor.set("font", "size", 14);
        editor.set_float("window", "opacity", 0.9);
        let _ = Config::parse(&editor.text());
    }
});
