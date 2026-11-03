# Contributing to vif

Bug reports, fixes and ideas are welcome. This page is what you need to build vif, check a change and send it.

## Building

You need Rust 1.90 or newer, and on Linux a C compiler (`base-devel` on Arch, `build-essential` on Debian and Ubuntu). Wayland, Vulkan and the rest are opened at run time, so no headers are needed.

```bash
cargo run --release              # a window
cargo run --release -p vif-settings
```

`vif.toml` is read from `$VIF_CONFIG` when it's set, so a change can be tried without touching your own config:

```bash
VIF_CONFIG=/tmp/try.toml cargo run --release
```

## Checking a change

CI runs the same three on Arch and macOS, and a change has to pass all of them:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Code that reads what programs or files send in (the PTY filter in `crates/vif/src/graphics.rs` and `marks.rs`, pictures, `vif.toml`, theme files) has fuzz targets in `fuzz/`. If you change it, run the one it belongs to for a few minutes (it needs nightly and `cargo install cargo-fuzz`):

```bash
cd fuzz && cargo +nightly fuzz run pty -- -max_total_time=300
```

## How the code is written

- `rustfmt.toml` sets the width to 120; `cargo fmt` does the rest.
- Comments say why, not what. Most functions need one line or none.
- Every `unsafe` block starts with a `// SAFETY:` comment that says why it is sound; clippy refuses one without.
- Nothing from outside panics vif: errors go up with `?`, or are logged where nothing can be done about them.
- A new feature comes with a test of what it does, not of how. `crates/vif/tests/session.rs` shows how to test something end to end.
- A new setting goes into the config's reader with a check of its range, into `README.md` under "Setting it up", and into the settings window if it's one people change often.

## Commits and pull requests

One change per commit, said in one plain sentence: "Resize splits with the mouse or keys", not "feat: misc fixes". A pull request says what it changes and how you checked it; a screenshot helps when something on screen changes. Add a line to `CHANGELOG.md` under "Unreleased".

## Reporting a bug

Open an issue with what you did, what you expected, what happened, your OS, compositor or desktop, and `vif --version`. A crash or a hang is easier to find with `RUST_LOG=debug vif 2> vif.log`.

A security problem, such as something a program can make vif do that it shouldn't, goes to a private report on GitHub (Security → Report a vulnerability) rather than an issue.
