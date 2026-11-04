#!/usr/bin/env bash
# Screenshot vif under a throwaway headless sway.
# usage: tools/headless.sh OUT.png [vif args...]   (Linux, needs sway and grim)
set -euo pipefail

out=$(realpath -m "$1")
shift
root=$(cd "$(dirname "$0")/.." && pwd)
target=${CARGO_TARGET_DIR:-$root/target}
cargo build --manifest-path "$root/Cargo.toml" --release --quiet -p vif

source "$root/tools/sway-headless.sh"

"$target/release/vif" "$@" > "${out%.png}.log" 2>&1 &
sleep "${WAIT:-4}"
grim "$out"
kill $! 2> /dev/null
echo "$out"
