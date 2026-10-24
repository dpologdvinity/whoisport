# Architecture

This document explains how `whoisport` is built and why. For what it does and how to use
it, see [README.md](README.md); for how to work on it, see
[CONTRIBUTING.md](CONTRIBUTING.md). Every source file also starts with a header comment
describing its job, and every item has a doc comment (`cargo doc --open
--document-private-items` renders them).

## 1. Purpose

`whoisport` answers one question, "what is listening on this port?", better than `lsof`,
`ss` or `netstat`, and can then stop that process safely. It is a single command-line
program for Linux, macOS and Windows with no runtime dependencies.

Design goals, in priority order:

1. **Never stop the wrong thing.** Killing is the dangerous part, so most of the design
   exists to make it conservative (section 6).
2. **Give the answer a person needs**: the command line, working directory, owner and
   uptime, not just a pid.
3. **Small and dependable**: one binary, few dependencies, no configuration, no daemon, no
   network access of its own.

## 2. Technology stack

| Concern | Choice |
|---------|--------|
| Language | Rust, edition 2024, minimum Rust 1.95 (`rust-version` in `Cargo.toml`) |
| Argument parsing | [`clap`](https://crates.io/crates/clap) (derive) |
| JSON | [`serde_json`](https://crates.io/crates/serde_json) |
| Linux, macOS signals and system calls | [`libc`](https://crates.io/crates/libc) |
| Process details on Windows and macOS | [`sysinfo`](https://crates.io/crates/sysinfo) |
| Windows system APIs | [`windows-sys`](https://crates.io/crates/windows-sys) (IP Helper, console, windows) |
| Concurrency | none beyond one helper thread per Docker call; the tool is synchronous |
| Tests | built-in `cargo test` (unit tests beside the code, end-to-end tests in `tests/`) |

Dependencies are chosen per platform in `Cargo.toml` (`[target.*.dependencies]`), so a
Linux build compiles neither `sysinfo` nor `windows-sys`.

## 3. Components

```
                         ┌────────────────────────────── main.rs ──────────────────────────────┐
  command line ───────►  │  parse (cli) ─► scan ─┬─► output  (table / details / JSON)          │
                         │                        └─► kill    (only with --kill)               │
                         └──────────────────────────────────────────────────────────────────────┘
                                      │                                    │
                          ┌───────────┴───────────┐                        │
                          ▼                       ▼                        ▼
                  proc backend (one per OS)    docker                proc backend
                  listeners · info             container names       alive · ask_exit · terminate
```

| File | Responsibility |
|------|----------------|
| `src/main.rs` | Entry point. Parses arguments, scans, and dispatches to list mode, JSON mode or detail mode. Defines `Info`, the description of one process. |
| `src/cli.rs` | The arguments (`Args`). clap enforces the flag rules (`--kill` needs a port, `--force` needs `--kill`, `--json` excludes `--kill`). |
| `src/scan.rs` | `Scan::new` is the only place that talks to the operating system and Docker. It produces plain data: `Row` (one listener with its owner and container) and `Scan` (the rows plus the requested ports with no listener). |
| `src/output.rs` | Turns rows into text and JSON. `format_*` and `json_rows` return values (and are unit-tested); `print_*` only write them. Escapes untrusted text. |
| `src/kill.rs` | All stopping logic and its safety rules (section 6). |
| `src/proc.rs` | Linux backend (`/proc`). |
| `src/proc_macos.rs` | macOS backend (`lsof`). |
| `src/proc_windows.rs` | Windows backend (IP Helper API, console, windows). |
| `src/sysinfo_info.rs` | Process details through `sysinfo`, shared by the macOS and Windows backends. |
| `src/unix_kill.rs` | `SIGTERM` / `SIGKILL`, shared by the Linux and macOS backends. |
| `src/docker.rs` | Names the container behind a published port, with a time limit. |
| `tests/cli.rs` | End-to-end tests: run the real binary against real sockets. |
| `install.sh`, `install.ps1` | The one-line installers (section 10). Not part of the crate; attached to every release. |
| `tests/install_test.sh`, `tests/install_test.ps1` | Tests for the installers, against a fake release served from disk. |

The separation that matters: **scanning and printing never mix**. `Scan::new` gathers
everything first; after that, `output` and `kill` work on in-memory data. That is what
makes both of them testable without a network or a second process, and it keeps all
operating-system access in the `proc` backends.

## 4. Data flow

For `whoisport 3000 8080 --kill`:

1. `main` parses the arguments (`cli::Args`) and removes repeated ports
   (`Args::unique_ports`).
2. `Scan::new(ports, udp)` asks the platform backend for `listeners(udp)`, a list of
   `(port, Option<pid>)` pairs. It keeps the requested ports, calls `info(pid)` for each
   to get an `Info`, and, if any row could be Docker's port forwarder, asks `docker` which
   container publishes that port. It also records requested ports with no listener
   (`Scan::missing`).
3. For each requested port, `main` prints the detail block for every row
   (`output::print_details`). Ports with nothing listening are named.
4. With `--kill`, `kill::kill_port` stops the holders of that port, applying the safety
   rules, and reports whether the port can be considered free.
5. The exit code is `0`, `1` (something requested was not found) or `3` (a kill failed or
   a holder was left running); `2` comes from clap for invalid arguments.

List mode (`whoisport` alone) and `--json` skip steps 3 and 4: they print the table or the
JSON array of all rows.

## 5. Platform backends

Exactly one backend is compiled in (selected with `cfg` in `main.rs`) and is imported as
`proc`. All three expose the same functions, so nothing outside them knows which operating
system it is running on.

| Function | Meaning |
|----------|---------|
| `listeners(udp) -> Vec<(u16, Option<u32>)>` | Every listening socket as `(port, owning pid)`, sorted and without duplicates. The pid is `None` when the owner cannot be seen. |
| `info(pid) -> Option<Info>` | Details of one process, or `None` if it cannot be read. |
| `alive(pid) -> bool` | Is the process still running? A zombie counts as gone. |
| `ask_exit(pid) -> io::Result<()>` | The polite request to exit. |
| `terminate(pid) -> io::Result<()>` | The forced stop. |
| `GRACEFUL`, `FORCEFUL` | The names of those two steps, used in messages. |

### Linux (`proc.rs`)

Reads the kernel's `/proc`; needs no other tools.

1. `/proc/net/{tcp,tcp6,udp,udp6}` list every socket with its port and an *inode* (the
   socket's identity). TCP counts state `0A` (LISTEN). UDP has no listen state, so it
   counts state `07` (bound) with no remote peer.
2. Every process lists its open files in `/proc/<pid>/fd/`; a socket appears as
   `socket:[<inode>]`. Matching inodes gives the owning pids (`socket_owners`).
3. `/proc/<pid>/{stat,cmdline,cwd}` and the owner of `/proc/<pid>` give the details.

A process of another user has an unreadable `fd` directory, so its sockets have no owner.
They are reported with `pid = None`, shown as `?`. Reading other users' processes needs
`sudo`. `Info.started` is the start time in clock ticks since boot.

### macOS (`proc_macos.rs`)

There is no `/proc`, and the native socket-to-process API (`libproc`) needs a large
amount of unsafe code. Instead the backend runs the system's own `lsof -F pn` and parses
its field output (`parse_lsof`). Process details come from `sysinfo`. Without `sudo`,
`lsof` only reports your own processes, so other users' listeners are missing rather
than shown as `?` (a documented limitation, marked with a `shortcut:` comment).

### Windows (`proc_windows.rs`)

`GetExtendedTcpTable` / `GetExtendedUdpTable` return every socket with its owning pid, so
no row is ever hidden. Process details come from `sysinfo`.

Asking a Windows program to exit has no single equivalent of `SIGTERM`. `ask_exit` does
two things: it posts `WM_CLOSE` to the program's visible windows (what clicking the X
does; console windows are skipped because closing one closes the whole terminal), and it
sends Ctrl+C to the program's console. Sending a console event requires being attached to
the target's console, which would detach whoisport from its own, so the work is done by a
**helper process**: whoisport re-runs itself with a hidden first argument
(`__whoisport_ctrl_c <pid>`) and no console of its own, and that helper attaches, checks,
sends Ctrl+C and exits. `run_helper_if_requested` is called first in `main` for this.
Forcing is `TerminateProcess`.

## 6. Stopping processes safely

Implemented in `kill.rs`. The rules, in the order they apply:

**1. Identity: pid plus start time.** A pid alone is not an identity; once a process
exits, its pid can be given to an unrelated one. `Info::started` records when the process
began, and `same_process(pid, started)` checks both before every signal and during every
wait. If the original process is gone, whoisport reports `already exited` and sends
nothing.

**2. Skipping.** When several processes hold one socket (a supervisor and its workers, or a
socket handed over by `systemd`), `skip_reason` refuses to signal:

- a service manager: pid 1, `systemd`, `launchd`, `svchost.exe`, `services.exe`;
- any holder that is an ancestor of another holder (`is_ancestor`, bounded to 64 steps so a
  corrupted parent chain cannot loop).

Skipped holders make the port's result "not free": whoisport prints which pid still holds
it and exits `3`, because a supervisor may simply respawn what was stopped. The same
applies to holders whose details could not be read.

**3. Ask, then force.** `kill_with` runs the polite step, waits up to 3 seconds
(`WAIT`), and only with `--force` runs the forced step. A step that returns an error (a
refused signal; on Windows, a shared console) counts as "did not work", so `--force`
still escalates. The polite step is `SIGTERM` (Linux, macOS) or window-close + Ctrl+C
(Windows); the forced step is `SIGKILL` or `TerminateProcess`.

**4. Windows consoles.** Ctrl+C reaches every process attached to a console. The helper
therefore lists the console's processes (`GetConsoleProcessList`) and sends Ctrl+C only if
they are the target, its descendants, whoisport itself, or shells and console hosts that
merely host the target (`is_shell_name`). Otherwise `--kill` fails with a clear message and
leaves the decision to `--force`.

**5. Bad pids.** `unix_kill` refuses pid 0 and pids above `i32::MAX`, because `kill(0, ..)`
signals the caller's whole process group and a negative pid signals another group.

A process listening on several requested ports is stopped once (`kill_port` keeps a list
of handled pids).

## 7. Docker names

Docker publishes container ports through a forwarder process (`docker-proxy`, or
`com.docker.backend` in Docker Desktop). That process runs as root, so on Linux it usually
appears as an unreadable `?` row. `scan.rs` therefore asks Docker whenever a row *could* be
the forwarder (known to be, or unreadable); `docker.rs` runs `docker ps` and matches
published host ports to container names.

Safety against a stuck or foreign daemon:

- Only a **local** endpoint is used (`is_local_endpoint`: `unix://`, `npipe://`, or
  `tcp://` to localhost). With `DOCKER_HOST` or a context pointing to another machine,
  no names are shown rather than another machine's.
- Every `docker` call goes through `run_with_timeout` (2 seconds): the child is polled,
  killed at the deadline, and its output is drained on a thread so a large output cannot
  stall it. A hung daemon therefore costs at most two seconds and never hangs whoisport.

## 8. Output

- **Text** is built by pure functions returning `String`, printed by thin wrappers.
- **Untrusted text** (command lines, paths, user and container names) goes through
  `output::shown`, which escapes control characters, bidirectional-text overrides and
  zero-width characters. A hostile process can otherwise rewrite the terminal (cursor
  moves, OSC 52 clipboard writes) or disguise its name.
- **JSON** (`json_rows`) builds every object from the same keys; unknown values are `null`,
  so a consumer never has to handle missing keys.

## 9. Testing

| Layer | Where | What it covers |
|-------|-------|----------------|
| Unit | `#[cfg(test)]` modules beside the code | Parsers (`/proc/net/*`, `stat`, `lsof`, `docker ps`), formatting, JSON shape, safety rules, the kill steps against real child processes (`kill_with`), the timeout helper |
| End to end | `tests/cli.rs` | The real binary against real sockets: output, exit codes, `--kill`, `--force`, `--json`, `--udp`, shared sockets, hostile command lines, `--help` |
| CI | `.github/workflows/ci.yml` | All of the above on Linux x86_64 and arm64, Windows and macOS; formatting, clippy for three targets, docs, spelling, workflow lint, dependency audit, coverage floor of 90 % of lines (Linux), package check, release build |

Notes for anyone adding tests:

- Kill tests only ever stop children they started.
- A test that needs "nothing listens here" uses port 1. Probing for a free port is
  unreliable: freed ports are reused by parallel tests, a probe that listens leaks its
  socket into children spawned by other test threads, and a probe that connects hangs
  where closed localhost ports drop packets (WSL2). `tests/cli.rs` explains this at
  `NOBODY_LISTENS`.
- Platform-specific behaviour is tested with `cfg(unix)` / `cfg(windows)`; CI is what
  exercises Windows and macOS.

## 10. Build, release and distribution

`ci.yml` gates every change (section 9). `release.yml` runs when a `v*` tag is pushed:

```
create (check tag == Cargo.toml version, run tests, make a DRAFT release)
  ├─► build (Linux x86_64 + aarch64, static musl; also .deb and .rpm)
  ├─► macos (Apple silicon, Intel)
  ├─► windows (zip)
  └─► pypi-wheels / pypi-sdist
        └─► publish  (release becomes public)
              ├─► pypi-publish  (trusted publishing, no token)
              └─► npm           (off unless the NPM_PUBLISH variable is "true")
```

Nothing is public until every build has succeeded. Just before the release is made public, the
`publish` job also uploads `SHA256SUMS` (the checksum of every release file) and the two
installers. After it is public, `installer-smoke` runs the *published* installers on Linux,
macOS and Windows, from the URLs users will use. Channels:

| Channel | How it gets a new version |
|---------|---------------------------|
| GitHub Releases (binaries, `.deb`, `.rpm`, `SHA256SUMS`) | `release.yml` |
| One-line installers (`curl … \| sh`, `irm … \| iex`) | `install.sh` / `install.ps1`, attached to each release by `release.yml`; `releases/latest/download/install.sh` always serves the newest release's script |
| crates.io | manual `cargo publish` after the tag |
| PyPI (wheels that contain the binary, built with maturin) | `release.yml`, trusted publishing |
| Scoop (separate `scoop-whoisport` repository) | its own workflow polls for new releases every 6 hours and updates the manifest from the release's published SHA-256 |
| Homebrew (separate `homebrew-tap` repository) | its own workflow polls for new releases every 6 hours |
| `cargo binstall` | reads `[package.metadata.binstall]` in `Cargo.toml`; uses the release assets |
| winget | manual pull request to `microsoft/winget-pkgs` |
| npm | the packaging is in `npm/`; the publish job is switched off until npm trusted publishing is configured for each package |

### The installers

`install.sh` (POSIX shell) and `install.ps1` (PowerShell 5.1 and 7) do the same five things:
pick the release target for this machine, resolve "latest" from the `/releases/latest`
redirect (no API call, so no rate limit), download the archive and `SHA256SUMS`, **verify the
archive's SHA-256 and refuse to install if it does not match or no checksum is listed**, and
copy the binary into a user-writable directory (no administrator rights). Each script is
wrapped in functions and only runs on its last line, so a download cut off halfway cannot
execute a partial command. The release URL can be overridden (`WHOISPORT_BASE_URL`), which
lets the tests serve a fake release from disk with no network. The shell version is tested by
`tests/install_test.sh` (checksum mismatch, missing checksum file, option handling, target
detection) and the PowerShell version by `tests/install_test.ps1` under both Windows
PowerShell 5.1 and PowerShell 7 (including the `PATH` edit).

## 11. Decisions and trade-offs

- **`lsof` on macOS instead of `libproc`.** Less code and no unsafe FFI, at the cost of
  not seeing other users' sockets without `sudo`. Revisit if that limitation matters.
- **A helper process on Windows.** Attaching to another console would detach whoisport
  from its own; a short-lived child isolates that.
- **Poll, don't subscribe.** After a signal, whoisport polls every 100 ms for up to 3
  seconds. Waiting on process handles would be marginally faster but needs different code
  per platform for no visible gain.
- **Synchronous, no async runtime.** The work is a few file reads or system calls and at
  most two short subprocesses; an async runtime would only add weight.
- **No configuration file.** Behaviour is fully determined by the arguments.
- **Refusing is better than guessing.** When whoisport cannot be sure a kill is safe (a
  shared console, a parent process, an unreadable owner), it stops and says why; the user
  can use `--force` where that makes sense.
- **Linux builds are static (musl).** One binary runs on any distribution. The trade-off
  is that user names come from `/etc/passwd` only, not network directories (a build against
  glibc, such as `cargo install` on Linux, also consults NSS).

## 12. Known limitations

- macOS without `sudo` shows only your own processes.
- Windows cannot report whether a UDP socket is connected, so `--udp` lists client
  sockets there too.
- Containers are named only for Docker (not Podman or other runtimes) and only for a local
  daemon.
- Processes in other network namespaces are visible only through their forwarder.
