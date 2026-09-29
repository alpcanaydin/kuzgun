#!/bin/bash
# Cut a release: bump the version, commit, tag and push. The tag starts the
# release workflow (.github/workflows/release.yml).
#   scripts/tag-release.sh 0.2.0
set -euo pipefail
cd "$(dirname "$0")/.."
version="${1:?usage: scripts/tag-release.sh <version, e.g. 0.2.0>}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "tag-release: version must look like 1.2.3" >&2; exit 1; }
[ -z "$(git status --porcelain)" ] || { echo "tag-release: commit or stash your changes first" >&2; exit 1; }
[ "$(git branch --show-current)" = main ] || { echo "tag-release: releases are cut from main" >&2; exit 1; }
git pull --ff-only -q
# Everything a release runs has to be executable in git.
bad=$(git ls-files -s -- '*.sh' | awk '$1 != "100755" { print $4 }')
[ -z "$bad" ] || { echo "tag-release: not executable in git: $bad" >&2; exit 1; }
# Release only a commit whose CI passed.
head=$(git rev-parse HEAD)
gate=$(gh api "repos/{owner}/{repo}/commits/$head/check-runs?check_name=gate" -q '.check_runs[0].conclusion // "missing"')
[ "$gate" = success ] || { echo "tag-release: CI gate on $(git rev-parse --short HEAD) is $gate; wait for a green run first" >&2; exit 1; }
sed -i '' "s/^version = \".*\"/version = \"$version\"/" Cargo.toml
cargo update -p kuzgun --offline -q 2>/dev/null || cargo update -p kuzgun -q
git add Cargo.toml Cargo.lock
# Already at this version (e.g. the first release): tag the current commit.
if ! git diff --cached --quiet; then
  git commit -q -m "Release $version"
fi
git tag -a "v$version" -m "Kuzgun $version"
git push -q origin main "v$version"
echo "Tagged v$version. Follow the build: gh run watch \$(gh run list -w release -L1 --json databaseId -q '.[0].databaseId')"
