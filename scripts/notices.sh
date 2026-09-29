#!/bin/bash
# Print THIRD_PARTY_NOTICES.md: the licenses of everything Kuzgun.app ships.
# scripts/bundle.sh writes it into Contents/Resources; it is generated, so it
# never drifts from Cargo.lock or the bundled assets.
#   scripts/notices.sh > THIRD_PARTY_NOTICES.md
set -euo pipefail
cd "$(dirname "$0")/.."
sparkle=$(sed -n 's/^version="\(.*\)"/\1/p' scripts/fetch-sparkle.sh)

cat <<NOTICES
# Third-party notices

The Kuzgun app bundle ships the software and assets below, each under its
own license.

## Frameworks, fonts and images

| Component | Version | License | Source |
| --- | --- | --- | --- |
| Sparkle.framework (in-app updates) | ${sparkle} | MIT | https://github.com/sparkle-project/Sparkle |
| Inter (font) | 4 | SIL Open Font License 1.1 | https://github.com/rsms/inter |
| Geist Mono (font) | 1 | SIL Open Font License 1.1 | https://github.com/vercel/geist-font |
| Pravka (font) | | see the font's own terms | |
| Blackbird, Microsoft Fluent Emoji (app icon) | | MIT | https://github.com/microsoft/fluentui-emoji |
| Claude Code and Codex marks, LobeHub Icons | 1.95.1 | MIT (the marks belong to Anthropic and OpenAI) | https://github.com/lobehub/lobe-icons |

## Rust crates compiled into Kuzgun

Generated from Cargo.lock (\`cargo metadata\`). Each crate's license text
ships with its source on crates.io.

| Crate | Version | License |
| --- | --- | --- |
NOTICES
cargo metadata --format-version 1 --locked 2>/dev/null | python3 -c '
import json, sys
d = json.load(sys.stdin)
for p in sorted(d["packages"], key=lambda p: (p["name"], p["version"])):
    if p["name"] == "kuzgun":
        continue
    print("| %s | %s | %s |" % (p["name"], p["version"], p.get("license") or "see crate"))
'
