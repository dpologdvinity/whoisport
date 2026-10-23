# whoisport

Find out what is listening on a TCP port, and optionally kill it.

```console
$ whoisport 3000
pid     373232
parent  373230
user    kaitlyn
cmd     python3 -m http.server 3000
cwd     /home/kaitlyn/site
uptime  4m 12s

$ whoisport 3000 --kill
...
pid 373232: exited after SIGTERM
```

`lsof -i :3000` gives you a PID and a wall of columns. `whoisport` gives you the
command line, the directory it was started from, who owns it and how long it has
been running, which is usually what you need to decide whether to kill it.

## Install

```sh
cargo install whoisport
```

Or download a prebuilt binary from the Releases page.

Linux only for now. It reads `/proc` directly and has no runtime dependencies.

## Usage

```
whoisport <PORT> [--kill [--force]]
```

| Flag      | Effect                                                          |
|-----------|-----------------------------------------------------------------|
| `--kill`  | Send SIGTERM and wait up to 3 seconds for the process to exit.  |
| `--force` | With `--kill`: escalate to SIGKILL if SIGTERM did not work.     |

Exit codes: `0` found (and killed, if asked), `1` nothing listening, `2` a kill failed.

Checks IPv4 and IPv6 TCP sockets in the LISTEN state. Processes owned by other
users are invisible without `sudo`; when nothing is found, `whoisport` says so.

## Not yet supported

UDP, macOS, Windows, Docker port-mapping awareness, JSON output.

## License

MIT
