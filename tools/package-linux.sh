#!/bin/sh
# Builds target/release/vif-VERSION-ARCH-linux.tar.gz: both binaries, the desktop entry, the icon and install.sh.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
cargo build --release --locked -p vif -p vif-settings
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
name=vif-$version-$(uname -m)-linux
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

mkdir "$work/$name"
cp target/release/vif target/release/vif-settings packaging/linux/install.sh assets/vif.desktop assets/vif.svg \
    README.md LICENSE-MIT LICENSE-APACHE "$work/$name/"
cp assets/fonts/OFL.txt "$work/$name/OFL-GEIST.txt"
tar -C "$work" -czf "target/release/$name.tar.gz" "$name"
echo "target/release/$name.tar.gz"
