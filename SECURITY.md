# Security policy

## Reporting a vulnerability

Please **don't** open a public issue. Report it privately instead: [open a security advisory](https://github.com/alpcanaydin/kuzgun/security/advisories/new). Only the maintainers see it.

Include what you found, how to reproduce it, and the Kuzgun version. We'll reply within a few days, and we'll credit you in the release notes unless you'd rather we didn't.

## Supported versions

Only the latest release gets security fixes. Kuzgun updates itself, so most people already have it.

## How Kuzgun handles your data

- **Kuzgun only reads.** It never writes to a board, a ticket, a repo or an agent's files.
- **Agent sessions** (Claude Code, Codex) are read from their own files on your Mac. Nothing leaves it.
- **Settings, saved boards and view state** are stored in `~/Library/Application Support/kuzgun/`.
- **No telemetry.** Kuzgun doesn't send data anywhere on its own.
- **Updates** are signed with an EdDSA key, and Kuzgun verifies each one before installing it. Every release is also signed with a Developer ID and notarized by Apple.
