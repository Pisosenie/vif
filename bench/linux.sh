#!/usr/bin/env bash
# Linux numbers for vif, Alacritty, kitty and foot under a headless sway of its own: time to a
# window, idle memory and vtebench. Needs sway, the four terminals, git and a built vif
# (CARGO_TARGET_DIR=~/.cache/vif-target cargo build --release) and vtebench there too.
set -uo pipefail
[ -d /tmp/vtebench-src ] || git clone -q --depth 1 https://github.com/alacritty/vtebench /tmp/vtebench-src
state=$(mktemp -d)
runtime=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
before=$(ls "$runtime" | grep -E '^wayland-[0-9]+$' || true)
printf 'output HEADLESS-1 resolution 1600x1000\ndefault_border none\n' > "$state/sway.conf"
WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman sway -c "$state/sway.conf" > "$state/sway.log" 2>&1 &
sway=$!
trap 'kill $sway 2> /dev/null; rm -rf "$state"' EXIT
for _ in $(seq 50); do
    display=$(comm -13 <(echo "$before") <(ls "$runtime" | grep -E '^wayland-[0-9]+$' || true) | head -1)
    [ -n "$display" ] && break
    sleep 0.1
done
export WAYLAND_DISPLAY=$display
export SWAYSOCK=$(ls -t "$runtime"/sway-ipc.*."$sway".sock 2> /dev/null | head -1)
python3 "$(dirname "$0")/linux.py"
