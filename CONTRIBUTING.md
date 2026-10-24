# Contributing to whoisport

Thanks for helping. This file covers how to set up, what is checked, how to write
changes that get merged quickly, and how releases are made. For how the code is organised,
read [ARCHITECTURE.md](ARCHITECTURE.md) first.

## Setup

You need [Rust](https://rustup.rs) 1.95 or newer. Then:

```sh
git clone https://github.com/dpologdvinity/whoisport
cd whoisport
cargo build
cargo test
```

The tests start real sockets and small helper processes on your machine (a Python
interpreter is used for some of them: `python3`, or `python` on Windows). They only stop
processes they started themselves.

## The checks (and how to run them)

CI runs all of these on every pull request; run them yourself before pushing.

| What | Command |
|------|---------|
| Formatting | `cargo fmt --check` (fix with `cargo fmt`) |
| Lints and type-check | `cargo clippy --all-targets -- -D warnings` |
| Tests | `cargo test` |
| Documentation | `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items` |
| Spelling | `typos` ([install](https://github.com/crate-ci/typos)) |
| Workflow files | `actionlint` ([install](https://github.com/rhysd/actionlint)) |
| Dependency audit | `cargo audit` |
| Coverage (must stay at or above 90 % of lines) | `cargo llvm-cov --fail-under-lines 90` (needs `cargo install cargo-llvm-cov` and `rustup component add llvm-tools-preview`) |

### Checking the other operating systems

Most of the code is per-platform, and you will usually only be able to run one platform.
You can still type-check and lint the others without owning them:

```sh
rustup target add x86_64-pc-windows-gnu aarch64-apple-darwin
cargo clippy --all-targets --target x86_64-pc-windows-gnu -- -D warnings
cargo clippy --all-targets --target aarch64-apple-darwin -- -D warnings
```

This catches compile errors and lints, not behaviour; CI then runs the real tests on
Linux (x86_64 and arm64), Windows and macOS.

## Writing code

- **Document everything.** Every item, including private ones, needs a `///` comment, and
  every file starts with a `//!` header that says what it is for and why it is shaped
  that way. This is enforced: `clippy::missing_docs_in_private_items` is on and CI fails
  on warnings. Explain *why*, not what the next line does. Every `unsafe` block needs a
  `// SAFETY:` comment.
- **Keep scanning and printing apart.** Anything that touches the operating system lives
  in a `proc*` backend or `docker.rs` and is called from `scan.rs`; `output.rs` and
  `kill.rs` work on plain data so they can be unit-tested.
- **Printing text from other processes?** Run it through `output::shown`. Command lines
  and paths are attacker-controlled.
- **Anything that signals a process** must go through `kill.rs`, keep the pid-plus-start-time
  check, and respect the skip rules (ARCHITECTURE.md, section 6). Do not add a second way
  to kill.
- **Prefer less.** No new dependency for a few lines, no configuration for something that
  has one right answer.

### Adding or changing a platform backend

A backend (`proc.rs`, `proc_macos.rs`, `proc_windows.rs`) must provide exactly the
functions listed in ARCHITECTURE.md, section 5, with the same meaning. If you change one
platform's behaviour, check what the other two do and keep the documented contract the
same, or update the contract and all three.

### Writing tests

- Put unit tests beside the code in a `#[cfg(test)] mod tests`. Parsers and formatters
  take strings and return values, so they need no operating-system access.
- Put end-to-end tests in `tests/cli.rs`. They run the real binary against real sockets.
- Open sockets on port `0` (the system picks one). For "nothing listens here" use the
  `NOBODY_LISTENS` constant (port 1); do not probe for a free port, which is unreliable in
  parallel tests (the reasons are written next to the constant).
- A test that starts a process must stop it, and must never signal anything it did not
  start.
- Gate platform-specific tests with `#[cfg(unix)]` or `#[cfg(windows)]`.
- A bug fix comes with a test that fails without the fix.

## Commits and pull requests

Commits follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<optional scope>): <summary in the imperative, under 72 characters>

<body: what changed and why, wrapped at about 72 columns>
```

Types: `feat`, `fix`, `refactor`, `test`, `docs`, `ci`, `build`, `chore`, `perf`. Keep
commits small and logical, one concern each.

Pull requests are squash-merged, so **the PR title becomes the commit message on
`master`**: write it as a Conventional Commit. Describe what changed, why, and how you
tested it. A PR should pass every CI job (`CI passed` is the single aggregate check).

Please do not commit editor, agent or local-tool files (`.claude/`, `.codex/`,
`CLAUDE.md`, `AGENTS.md`, `.env`); add them to `.git/info/exclude` instead.

## Releases (maintainers)

1. Open a PR `chore: release vX.Y.Z` that changes `version` in `Cargo.toml` (the single
   source of truth: PyPI, npm, `.deb` and `.rpm` versions follow it) and `Cargo.lock`.
   Merge it once CI is green.
2. Tag the merge commit and push the tag: `git tag -a vX.Y.Z -m vX.Y.Z && git push origin
   vX.Y.Z`. `release.yml` checks the tag matches `Cargo.toml`, runs the tests, builds
   every platform into a *draft* release, and only when all builds succeeded publishes it
   and PyPI, and updates the Scoop manifest.
3. Publish the crate: `cargo publish` (then `cargo clean -p whoisport`, so a later build
   does not reuse the packaged copy).
4. Homebrew updates itself within six hours (the tap polls releases). winget needs a pull
   request to `microsoft/winget-pkgs`.

## Reporting a security problem

Please use GitHub's private vulnerability reporting (the repository's **Security** tab)
instead of a public issue. whoisport reads process information and can signal processes,
so problems such as signalling the wrong process, or unescaped output from another
process, are in scope.
