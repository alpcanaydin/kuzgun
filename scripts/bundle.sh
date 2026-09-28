#!/bin/bash
# Build Kuzgun.app: release binary + Info.plist + the raven icon.
#   scripts/bundle.sh   → target/release/bundle/Kuzgun.app
set -euo pipefail
cd "$(dirname "$0")/.."
root="$(pwd)"
cargo build --release
app="$root/target/release/bundle/Kuzgun.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/kuzgun "$app/Contents/MacOS/Kuzgun"
cp assets/icon/Kuzgun.icns "$app/Contents/Resources/Kuzgun.icns"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
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
</dict></plist>
PLIST
# Ad-hoc signature; a Developer ID can replace it for distribution.
codesign --force --sign - "$app" >/dev/null 2>&1 || true
echo "$app"
