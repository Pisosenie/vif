#!/usr/bin/env bash
# Linux numbers for vif, Alacritty, kitty and foot under a headless sway of its own: time to a
# window, idle memory and vtebench. Needs sway, the four terminals, a release build of vif and
# vtebench built in $VTEBENCH, as bench/run.sh does.
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
RES=1600x1000 source "$root/tools/sway-headless.sh"
python3 "$root/bench/linux.py"
