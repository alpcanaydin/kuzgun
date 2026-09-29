#!/bin/bash
# Build Kuzgun.app: release binary, Info.plist, the raven icon, Sparkle for
# in-app updates and the third-party notices.
#   scripts/bundle.sh   → target/release/bundle/Kuzgun.app
#
# Updates are on when assets/sparkle-public-key exists (made once by
# scripts/setup-release.sh). Signing: the Developer ID when one is in the
# keychain ($KUZGUN_DIST_IDENTITY overrides), else an Apple Development
# identity, else ad-hoc.
set -euo pipefail
cd "$(dirname "$0")/.."
root="$(pwd)"
cargo build --release
app="$root/target/release/bundle/Kuzgun.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/kuzgun "$app/Contents/MacOS/Kuzgun"
cp assets/icon/Kuzgun.icns "$app/Contents/Resources/Kuzgun.icns"
# In-app updates (src/updater.rs): Sparkle.framework, loaded at runtime.
public_key=""
[ -s assets/sparkle-public-key ] && public_key="$(tr -d '[:space:]' < assets/sparkle-public-key)"
if [ -n "$public_key" ] && ! [[ "$public_key" =~ ^[A-Za-z0-9+/]{43}=$ ]]; then
  echo "bundle: assets/sparkle-public-key is not a Sparkle public key" >&2
  exit 1
fi
if [ -n "$public_key" ]; then
  sparkle="$("$root/scripts/fetch-sparkle.sh")"
  mkdir -p "$app/Contents/Frameworks"
  ditto "$sparkle/Sparkle.framework" "$app/Contents/Frameworks/Sparkle.framework"
fi
# Licenses of everything the app ships.
"$root/scripts/notices.sh" > "$app/Contents/Resources/THIRD_PARTY_NOTICES.md"
version="${KUZGUN_VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}"
repo="${KUZGUN_REPO:-alpcanaydin/kuzgun}"
updates=""
if [ -n "$public_key" ]; then
  updates="  <key>SUFeedURL</key><string>https://github.com/$repo/releases/latest/download/appcast.xml</string>
  <key>SUPublicEDKey</key><string>$public_key</string>
  <key>SUEnableAutomaticChecks</key><true/>
  <key>SUAutomaticallyUpdate</key><true/>
  <key>SUVerifyUpdateBeforeExtraction</key><true/>"
fi
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Kuzgun</string>
  <key>CFBundleDisplayName</key><string>Kuzgun</string>
  <key>CFBundleIdentifier</key><string>ai.reyz.kuzgun</string>
  <key>CFBundleExecutable</key><string>Kuzgun</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>${version}</string>
  <key>CFBundleIconFile</key><string>Kuzgun</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>© 2026 Alpcan Aydın. Free and open source (MIT).</string>
${updates}
</dict></plist>
PLIST
# Distribution signature: Developer ID, hardened runtime, timestamped (what
# notarization needs).
dist="${KUZGUN_DIST_IDENTITY:-$(security find-identity -v -p codesigning 2>/dev/null \
  | sed -n 's/.*"\(Developer ID Application: [^"]*\)".*/\1/p' | head -1)}"
if [ -n "$dist" ]; then
  # Nested code first (Sparkle's helpers inside out), then the app.
  fw="$app/Contents/Frameworks/Sparkle.framework/Versions/B"
  if [ -d "$fw" ]; then
    for f in "$fw/XPCServices/Installer.xpc" "$fw/XPCServices/Downloader.xpc" "$fw/Autoupdate" "$fw/Updater.app"; do
      codesign --force --options runtime --timestamp --sign "$dist" "$f"
    done
    codesign --force --options runtime --timestamp --sign "$dist" "$app/Contents/Frameworks/Sparkle.framework"
  fi
  codesign --force --options runtime --timestamp --sign "$dist" --identifier ai.reyz.kuzgun "$app"
else
  "$root/scripts/sign-dev.sh" "$app" || codesign --force --deep --sign - "$app" >/dev/null 2>&1 || true
fi
echo "$app"
