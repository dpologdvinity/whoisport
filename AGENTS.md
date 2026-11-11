# AGENTS.md

Instructions for coding agents (Claude Code, Codex and others) working in this repository.
`CLAUDE.md` only imports this file, so this is the single source of truth.

## What this is

`whoisport` is a cross-platform Rust CLI that shows what is listening on a TCP or UDP port
and can ask that process to exit or kill it. It ships as a crate, prebuilt binaries,
PyPI wheels (maturin, `pyproject.toml`), npm packages (`npm/`), `.deb`/`.rpm`, a signed apt
repository (`apt/`) and one-line installers (`install.sh`, `install.ps1`).

Read [ARCHITECTURE.md](ARCHITECTURE.md) before changing code, and
[CONTRIBUTING.md](CONTRIBUTING.md) for the full check table and the release flow.

## Setup and commands

Rust 1.95 or newer (`rust-version` in `Cargo.toml`). Tests need `python3` (`python` on
Windows). No environment variables are needed to build or test.

```sh
cargo build
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps --document-private-items
cargo test --locked
typos
node npm/check.mjs                    # after touching npm/
shellcheck install.sh tests/install_test.sh apt/build.sh tests/apt_test.sh
sh tests/install_test.sh              # after touching install.sh
```

Run fmt, clippy and `cargo test` before calling any change done. CI (`.github/workflows/ci.yml`)
also runs clippy for the Windows and macOS targets, coverage (90 % line minimum) and
`cargo audit`.

Git hooks run through [lefthook](https://lefthook.dev) (`lefthook install`): pre-commit runs
gitleaks on staged files, `cargo fmt --check` and typos; pre-push runs clippy.

## Rules that are easy to miss

- Every item, private ones included, needs a `///` doc comment, and every file starts with a
  `//!` header. CI fails on a missing one (`clippy::missing_docs_in_private_items`).
  Every `unsafe` block needs a `// SAFETY:` comment.
- OS access lives only in the `proc*` backends and `docker.rs`, called from `scan.rs`.
  `output.rs` and `kill.rs` work on plain data.
- Text from other processes goes through `output::shown` before it is printed.
- All signalling goes through `kill.rs` and keeps its pid-plus-start-time check.
- The three backends (`proc.rs`, `proc_macos.rs`, `proc_windows.rs`) share one contract
  (ARCHITECTURE.md, section 5): change all three or none.
- Tests open sockets on port `0`, use `NOBODY_LISTENS` for "nothing listens here", and only
  ever stop processes they started. A bug fix comes with a test that fails without it.
- No new dependency for a few lines of code.

## Commits and releases

Conventional Commits (`feat:`, `fix:`, `docs:`, `ci:`, `chore:` …), small and one concern
each. PRs are squash-merged, so the PR title is the commit message on `master`.
`Cargo.toml` `version` is the only version number; releases are tag-driven
(`release.yml`), see CONTRIBUTING.md.

## Agent setup

- `.claude/settings.json` enables the Claude Code plugins used here: `rust-analyzer-lsp`
  (LSP diagnostics), `commit-commands` (commit and PR flow) and
  [`ponytail`](https://github.com/DietrichGebert/ponytail) (smallest-complete-change
  discipline). `.codex/config.toml` enables `ponytail` for Codex too.
- MCP servers are not configured per project; none are required.
- Personal overrides go in `.claude/settings.local.json` or `CLAUDE.local.md` (gitignored).
