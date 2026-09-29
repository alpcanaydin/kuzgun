#!/bin/bash
# One-time release setup wizard. Walks through everything the release
# workflow needs that only a person can do, checks each piece, and stores it
# as a GitHub secret in the "release" environment. Safe to re-run: finished
# steps are detected and can be skipped. Secrets are read without echo and go
# to `gh secret set` on stdin; nothing is printed or written to the repo.
#   scripts/setup-release.sh
set -euo pipefail
cd "$(dirname "$0")/.."
REPO="${KUZGUN_REPO:-alpcanaydin/kuzgun}"
TAP="${KUZGUN_TAP:-alpcanaydin/homebrew-kuzgun}"
ENV_NAME=release
PUBLIC_KEY=$(tr -d '[:space:]' < assets/sparkle-public-key 2>/dev/null || true)

bold=$(tput bold 2>/dev/null || true); dim=$(tput dim 2>/dev/null || true); reset=$(tput sgr0 2>/dev/null || true)
green=$(tput setaf 2 2>/dev/null || true); red=$(tput setaf 1 2>/dev/null || true); yellow=$(tput setaf 3 2>/dev/null || true)
step() { printf '\n%s━━ %s ━━%s\n' "$bold" "$1" "$reset"; }
ok() { printf '%s✓%s %s\n' "$green" "$reset" "$1"; }
warn() { printf '%s!%s %s\n' "$yellow" "$reset" "$1"; }
fail() { printf '%s✗%s %s\n' "$red" "$reset" "$1" >&2; exit 1; }
ask() { local a; read -r -p "$1 " a; printf '%s' "$a"; }
ask_secret() { local a; read -r -s -p "$1 " a; echo >&2; printf '%s' "$a"; }
yes() { local a; read -r -p "$1 [Y/n] " a; [[ -z "$a" || "$a" =~ ^[Yy] ]]; }
no() { local a; read -r -p "$1 [y/N] " a; [[ ! "$a" =~ ^[Yy] ]]; }
pause() { read -r -p "${dim}Press Enter when done…${reset}" _; }
has_secret() { gh secret list --repo "$REPO" --env "$ENV_NAME" --json name -q '.[].name' 2>/dev/null | grep -qx "$1"; }
set_secret() { printf '%s' "$2" | gh secret set "$1" --repo "$REPO" --env "$ENV_NAME" >/dev/null && ok "secret $1 saved"; }
skip_if_set() {
  local all=1 s
  for s in "$@"; do has_secret "$s" || all=0; done
  # Replacing is the exception: Enter keeps what's there.
  [ "$all" = 1 ] && no "Already set ($*). Replace?" && { ok "kept"; return 0; }
  return 1
}

printf '%sKuzgun release setup%s — signing, notarization, updates, Homebrew.\n' "$bold" "$reset"
printf '%sRepo: %s · Tap: %s · Environment: %s%s\n' "$dim" "$REPO" "$TAP" "$ENV_NAME" "$reset"

# ── 0 ──────────────────────────────────────────────────────────────────────
step "0/6  Tools and access"
command -v gh >/dev/null || fail "Install the GitHub CLI: brew install gh"
gh auth status >/dev/null 2>&1 || fail "Sign in first: gh auth login"
gh repo view "$REPO" >/dev/null 2>&1 || fail "Can't see $REPO with your gh account"
xcrun --find notarytool >/dev/null 2>&1 || fail "Install Xcode (notarytool comes with it)"
ok "gh signed in as $(gh api user -q .login), Xcode tools present"

# ── 1 ──────────────────────────────────────────────────────────────────────
step "1/6  GitHub environment \"$ENV_NAME\""
echo "The signing secrets live in an environment that only v* tags can deploy to."
gh api -X PUT "repos/$REPO/environments/$ENV_NAME" --input - >/dev/null <<'EOF'
{ "deployment_branch_policy": { "protected_branches": false, "custom_branch_policies": true } }
EOF
if ! gh api "repos/$REPO/environments/$ENV_NAME/deployment-branch-policies" -q '.branch_policies[].name' | grep -Fqx 'v*'; then
  gh api -X POST "repos/$REPO/environments/$ENV_NAME/deployment-branch-policies" -f name='v*' -f type=tag >/dev/null
fi
ok "environment ready (deploys only from v* tags)"

