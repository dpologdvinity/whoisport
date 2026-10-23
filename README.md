# whoisport

[![crates.io](https://img.shields.io/crates/v/whoisport.svg)](https://crates.io/crates/whoisport)
[![CI](https://github.com/dpologdvinity/whoisport/actions/workflows/ci.yml/badge.svg)](https://github.com/dpologdvinity/whoisport/actions/workflows/ci.yml)

Find out what is listening on a TCP or UDP port, and optionally kill it.

![whoisport demo](demo.gif)

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

`lsof -i :3000` gives you a PID and a wall of columns. `whoisport` gives you the
command line, the directory it was started from, who owns it and how long it has
been running, which is usually what you need to decide whether to kill it.

## Install

```sh
cargo binstall whoisport   # prebuilt binary, no compiling
cargo install whoisport    # build from source
pipx install whoisport     # or: uvx whoisport, pip install whoisport
npm install -g whoisport   # or: pnpm add -g, yarn global add, bun add -g, npx whoisport
```

macOS or Linux, with [Homebrew](https://brew.sh):

```sh
brew install dpologdvinity/tap/whoisport
```

Debian/Ubuntu or Fedora/RHEL: download the `.deb` or `.rpm` for your architecture
from the [Releases page](https://github.com/dpologdvinity/whoisport/releases), then

```sh
sudo apt install ./whoisport_*.deb     # Debian, Ubuntu
sudo dnf install ./whoisport-*.rpm     # Fedora, RHEL
```

Windows, with [Scoop](https://scoop.sh):

```powershell
scoop bucket add whoisport https://github.com/dpologdvinity/whoisport
scoop install whoisport
```

Or download a binary from the
[Releases page](https://github.com/dpologdvinity/whoisport/releases): Linux (static,
x86_64 and aarch64), macOS (Apple Silicon and Intel) and Windows (x86_64).

Works on **Linux** (reads `/proc` directly, no runtime dependencies), **macOS**
(uses the built-in `lsof`) and **Windows 10/11**.

## Usage

```
whoisport                          # list every listening port
whoisport <PORT>...                # details for one or more ports
whoisport <PORT>... --kill [--force]
whoisport [<PORT>...] --json       # machine-readable output
whoisport [<PORT>...] --udp        # UDP instead of TCP
```

| Flag      | Effect                                                              |
|-----------|---------------------------------------------------------------------|
| `--kill`  | Ask the process to exit and wait up to 3 seconds.                   |
| `--force` | With `--kill`: forcibly terminate it if asking did not work.        |
| `--json`  | Print a JSON array with `port`, `proto`, `pid`, `ppid`, `user`, `cmd`, `cwd`, `uptime_secs`, `container`, `image`. |
| `--udp`   | Look at UDP sockets instead of TCP. UDP has no listening state, so any bound socket without a remote peer counts. Windows doesn't report UDP peers, so there every bound UDP socket is listed, including short-lived client ones. |

Docker: when a port is published by a container, whoisport shows the container
name and image (`docker  web (nginx:alpine)`), even when the `docker-proxy`
process itself is owned by root. It asks `docker ps` only in that case.

`--kill` always needs explicit ports; it never kills everything.

How `--kill` asks, per platform:

| Platform | `--kill`                                                        | `--force`          |
|----------|-----------------------------------------------------------------|--------------------|
| Linux, macOS | `SIGTERM`                                                   | `SIGKILL`          |
| Windows  | Ctrl+C in the process's console, and `WM_CLOSE` to its windows  | `TerminateProcess` |

On Windows, Ctrl+C reaches every program attached to a console, so whoisport only
sends it when the target's console holds nothing but the target, its child
processes and a shell. Otherwise it says so and leaves the choice of `--force` to you.

Exit codes: `0` success, `1` a requested port had nothing listening (or nothing
is listening at all), `2` invalid arguments, `3` a kill failed.

Checks IPv4 and IPv6 TCP sockets in the LISTEN state. On Linux, processes owned
by other users can't be inspected without `sudo`; they show up as `?`. On
Windows, details of system services need an elevated terminal. On macOS,
other users' listeners only appear when run with `sudo`.

## License

MIT
