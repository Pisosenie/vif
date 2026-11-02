#!/bin/sh
# Builds target/release/vif.app: both binaries, the icon and an ad-hoc signature.
# --universal makes one app for Apple silicon and Intel.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
universal=${1:-}
if [ "$universal" = --universal ]; then
    for arch in aarch64 x86_64; do
        cargo build --release --locked --target $arch-apple-darwin -p vif -p vif-settings
    done
else
    cargo build --release -p vif -p vif-settings
fi
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
app=target/release/vif.app
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
for bin in vif vif-settings; do
    if [ "$universal" = --universal ]; then
        lipo -create -output "$app/Contents/MacOS/$bin" target/aarch64-apple-darwin/release/$bin \
            target/x86_64-apple-darwin/release/$bin
    else
        cp target/release/$bin "$app/Contents/MacOS/"
    fi
done

swiftc -O -o "$work/icon" tools/icon.swift
mkdir "$work/vif.iconset"
for size in 16 32 128 256 512; do
    "$work/icon" assets/vif.svg "$work/vif.iconset/icon_${size}x${size}.png" $size
    "$work/icon" assets/vif.svg "$work/vif.iconset/icon_${size}x${size}@2x.png" $((size * 2))
done
iconutil -c icns "$work/vif.iconset" -o "$app/Contents/Resources/vif.icns"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>vif</string>
  <key>CFBundleDisplayName</key><string>vif</string>
  <key>CFBundleIdentifier</key><string>io.github.pisosenie.vif</string>
  <key>CFBundleExecutable</key><string>vif</string>
  <key>CFBundleIconFile</key><string>vif</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

codesign --force --deep --sign - "$app"
echo "$app"
