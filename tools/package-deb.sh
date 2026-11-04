#!/bin/sh
# Builds target/release/vif_VERSION_ARCH.deb from the binaries tools/package-linux.sh built (it
# builds them itself when they aren't there): both binaries, the terminfo, the desktop entry,
# the icon and the licenses, for Debian 12 and Ubuntu 22.04 on.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
case $(uname -m) in
    x86_64) arch=amd64 ;;
    aarch64 | arm64) arch=arm64 ;;
    *) echo "no Debian name for $(uname -m)" >&2; exit 1 ;;
esac
[ -x target/release/vif ] && [ -x target/release/vif-settings ] ||
    cargo build --release --locked -p vif -p vif-settings
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
pkg=$work/vif

install -Dm755 target/release/vif "$pkg/usr/bin/vif"
install -Dm755 target/release/vif-settings "$pkg/usr/bin/vif-settings"
mkdir -p "$pkg/usr/share/terminfo"
tic -x -o "$pkg/usr/share/terminfo" assets/xterm-vif.terminfo
install -Dm644 assets/vif.desktop "$pkg/usr/share/applications/vif.desktop"
install -Dm644 assets/vif.svg "$pkg/usr/share/icons/hicolor/scalable/apps/vif.svg"
install -Dm644 README.md "$pkg/usr/share/doc/vif/README.md"
{
    echo "vif is under MIT or Apache-2.0, at your choice; Geist Mono, built in, under the OFL."
    for license in LICENSE-MIT LICENSE-APACHE assets/fonts/OFL.txt; do
        printf '\n== %s ==\n\n' "$(basename "$license")"
        cat "$license"
    done
} > "$pkg/usr/share/doc/vif/copyright"

mkdir -p "$pkg/DEBIAN"
cat > "$pkg/DEBIAN/control" <<CONTROL
Package: vif
Version: $version
Architecture: $arch
Maintainer: Danil Zorin <45851636+Pisosenie@users.noreply.github.com>
Installed-Size: $(du -ks "$pkg/usr" | cut -f1)
Depends: libc6 (>= 2.35), libgcc-s1, libwayland-client0, libxkbcommon0, fontconfig, libegl1 | libvulkan1
Recommends: libvulkan1, libnotify-bin, curl
Suggests: libx11-6, libxcursor1, libxi6, libxrandr2, libxkbcommon-x11-0
Section: x11
Priority: optional
Homepage: https://github.com/Pisosenie/vif
Description: fast GPU terminal with command blocks and sessions that outlive the window
 vif draws on the GPU through wgpu and runs on Wayland without XWayland. Every
 command is a block to copy, fold or run again; shells keep running when the
 window closes, and over ssh when the connection drops; a quick terminal drops
 in from the top of the screen. Themes from other terminals, Ghostty shaders.
CONTROL
dpkg-deb --root-owner-group --build "$pkg" "target/release/vif_${version}_${arch}.deb" > /dev/null
echo "target/release/vif_${version}_${arch}.deb"
