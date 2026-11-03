#![no_main]

// A file `vif theme add` was handed: vif's own theme, or another terminal's colors.
libfuzzer_sys::fuzz_target!(|text: &str| {
    let theme = vif_config::themes::adopt(text, "fuzz");
    let _ = vif_config::Config::parse(&theme);
});
