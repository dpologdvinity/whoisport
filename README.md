# whoisport

[![crates.io](https://img.shields.io/crates/v/whoisport.svg)](https://crates.io/crates/whoisport)
[![CI](https://github.com/dpologdvinity/whoisport/actions/workflows/ci.yml/badge.svg)](https://github.com/dpologdvinity/whoisport/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**Find out what is listening on a port, who started it, and stop it safely.**
Works on Linux, macOS and Windows. One small binary, no runtime dependencies.

![whoisport demo](https://raw.githubusercontent.com/dpologdvinity/whoisport/master/demo.gif)

`lsof -i :3000` gives you a PID and a wall of columns. `whoisport` gives you the command
line, the directory it was started from, who owns it and how long it has been running,
which is usually what you need to decide whether to kill it. And when you do kill it,
whoisport makes sure it only stops the process you were looking at.

## Quickstart

```sh
whoisport              # what is listening on this machine?
whoisport 3000         # what is on port 3000, and who started it?
whoisport 3000 --kill  # ask it to exit (add --force if it refuses)
```

```console
$ whoisport 3000
port    3000
pid     373232
parent  373230
user    kaitlyn
cmd     node node_modules/.bin/vite --port 3000
cwd     /home/kaitlyn/site
uptime  4m 12s

$ whoisport 3000 --kill
...
pid 373232: exited after SIGTERM

$ whoisport
PORT   PID      USER       CMD
3000   373232   kaitlyn    node node_modules/.bin/vite --port 3000
5432   ?        ?          ?
8080   41120    kaitlyn    python3 -m http.server 8080
ports marked ? are owned by another user; run with sudo to see it
```

## Install

| Platform | Command |
|----------|---------|
| Any, with Rust | `cargo binstall whoisport` (prebuilt) or `cargo install whoisport` (builds from source) |
| Any, with Python | `pipx install whoisport` or `uvx whoisport` |
| macOS, Linux | `brew install dpologdvinity/tap/whoisport` |
| Windows | `scoop bucket add whoisport https://github.com/dpologdvinity/whoisport` then `scoop install whoisport` |
| Debian, Ubuntu | download the `.deb` from [Releases](https://github.com/dpologdvinity/whoisport/releases), then `sudo apt install ./whoisport_*.deb` |
| Fedora, RHEL | download the `.rpm` from [Releases](https://github.com/dpologdvinity/whoisport/releases), then `sudo dnf install ./whoisport-*.rpm` |

Or download a binary from the [Releases page](https://github.com/dpologdvinity/whoisport/releases):
Linux (static, x86_64 and aarch64), macOS (Apple silicon and Intel) and Windows (x86_64).

**Supported systems:** Linux (reads `/proc`, no other tools needed), macOS (uses the
built-in `lsof`), Windows 10 and 11.

## Usage

```
whoisport [PORTS]... [--kill [--force]] [--json] [--udp]
```

| You type | You get |
|----------|---------|
| `whoisport` | A table of every listening port |
| `whoisport 3000 8080` | Details of what is on each port (repeated ports are shown once) |
| `whoisport 3000 --kill` | The details, then the process is asked to exit and given 3 seconds |
| `whoisport 3000 --kill --force` | Same, but forcibly ended if it does not exit |
| `whoisport --json` | The same data as a JSON array, for scripts |
| `whoisport --udp` | UDP sockets instead of TCP |

`--kill` always needs explicit ports; it never means "kill everything". `--json` cannot
be combined with `--kill`.

### JSON output

`--json` prints an array with one object per listener. Every object has the same keys;
what is unknown (for example the owner of a process you may not inspect) is `null`:

```json
[
  {
    "port": 3000, "proto": "tcp", "pid": 373232, "ppid": 373230, "user": "kaitlyn",
    "cmd": "node node_modules/.bin/vite --port 3000", "cwd": "/home/kaitlyn/site",
    "uptime_secs": 252, "container": null, "image": null
  }
]
```

### Exit codes

| Code | Meaning |
|------|---------|
| `0` | Success: everything asked for was found (and killed, if asked) |
| `1` | A requested port has nothing listening, or nothing is listening at all |
| `2` | Invalid arguments |
| `3` | A kill failed, or a process holding the port was deliberately not killed (see below), so the port may still be in use |

### Reading `?`

On Linux, a process owned by another user cannot be inspected without `sudo`, so its row
shows `?`. On macOS `lsof` only lists your own processes without `sudo`, so other
users' listeners are missing. On Windows, details of system services need an elevated
terminal.

## Stopping processes safely

`--kill` is built so that it cannot take down more than you meant to:

- **The process you saw is the process that is signalled.** Each target is identified by
  its pid *and* its start time. If it exits and the system hands its pid to something
  else, that something else is left alone.
- **Service managers and parents are never signalled.** When several processes share a
  socket (a supervisor and its workers, or `systemd` socket activation), stopping the
  supervisor takes down far more than one port. `systemd`, `launchd` and `svchost.exe`,
  and any process that is a parent of another holder, are skipped. whoisport then says
  which one still holds the port and exits `3`.
- **It asks first.** Each target is asked to exit; it is forced only with `--force`.

| Platform | `--kill` asks by | `--force` |
|----------|------------------|-----------|
| Linux, macOS | `SIGTERM` | `SIGKILL` |
| Windows | Ctrl+C in the process's console, and `WM_CLOSE` to its windows | `TerminateProcess` |

On Windows, Ctrl+C reaches every program attached to a console, so whoisport only sends
it when the target's console holds nothing but the target, its child processes and a
shell. Otherwise it says so and leaves the choice of `--force` to you.

Text from other processes (command lines, paths) is printed with control characters,
bidirectional-text overrides and zero-width characters escaped, so a hostile process
cannot rewrite your terminal or disguise what it is.

## Docker

When a port is published by a container, whoisport shows the container name and image
(`docker  web (nginx:alpine)`), even though the `docker-proxy` process behind it is owned
by root. Docker is consulted only when a listener looks like Docker's forwarder or its
owner cannot be read, only for a local daemon (not an `ssh://` or remote `DOCKER_HOST`),
and each Docker call is cut off after 2 seconds, so a stuck daemon never hangs
whoisport: the names are simply left out. `--kill` on such a port prints the matching
`docker stop <name>` command, because killing the forwarder only breaks the port mapping.

## UDP

UDP has no listening state, so `--udp` lists bound sockets that have no remote peer.
Windows does not report UDP peers, so there every bound UDP socket is listed, including
short-lived client ones.

## How it works

A short version (the long one is in [ARCHITECTURE.md](ARCHITECTURE.md)): whoisport asks
the operating system which sockets are listening and which process owns each one, looks
up details of those processes, and prints them. Each platform has its own small backend
for that (`/proc` on Linux, `lsof` on macOS, the IP Helper API on Windows); everything
else is shared.

```
cli ─► scan ─► output     (list, details, JSON)
         │
         └───► kill       (identity, safety rules, ask-then-force)
         ▲
   proc backend per OS · docker names
```

## Development

```sh
cargo test                 # unit and end-to-end tests
cargo clippy --all-targets -- -D warnings
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full workflow (formatting, coverage,
checking the other platforms, commit style) and [ARCHITECTURE.md](ARCHITECTURE.md) for
how the code is organised.

## License

[MIT](LICENSE)
