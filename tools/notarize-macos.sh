#!/bin/sh
# Signs vif.app with a Developer ID, has Apple notarize it and staples the ticket to it, so macOS
# opens it without asking. Takes the app's path; reads the certificate and the Apple account from:
#   MACOS_CERTIFICATE           the Developer ID Application certificate and key, a .p12 in base64
#   MACOS_CERTIFICATE_PASSWORD  the .p12's password
#   MACOS_SIGNING_IDENTITY      "Developer ID Application: Name (TEAMID)"
#   APPLE_ID, APPLE_TEAM_ID     the developer account
#   APPLE_APP_PASSWORD          an app-specific password for it, from account.apple.com
set -eu
app=$1
work=$(mktemp -d)
keychain=$work/signing.keychain-db
password=$(uuidgen)
trap 'security delete-keychain "$keychain" 2>/dev/null; rm -rf "$work"' EXIT

security create-keychain -p "$password" "$keychain"
security set-keychain-settings -lut 3600 "$keychain"
security unlock-keychain -p "$password" "$keychain"
printf '%s' "$MACOS_CERTIFICATE" | base64 --decode > "$work/cert.p12"
security import "$work/cert.p12" -k "$keychain" -P "$MACOS_CERTIFICATE_PASSWORD" -T /usr/bin/codesign
rm "$work/cert.p12"
security set-key-partition-list -S apple-tool:,apple: -s -k "$password" "$keychain" > /dev/null
security list-keychains -d user -s "$keychain" $(security list-keychains -d user | tr -d '"')

# The binaries inside first, then the bundle that seals them.
for bin in "$app"/Contents/MacOS/*; do
    codesign --force --options runtime --timestamp --sign "$MACOS_SIGNING_IDENTITY" --keychain "$keychain" "$bin"
done
codesign --force --options runtime --timestamp --sign "$MACOS_SIGNING_IDENTITY" --keychain "$keychain" "$app"
codesign --verify --strict --deep --verbose=2 "$app"

ditto -c -k --keepParent "$app" "$work/notarize.zip"
xcrun notarytool submit "$work/notarize.zip" --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" \
    --password "$APPLE_APP_PASSWORD" --wait --timeout 30m
xcrun stapler staple "$app"
spctl --assess --type execute --verbose "$app"
