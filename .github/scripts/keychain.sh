#!/bin/bash
# Release CI: import the Developer ID certificate (base64 .p12 in
# MACOS_CERTIFICATE_P12, password in MACOS_CERTIFICATE_PASSWORD) into a
# throwaway keychain that codesign can use without prompting, and write the
# App Store Connect API key (base64 .p8 in NOTARY_KEY_P8) to a file.
# Exports NOTARY_KEY_PATH for later steps through $GITHUB_ENV.
set -euo pipefail
keychain="$RUNNER_TEMP/release.keychain-db"
password=$(openssl rand -hex 24)
cert="$RUNNER_TEMP/cert.p12"
printf '%s' "$MACOS_CERTIFICATE_P12" | base64 --decode > "$cert"

security create-keychain -p "$password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$password" "$keychain"
security import "$cert" -k "$keychain" -P "$MACOS_CERTIFICATE_PASSWORD" -T /usr/bin/codesign
security set-key-partition-list -S apple-tool:,apple: -s -k "$password" "$keychain" >/dev/null
# Put it first in the search list, so bundle.sh finds the identity.
existing=$(security list-keychains -d user | tr -d '"')
# shellcheck disable=SC2086 # the list is whitespace-separated paths without spaces
security list-keychains -d user -s "$keychain" $existing
rm -f "$cert"
security find-identity -v -p codesigning "$keychain" | grep -q "Developer ID Application" \
  || { echo "keychain: no Developer ID Application identity in the certificate" >&2; exit 1; }

key="$RUNNER_TEMP/notary.p8"
printf '%s' "$NOTARY_KEY_P8" | base64 --decode > "$key"
echo "NOTARY_KEY_PATH=$key" >> "$GITHUB_ENV"
echo "RELEASE_KEYCHAIN=$keychain" >> "$GITHUB_ENV"