# ── 2 ──────────────────────────────────────────────────────────────────────
step "2/6  Developer ID certificate"
if ! skip_if_set MACOS_CERTIFICATE_P12 MACOS_CERTIFICATE_PASSWORD; then
  identity=$(security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application:[^"]*\)".*/\1/p' | head -1)
  [ -n "$identity" ] || fail "No \"Developer ID Application\" certificate in your keychain. Create one at https://developer.apple.com/account/resources/certificates (Developer ID Application), install it, then re-run."
  ok "found: $identity"
  cat <<EOF
Export it as a .p12 (the certificate together with its private key):
  1. Keychain Access opens on "My Certificates".
  2. Right-click "$identity" → Export…
  3. Format: Personal Information Exchange (.p12). Save it to your Desktop.
  4. Set an export password (you'll type it here next).
EOF
  open -a "Keychain Access"
  pause
  p12=$(ask "Path to the .p12 (drag the file here):")
  p12="${p12//\\ / }"; p12="${p12%\"}"; p12="${p12#\"}"; p12="${p12%\'}"; p12="${p12#\'}"; p12="${p12% }"
  [ -f "$p12" ] || fail "No file at $p12"
  p12pass=$(ask_secret "Export password:")
  # Check it imports and holds a Developer ID identity, in a throwaway keychain.
  tmpkc="$(mktemp -d)/check.keychain-db"
  security create-keychain -p x "$tmpkc"
  security import "$p12" -k "$tmpkc" -P "$p12pass" >/dev/null 2>&1 || { security delete-keychain "$tmpkc"; fail "Wrong password, or not a .p12"; }
  security find-identity -v -p codesigning "$tmpkc" | grep -q "Developer ID Application" || { security delete-keychain "$tmpkc"; fail "That .p12 has no Developer ID Application identity (did you export the private key too?)"; }
  security delete-keychain "$tmpkc"
  ok "the .p12 checks out"
  set_secret MACOS_CERTIFICATE_P12 "$(base64 -i "$p12")"
  set_secret MACOS_CERTIFICATE_PASSWORD "$p12pass"
  yes "Delete the exported .p12 now? (it's in GitHub; keep your own backup in 1Password)" && rm -P "$p12" && ok "deleted $p12"
fi

# ── 3 ──────────────────────────────────────────────────────────────────────
step "3/6  Notarization (App Store Connect API key)"
if ! skip_if_set NOTARY_KEY_P8 NOTARY_KEY_ID NOTARY_ISSUER_ID; then
  cat <<'EOF'
Create an API key that can notarize:
  1. App Store Connect → Users and Access → Integrations → App Store Connect API
     (opening it now). Account Holder or Admin can create keys.
  2. "Generate API Key" (Team Keys) · Name: Kuzgun notarization · Access: Developer
  3. Download the .p8 — Apple lets you download it only once.
  4. Note the Key ID (in the row) and the Issuer ID (above the table).
EOF
  open "https://appstoreconnect.apple.com/access/integrations/api"
  pause
  p8=$(ask "Path to AuthKey_XXXXXXXXXX.p8 (drag the file here):")
  p8="${p8//\\ / }"; p8="${p8%\"}"; p8="${p8#\"}"; p8="${p8%\'}"; p8="${p8#\'}"; p8="${p8% }"
  [ -f "$p8" ] || fail "No file at $p8"
  guess=$(basename "$p8" | sed -n 's/^AuthKey_\(.*\)\.p8$/\1/p')
  key_id=$(ask "Key ID${guess:+ [$guess]}:"); key_id="${key_id:-$guess}"
  issuer=$(ask "Issuer ID:")
  echo "Checking the key with Apple…"
  xcrun notarytool history --key "$p8" --key-id "$key_id" --issuer "$issuer" >/dev/null 2>&1 \
    || fail "Apple rejected the key (check the Key ID, the Issuer ID and the key's access)"
  ok "Apple accepts the key"
  set_secret NOTARY_KEY_P8 "$(base64 -i "$p8")"
  set_secret NOTARY_KEY_ID "$key_id"
  set_secret NOTARY_ISSUER_ID "$issuer"
  # The same key for local releases (scripts/release.sh uses this profile).
  xcrun notarytool store-credentials kuzgun --key "$p8" --key-id "$key_id" --issuer "$issuer" >/dev/null \
    && ok "saved as the local keychain profile \"kuzgun\" too"
  warn "Keep the .p8 in 1Password: Apple won't let you download it again."
fi

# ── 4 ──────────────────────────────────────────────────────────────────────
step "4/6  Sparkle update-signing key"
if ! skip_if_set SPARKLE_PRIVATE_KEY; then
  sparkle="$(scripts/fetch-sparkle.sh)"
  # A public key is 44 characters of base64. generate_keys prints its
  # errors on stdout, so an exit code of 0 is not enough.
  valid_key() { [[ "$1" =~ ^[A-Za-z0-9+/]{43}=$ ]]; }
  current=$("$sparkle/bin/generate_keys" --account kuzgun -p 2>/dev/null) || current=""
  valid_key "$current" || current=""
  if [ -z "$current" ]; then
    yes "No Sparkle key named \"kuzgun\" in your keychain. Make one now?" || fail "Kuzgun needs a Sparkle key to sign its updates"
    "$sparkle/bin/generate_keys" --account kuzgun >/dev/null
    current=$("$sparkle/bin/generate_keys" --account kuzgun -p 2>/dev/null) || current=""
    valid_key "$current" || fail "generate_keys made no key (allow keychain access and run the wizard again)"
    ok "made the Sparkle key \"kuzgun\""
  fi
  if [ "$current" != "$PUBLIC_KEY" ]; then
    # A key that installed copies trust is replaced only on an explicit yes.
    if valid_key "$PUBLIC_KEY" && no "assets/sparkle-public-key holds another key. Installed copies trust that one. Replace it?"; then
      fail "Kept assets/sparkle-public-key. Import its private key into your keychain as \"kuzgun\" (generate_keys -f) and run the wizard again."
    fi
    printf '%s\n' "$current" > assets/sparkle-public-key
    warn "wrote assets/sparkle-public-key: commit it, the app bundle reads it"
  fi
  ok "keychain key matches assets/sparkle-public-key"
  tmp="$(mktemp -d)/key"
  echo "Exporting the private key. If macOS asks for keychain access, choose Always Allow."
  if ! out=$("$sparkle/bin/generate_keys" --account kuzgun -x "$tmp" 2>&1) || [ ! -s "$tmp" ]; then
    rm -f "$tmp"
    printf '%s\n' "$out" >&2
    fail "Couldn't export the Sparkle key. Allow keychain access and run the wizard again; finished steps are kept."
  fi
  set_secret SPARKLE_PRIVATE_KEY "$(cat "$tmp")"
  rm -P "$tmp"
  warn "Back up the private key in 1Password (Keychain Access → \"https://sparkle-project.org\" → kuzgun)."
  warn "If it's lost, installed copies can never verify an update again."
fi

# ── 5 ──────────────────────────────────────────────────────────────────────
step "5/6  Homebrew tap ($TAP)"
if ! gh repo view "$TAP" >/dev/null 2>&1; then
  if yes "Create the public repo $TAP for the cask?"; then
    gh repo create "$TAP" --public --description "Homebrew tap for Kuzgun" --add-readme >/dev/null
    ok "created $TAP"
  else
    warn "skipped: the release workflow's Homebrew job will fail until the tap exists"
  fi
else
  ok "$TAP exists"
fi
if ! skip_if_set HOMEBREW_TAP_TOKEN; then
  cat <<EOF
Create a fine-grained token that can push to the tap only:
  1. GitHub → Settings → Developer settings → Fine-grained tokens → Generate (opening it now)
  2. Name: kuzgun-tap · Expiration: 1 year · Resource owner: ${TAP%%/*}
  3. Repository access: Only select repositories → $TAP
  4. Permissions → Repository → Contents: Read and write
EOF
  open "https://github.com/settings/personal-access-tokens/new"
  pause
  token=$(ask_secret "Paste the token:")
  git ls-remote "https://x-access-token:${token}@github.com/${TAP}.git" >/dev/null 2>&1 \
    || fail "The token can't read $TAP"
  ok "token reaches $TAP"
  set_secret HOMEBREW_TAP_TOKEN "$token"
  warn "Put a reminder in your calendar to renew it before it expires."
fi

# ── 6 ──────────────────────────────────────────────────────────────────────
step "6/6  Protect main"
if gh api "repos/$REPO/rulesets" -q '.[].name' 2>/dev/null | grep -qx main; then
  ok "ruleset \"main\" already active"
elif yes "Require the CI gate and pull requests on main (admins can still bypass)?"; then
  gh api -X POST "repos/$REPO/rulesets" --input .github/rulesets/main.json >/dev/null && ok "ruleset \"main\" active"
fi

printf '\n%sDone.%s Secrets in "%s":\n' "$bold" "$reset" "$ENV_NAME"
gh secret list --repo "$REPO" --env "$ENV_NAME"
cat <<EOF

${bold}Cut a release${reset}
  scripts/tag-release.sh 0.1.0        # bump, commit, tag v0.1.0, push
  gh run watch                        # follow the build (~20–30 min with notarization)

The release page gets Kuzgun-<version>-arm64.dmg and appcast.xml; installed
copies find the update within a day, and the tap gets the new cask:
  brew install ${TAP%%/*}/kuzgun/kuzgun
EOF
