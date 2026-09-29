#!/bin/bash
# Download the pinned Sparkle release (the macOS app updater) into
# target/sparkle: Sparkle.framework plus its tools (bin/sign_update,
# bin/generate_keys). Checksum-verified; a second run is a no-op.
#   scripts/fetch-sparkle.sh        → prints the directory
set -euo pipefail
cd "$(dirname "$0")/.."
version="2.10.0"
sha256="c2bf58aa8387266ac179357b1415d6f2635f044da8be41042af32425dae6da0c"
dir="target/sparkle/$version"
if [ ! -d "$dir/Sparkle.framework" ]; then
  mkdir -p "$dir"
  tarball="$dir.tar.xz"
  curl -fsSL -o "$tarball" "https://github.com/sparkle-project/Sparkle/releases/download/$version/Sparkle-$version.tar.xz"
  echo "$sha256  $tarball" | shasum -a 256 -c - >/dev/null
  tar -xJf "$tarball" -C "$dir"
  rm -f "$tarball"
fi
echo "$(pwd)/$dir"
