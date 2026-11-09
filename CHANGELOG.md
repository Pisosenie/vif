# Changelog

Every release, newest first. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/); until 1.0, a minor version may change the config.

## 0.1.8 — 2026-11-09

### Added

- Ctrl with a hint's letter opens a commit, or a file at its line, on the repository's page on GitHub, GitLab or Codeberg, from where `origin` points.

### Changed

- A session host marks where the painting it starts a client with ends, so `vif ssh` over UDP no longer waits a moment to be sure.
- On macOS, `vif ssh` acts only on changes to addresses, routes and interfaces, not on every lookup the system's routing socket reports.

### Fixed

- On a slow link, typing showed before its echo only when the first character's echo came back within half a second; now a slow echo still turns it on.

## 0.1.7 — 2026-11-09

### Added

- Tabs say the git branch of their folder, quieter after it, read from the repository without running git; `{branch}` puts it in the title, and `[[rules]]` take a `branch` pattern, like `release/*`, to give a branch its own look.

### Fixed

- A session that comes back keeps when each command ran and how long it took; its badges said 0.00s and the time it came back.
- Over plain ssh, a network change cut the first connection too, not only those after it.

## 0.1.6 — 2026-11-09

### Added

- `vif ssh` over UDP, the way mosh does it: ssh starts an agent over there and hands back its port and a one-time key, and keys and output then travel as datagrams sealed with ChaCha20-Poly1305. A laptop that sleeps or changes networks finds its shell where it left it, at once. Output passes through as it is, so the terminal's scrollback keeps it; after a long time away the screen is painted afresh with the lines that scrolled by. Typing shows before its echo on slow links, once a guess in that line came true. Ports 60000–61000, as mosh's; where UDP doesn't get through, `vif ssh` stays on ssh. `[ssh] udp`, `--udp` and `--no-udp` choose; ctrl-^ `.` leaves the shell running.
- Over plain ssh, `vif ssh` sends keepalives unless the ssh config has its own, so a dead link shows in seconds, and reconnects as soon as the network moves the machine to another address.
- A session comes back the way tmux brings one back: its host keeps a screen of what the shell printed and paints it, with the last thousand lines above it and commands still blocks with their exit codes. Before, it played the last two megabytes of output again, which could start in the middle of an escape sequence, miss that a full-screen program had the alternate screen, or wrap at the width it was printed at.
- `[keys] like = "kitty"`, `"wezterm"` or `"ghostty"` lays that terminal's shortcuts for what vif shares with it under your own; Settings → Keys has it too.

## 0.1.5 — 2026-11-04

### Security

- Links a program printed opened through the system whatever their scheme, so text that looked like a web link could start an app. Only `http`, `https`, `mailto` and `ftp` links go to the system now; a `file:` link opens as a path, in the editor or a tab, and a link with any other scheme is copied instead, with its hint saying so.
- A hint on a path starting with `-` handed it to the editor as an option; it's given as `./-…` now.
- `async-executor` is updated past a yanked release.

### Fixed

- Taking a session back after the window changed size no longer leaves zsh's `%` marks with the prompt pushed over.

## 0.1.4 — 2026-11-04

### Added

- Linux numbers in the README next to Alacritty, kitty and foot, with `bench/linux.sh` to make them.
- A Debian package with every release, for Debian 12, Ubuntu 22.04 and newer on amd64 and arm64: `sudo apt install ./vif_<version>_<arch>.deb` brings what vif needs with it.

### Fixed

- A new split, and the panes of a window coming back, started at the whole window's size and shrank a moment later; zsh's mark for a line without a newline was left behind as a stray `%`, with the prompt pushed over. Panes now start at the size they'll have.
- Output shown again after a restart no longer carries those marks from the width it was printed at.
- The AI panel shows a suggested command in color instead of between its ``` fences.

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
