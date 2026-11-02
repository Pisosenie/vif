#!/bin/sh
# Measures vif against kitty, Alacritty and Ghostty on macOS.
#
#   bench/run.sh [startup|latency|vtebench|memory]…   (all four by default)
#
# Needs: a release build of vif, hyperfine-free; vtebench checked out in $VTEBENCH
# (default ~/.local/vif-bench/src/vtebench, built with cargo build --release);
# Accessibility and Screen Recording for the app running this script (latency only).
# Every terminal gets font size 13 and a 100×30 window. Only processes started
# here are closed.

set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT=${OUT:-$ROOT/target/bench}
VTEBENCH=${VTEBENCH:-$HOME/.local/vif-bench/src/vtebench}
ALACRITTY=${ALACRITTY:-$(command -v alacritty || echo "$HOME/.local/vif-bench/bin/alacritty")}
KITTY=${KITTY:-/Applications/kitty.app/Contents/MacOS/kitty}
GHOSTTY=${GHOSTTY:-/Applications/Ghostty.app/Contents/MacOS/ghostty}
mkdir -p "$OUT"
printf '[font]\nsize = 13\n\n[window]\ncolumns = 100\nlines = 30\n\n[terminal]\nsessions = false\n' > "$OUT/vif.toml"
export VIF_CONFIG="$OUT/vif.toml"
swiftc -O -o "$OUT/probe" "$ROOT/bench/probe.swift" 2>/dev/null

# name, window owner, command (the program to run is appended)
terminal() {
    case $1 in
        vif) echo "vif|$ROOT/target/release/vif -e" ;;
        alacritty) echo "alacritty|$ALACRITTY -o font.size=13 -o window.dimensions.columns=100 -o window.dimensions.lines=30 -e" ;;
        kitty) echo "kitty|$KITTY -o font_size=13 -o remember_window_size=no -o initial_window_width=100c -o initial_window_height=30c -o macos_quit_when_last_window_closed=yes" ;;
        ghostty) echo "Ghostty|$GHOSTTY --auto-update=off --font-size=13 --window-width=100 --window-height=30 --command=" ;;
    esac
}

launch() {
    name=$1; shift
    spec=$(terminal "$name"); owner=${spec%%|*}; cmd=${spec#*|}
    case $cmd in
        *--command=) set -- ${cmd%--command=} "--command=$*" ;;
        *) set -- $cmd "$@" ;;
    esac
    "$@" >/dev/null 2>&1 &
    echo $!
}

stop() { kill "$1" 2>/dev/null || true; sleep 1; kill -9 "$1" 2>/dev/null || true; }

startup() {
    for t in vif alacritty kitty ghostty; do
        spec=$(terminal $t); owner=${spec%%|*}; cmd=${spec#*|}
        cmd=${cmd% -e}; cmd=${cmd%--command=}
        printf '%-10s ' "$t"; "$OUT/probe" startup "$owner" 10 -- $cmd
    done
}

latency() {
    for t in vif alacritty kitty ghostty; do
        spec=$(terminal $t); owner=${spec%%|*}
        pid=$(launch $t cat); sleep 3
        printf '%-10s ' "$t"; "$OUT/probe" latency "$owner" 30
        stop "$pid"
    done
}

vtebench() {
    cat > "$OUT/vtebench.sh" <<EOF
#!/bin/sh
cd "$VTEBENCH" && ./target/release/vtebench -s --max-secs 3 --dat "$OUT/\$1.dat"
touch "$OUT/\$1.done"
EOF
    chmod +x "$OUT/vtebench.sh"
    for t in vif alacritty kitty ghostty; do
        rm -f "$OUT/$t.dat" "$OUT/$t.done"
        pid=$(launch $t "$OUT/vtebench.sh" $t)
        while [ ! -f "$OUT/$t.done" ]; do sleep 1; done
        stop "$pid"
    done
    python3 - "$OUT" <<'EOF'
import os, statistics, sys
out = sys.argv[1]
names = ["vif", "alacritty", "kitty", "ghostty"]
res = {}
for n in names:
    rows = [l.split() for l in open(os.path.join(out, n + ".dat"))]
    res[n] = {h: statistics.median(float(r[i]) for r in rows[1:] if i < len(r) and r[i] != "_") for i, h in enumerate(rows[0])}
print("| vtebench, ms per sample | " + " | ".join(names) + " |")
print("|---|" + "---:|" * len(names))
for b in res["vif"]:
    vals = [res[n].get(b) for n in names]
    best = min(v for v in vals if v is not None)
    print(f"| {b} | " + " | ".join(f"**{v:g}**" if v == best else f"{v:g}" for v in vals) + " |")
EOF
}

memory() {
    for t in vif alacritty kitty ghostty; do
        pid=$(launch $t /bin/zsh); sleep 4
        printf '%-10s %s MB\n' "$t" $(( $(ps -o rss= -p "$pid") / 1024 ))
        stop "$pid"
    done
}

[ $# -eq 0 ] && set -- startup latency vtebench memory
for step in "$@"; do echo "== $step"; $step; done
