#!/bin/bash
# Build a distributable release: signed + notarized + stapled Kuzgun.app and DMG,
# the DMG's Sparkle signature, and appcast.xml (the in-app update feed).
#   scripts/release.sh [notes file]
#   → target/release/bundle/{Kuzgun-<version>-arm64.dmg, appcast.xml}
#
# Notarization credentials, whichever is set:
#   - App Store Connect API key (CI): NOTARY_KEY_PATH, NOTARY_KEY_ID, NOTARY_ISSUER_ID
#   - a keychain profile (local):     $KUZGUN_NOTARY_PROFILE (default "kuzgun"),
#     made once by scripts/setup-release.sh.
# Sparkle signing key, whichever is set:
#   - SPARKLE_PRIVATE_KEY (CI), or the "kuzgun" key in the login keychain (local).
# KUZGUN_SKIP_NOTARIZE=1 skips notarization (local update tests only).
set -euo pipefail
cd "$(dirname "$0")/.."
notes="${1:-}"
version="${KUZGUN_VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}"
out="target/release/bundle"
app="$out/Kuzgun.app"
dmg="$out/Kuzgun-${version}-arm64.dmg"
repo="${KUZGUN_REPO:-alpcanaydin/kuzgun}"

# notarize <file to submit> <file to staple the ticket to>
notarize() {
  if [ -n "${KUZGUN_SKIP_NOTARIZE:-}" ]; then
    echo "release: notarization skipped"
    return
  fi
  if [ -n "${NOTARY_KEY_PATH:-}" ]; then
    xcrun notarytool submit "$1" --key "$NOTARY_KEY_PATH" --key-id "$NOTARY_KEY_ID" \
      --issuer "$NOTARY_ISSUER_ID" --wait --timeout 30m
  else
    xcrun notarytool submit "$1" --keychain-profile "${KUZGUN_NOTARY_PROFILE:-kuzgun}" --wait --timeout 30m
  fi
  xcrun stapler staple "$2"
}

# Signed with the Developer ID (hardened runtime, timestamp) by bundle.sh.
KUZGUN_VERSION="$version" scripts/bundle.sh
codesign --verify --deep --strict "$app"
identity=$(codesign -dvv "$app" 2>&1 | sed -n 's/^Authority=\(Developer ID Application:.*\)/\1/p' | head -1)
[ -n "$identity" ] || { echo "release: Kuzgun.app isn't signed with a Developer ID" >&2; exit 1; }

# Notarize the app itself first, so a copy taken out of the DMG keeps its ticket.
zip="$out/Kuzgun-notarize.zip"
ditto -c -k --keepParent "$app" "$zip"
notarize "$zip" "$app"
rm -f "$zip"

# DMG with an /Applications shortcut, signed and notarized too.
stage=$(mktemp -d)
ditto "$app" "$stage/Kuzgun.app"
ln -s /Applications "$stage/Applications"
rm -f "$dmg"
hdiutil create -volname "Kuzgun ${version}" -srcfolder "$stage" -ov -format UDZO "$dmg" >/dev/null
rm -rf "$stage"
codesign --force --timestamp --sign "$identity" "$dmg"
notarize "$dmg" "$dmg"

# What a user's Gatekeeper will say.
if [ -z "${KUZGUN_SKIP_NOTARIZE:-}" ]; then
  spctl -a -vv -t exec "$app"
  spctl -a -vv -t open --context context:primary-signature "$dmg"
fi

# The update feed: one item, the DMG, signed with the Sparkle (EdDSA) key.
scripts/appcast.sh "$version" "$dmg" "$repo" "$notes" > "$out/appcast.xml"
shasum -a 256 "$dmg"
echo "$dmg"
