# whoisport

[![crates.io](https://img.shields.io/crates/v/whoisport.svg)](https://crates.io/crates/whoisport)
[![CI](https://github.com/dpologdvinity/whoisport/actions/workflows/ci.yml/badge.svg)](https://github.com/dpologdvinity/whoisport/actions/workflows/ci.yml)

Find out what is listening on a TCP port, and optionally kill it.

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
```

Or download a static binary (x86_64 or aarch64) from the
[Releases page](https://github.com/dpologdvinity/whoisport/releases).

Linux only for now. It reads `/proc` directly and has no runtime dependencies.

## Usage

```
whoisport                          # list every listening port
whoisport <PORT>...                # details for one or more ports
whoisport <PORT>... --kill [--force]
```

| Flag      | Effect                                                          |
|-----------|-----------------------------------------------------------------|
| `--kill`  | Send SIGTERM and wait up to 3 seconds for the process to exit.  |
| `--force` | With `--kill`: escalate to SIGKILL if SIGTERM did not work.     |

`--kill` always needs explicit ports; it never kills everything.

Exit codes: `0` success, `1` a requested port had nothing listening (or nothing
is listening at all), `2` a kill failed or the arguments were invalid.

Checks IPv4 and IPv6 TCP sockets in the LISTEN state. Processes owned by other
users can't be inspected without `sudo`; they show up as `?`.

## Not yet supported

UDP, macOS, Windows, Docker port-mapping awareness, JSON output.

## License

MIT
