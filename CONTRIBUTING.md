# Contributing to Kuzgun

Thanks for helping! Bug reports, ideas and pull requests are all welcome.

## Questions and ideas

- **A question, or an idea you want to talk through:** start a [discussion](https://github.com/alpcanaydin/kuzgun/discussions).
- **A bug:** open an [issue](https://github.com/alpcanaydin/kuzgun/issues/new/choose) with your macOS version, the Kuzgun version (Kuzgun ▸ About Kuzgun) and, if you can, a board that shows it.
- **A security problem:** don't open an issue. See [SECURITY.md](SECURITY.md).

## Development setup

You need macOS 14 or later on Apple Silicon and the Xcode Command Line Tools. `rust-toolchain.toml` pins the Rust version, and rustup installs it on the first build.

```sh
git clone https://github.com/alpcanaydin/kuzgun.git && cd kuzgun
cargo run -- ~/some/repo  # the app, on a repo that uses mattpocock/skills
cargo test                # unit tests
```

The lint tools CI runs are pinned in `mise.toml`. Run `mise install` once, and after that you can run the same checks locally:

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
mise exec -- actionlint && mise exec -- shellcheck --severity=warning scripts/*.sh .github/scripts/*.sh
```

## Pull requests

- Keep a pull request to one change, and say in its description what it changes and why.
- The PR title becomes the commit message on `main` (we squash-merge), so write it as a short sentence in the imperative: "Show the Codex sessions of a ticket".
- The **gate** check has to pass: format, clippy, tests, the workflow linters and a secret scan.
- For UI changes, add a screenshot or a short recording.

## Releases

Maintainers release with `scripts/tag-release.sh <version>`. See [Releasing](README.md#releasing-maintainers) in the README.

By contributing, you agree that your contributions are licensed under the [MIT License](LICENSE).
