#!/bin/bash
# Write Casks/kuzgun.rb for $VERSION / $SHA256 into the tap repo and push it.
# Also used locally by scripts/setup-release.sh to create the first cask.
# The cask has no `auto_updates`, so `brew upgrade` updates Kuzgun too; the app
# itself updates in place through Sparkle.
set -euo pipefail
: "${VERSION:?}" "${SHA256:?}" "${TAP_REPO:?}" "${TAP_TOKEN:?}"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
git clone -q "https://x-access-token:${TAP_TOKEN}@github.com/${TAP_REPO}.git" "$work/tap"
mkdir -p "$work/tap/Casks"
cat > "$work/tap/Casks/kuzgun.rb" <<EOF
cask "kuzgun" do
  version "$VERSION"
  sha256 "$SHA256"

  url "https://github.com/alpcanaydin/kuzgun/releases/download/v#{version}/Kuzgun-#{version}-arm64.dmg"
  name "Kuzgun"
  desc "Native kanban board for mattpocock/skills tickets"
  homepage "https://github.com/alpcanaydin/kuzgun"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on arch: :arm64
  depends_on macos: :sonoma

  app "Kuzgun.app"

  zap trash: [
    "~/Library/Application Support/kuzgun",
    "~/Library/Caches/ai.reyz.kuzgun",
    "~/Library/HTTPStorages/ai.reyz.kuzgun",
    "~/Library/Preferences/ai.reyz.kuzgun.plist",
    "~/Library/Saved Application State/ai.reyz.kuzgun.savedState",
  ]
end
EOF
cd "$work/tap"
git add Casks/kuzgun.rb
if git diff --cached --quiet; then
  echo "cask: already at $VERSION"
  exit 0
fi
git -c user.name="kuzgun-release" -c user.email="41898282+github-actions[bot]@users.noreply.github.com" \
  commit -q -m "kuzgun $VERSION"
git push -q origin HEAD
echo "cask: kuzgun $VERSION pushed to $TAP_REPO"
