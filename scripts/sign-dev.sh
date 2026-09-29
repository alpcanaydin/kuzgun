#!/bin/bash
# Sign a dev build with a stable identity, so macOS keeps what it granted it
# (notification permission) across rebuilds. Ad-hoc signatures change every
# build. Identity: $KUZGUN_SIGN_IDENTITY, else the first "Apple Development"
# identity. No identity: exit 1, and the caller signs ad-hoc.
set -euo pipefail
target="$1"
id="${KUZGUN_SIGN_IDENTITY:-$(security find-identity -v -p codesigning 2>/dev/null \
  | sed -n 's/.*"\(Apple Development: [^"]*\)".*/\1/p' | head -1)}"
[ -n "$id" ] || exit 1
codesign --force --deep --sign "$id" --identifier ai.reyz.kuzgun "$target" 2>/dev/null
