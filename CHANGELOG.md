# Changelog

Every release, newest first. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/); until 1.0, a minor version may change the config.

## Unreleased

### Added

- A Debian package with every release, for Debian 12, Ubuntu 22.04 and newer on amd64 and arm64: `sudo apt install ./vif_<version>_<arch>.deb` brings what vif needs with it.

## 0.1.3 — 2026-11-04

### Security

- A program could crash vif, every tab with it, with one kitty graphics command: a PNG whose header claimed billions of pixels, a zlib stream that unpacked to gigabytes, `t=f` pointing at `/dev/zero`, or sizes that overflowed. Images are now refused past 8192×8192 pixels or 256 MB before anything is allocated, files must be regular files, and sizes are computed without overflow. Found by fuzzing.
- `vif gif` and the greeting held a full frame for every frame of a GIF and took any size. Frames are now scaled down as they are read, and a GIF may be at most 4096×4096 and 1000 frames.
- The archive of commands and their output was readable by other users of the machine. Its folder is now 0700 and its files 0600, older ones included, and tokens, keys, passwords, bearer headers, JWTs and private keys are taken out before anything is written.
- Without `XDG_RUNTIME_DIR`, the sessions' sockets went into a folder in `/tmp` that another user could have made first. vif now makes these folders 0700 and refuses one it doesn't own or that is a link.
- The session host asked for its terminal's name with `ptsname`, which isn't thread-safe; it uses `ptsname_r` on Linux and `TIOCPTYGNAME` on macOS.

### Fixed

- `vif greet --help` printed an error about a file called `--help`.
- A session host kept every control connection it ever had, so each look at the sessions dialog cost it a file; after a few hundred it could take no more.

### Added

- Ask a model, off until turned on: in block mode `e` explains why a command failed and offers the fix, Enter types it at the prompt; ⌘I (ctrl+shift+i) turns the words at the prompt into a command. Ollama on this machine by default, or any OpenAI-compatible API; secrets are taken out of what is sent, the key comes from an environment variable, and nothing runs by itself. The AI page in Settings sets it up.
- `vif ssh HOST [NAME]`: a shell on another machine that outlives the connection. It lives in a session host over there, vif connects again when the connection drops, and the screen comes back as it was; with vif's shell integration on that side too. vif has to be installed on HOST.
- After a restart, vif brings back the windows, tabs and splits it had, with a new shell in each pane's folder under what the old one printed last. Session hosts keep their output when the system stops them, with secrets taken out, readable by you alone.
- The banner about problems in `vif.toml` has Fix typo, Open in settings and × buttons.
- Search settings by name from the field above the pages; Esc clears it.
- Settings say what the last change replaced next to Undo, like `font.size was 13`.
- The sessions dialog groups sessions under the window that has them, with how many panes each has.
- On Linux, a notification about a finished command has Show block, which brings the command up picked, and Dismiss.
- Resize splits by dragging the divider, or with ⌃⌘ arrows (ctrl+shift+alt arrows on Linux).
- Zoom a pane to the whole tab and back with ⇧⌘Enter (ctrl+shift+z), like tmux's `prefix z`.
- Fuzzing with cargo-fuzz for program output, pictures, `vif.toml` and theme files, run weekly in CI.
- End-to-end tests of the session host.
- `vif-bin`, an Arch package of the release's prebuilt binaries, and a Homebrew cask in `packaging/homebrew`.
- Release builds for macOS are signed and notarized once the repository has a Developer ID; `tools/notarize-macos.sh` does it.
- The Linux tarball carries the `xterm-vif` terminfo source.
- `CONTRIBUTING.md` and issue forms.

## 0.1.2 — 2026-11-02

### Added

- A greeting like neofetch: `[greeting]` plays a GIF, or shows a PNG or JPEG, above the first prompt of each window, with the system, the shell and the colors beside it. Pick the file with Choose… in Settings, or run `vif greet`.
- Choose… for the picture behind the text.
- On macOS, ⌥⌫ ⌥← ⌥→ delete and move by word and ⌘⌫ ⌘← ⌘→ by line, the way other Mac terminals do.

## 0.1.1 — 2026-11-02

### Added

- `vif gif FILE` plays a GIF in colored text where you are, at under 1% CPU.

### Fixed

- Linux builds no longer carry macOS-only menu and focus code.

## 0.1.0 — 2026-11-02

The first release: GPU rendering through wgpu, commands as blocks, hints, search through everything, shells that outlive their window, a quick terminal, themes from any terminal, Ghostty shaders, rules by host and folder, a settings window, packages for Arch and builds for Linux and macOS.
