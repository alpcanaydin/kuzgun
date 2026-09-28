#!/bin/bash
# Screenshot the Kuzgun window (no focus steal): scripts/shot.sh out.png
set -euo pipefail
out="${1:-/tmp/kuzgun.png}"
id=$(swift -e '
import CoreGraphics
let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as! [[String: Any]]
for w in list {
  let owner = w[kCGWindowOwnerName as String] as? String ?? ""
  let layer = w[kCGWindowLayer as String] as? Int ?? 1
  if owner.lowercased() == "kuzgun" && layer == 0 { print(w[kCGWindowNumber as String] as! Int); break }
}' 2>/dev/null)
[ -n "$id" ] || { echo "no kuzgun window" >&2; exit 1; }
screencapture -x -o -l"$id" "$out"
echo "$out"
