#!/bin/sh
# Puts vif under PREFIX (~/.local unless given): both binaries, a desktop entry and the icon.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
prefix=${1:-$HOME/.local}

install -Dm755 "$here/vif" "$prefix/bin/vif"
install -Dm755 "$here/vif-settings" "$prefix/bin/vif-settings"
install -Dm644 "$here/vif.svg" "$prefix/share/icons/hicolor/scalable/apps/vif.svg"
install -Dm644 "$here/vif.1" "$prefix/share/man/man1/vif.1"
mkdir -p "$prefix/share/applications"
sed "s|^Exec=vif|Exec=$prefix/bin/vif|" "$here/vif.desktop" > "$prefix/share/applications/vif.desktop"
echo "vif is in $prefix/bin"
