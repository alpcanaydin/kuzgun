#!/bin/bash
# Print Sparkle's appcast.xml for one release. It's uploaded with each GitHub
# release, and Kuzgun.app reads it from releases/latest/download/appcast.xml.
#   scripts/appcast.sh <version> <dmg> [owner/repo] [notes file]
# The DMG is signed with SPARKLE_PRIVATE_KEY (CI) or the "kuzgun" key in the
# login keychain (local); the app verifies it with SUPublicEDKey.
set -euo pipefail
cd "$(dirname "$0")/.."
version="$1"
dmg="$2"
repo="${3:-alpcanaydin/kuzgun}"
notes_file="${4:-}"
sign_update="$(scripts/fetch-sparkle.sh)/bin/sign_update"
if [ -n "${SPARKLE_PRIVATE_KEY:-}" ]; then
  sig=$(printf '%s' "$SPARKLE_PRIVATE_KEY" | "$sign_update" --ed-key-file - "$dmg")
else
  sig=$("$sign_update" --account kuzgun "$dmg")
fi
# sign_update prints: sparkle:edSignature="…" length="…"
case "$sig" in *edSignature=*) ;; *) echo "appcast: signing failed" >&2; exit 1 ;; esac
file=$(basename "$dmg")
url="${KUZGUN_DOWNLOAD_BASE:-https://github.com/$repo/releases/download/v$version}/$file"
notes="Kuzgun $version"
[ -n "$notes_file" ] && [ -f "$notes_file" ] && notes=$(cat "$notes_file")
# Plain-text notes: escape what XML needs escaped.
notes=$(printf '%s' "$notes" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g')
cat <<EOF
<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>Kuzgun</title>
    <link>https://github.com/$repo</link>
    <item>
      <title>Kuzgun $version</title>
      <pubDate>$(LC_ALL=C date -u "+%a, %d %b %Y %H:%M:%S +0000")</pubDate>
      <sparkle:version>$version</sparkle:version>
      <sparkle:shortVersionString>$version</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>14.0</sparkle:minimumSystemVersion>
      <sparkle:fullReleaseNotesLink>https://github.com/$repo/releases/tag/v$version</sparkle:fullReleaseNotesLink>
      <description sparkle:format="plain-text">$notes</description>
      <enclosure url="$url" type="application/octet-stream" $sig />
    </item>
  </channel>
</rss>
EOF
