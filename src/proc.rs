//! Linux backend: reads the kernel's `/proc` filesystem. No external tools, no libraries.
//!
//! How a port is traced to a process:
//!
//! 1. `/proc/net/{tcp,tcp6,udp,udp6}` list every socket with its port and an *inode*
//!    number (the socket's identity). See [`listen_sockets`].
//! 2. Every process lists its open files in `/proc/<pid>/fd/`; a socket shows up as a
//!    link named `socket:[<inode>]`. Matching inodes gives the owning pids
//!    ([`socket_owners`]).
//! 3. `/proc/<pid>/{stat,cmdline,cwd}` and the owner of `/proc/<pid>` give the details
//!    ([`info`]).
//!
//! A process of another user has an unreadable `/proc/<pid>/fd`, so its sockets have no
//! owner and are reported with `pid = None` (shown as `?`).

use crate::Info;
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::MetadataExt;

// Signals are the same on every Unix, so the kill functions are shared with macOS.
pub use crate::unix_kill::{FORCEFUL, GRACEFUL, ask_exit, terminate};

/// `(port, socket inode)` of the listening sockets in a `/proc/net/{tcp,udp}{,6}` dump.
///
/// Each data line looks like `0: 0100007F:0BB8 00000000:0000 0A ... <inode> ...`: the
/// local address (`IP:PORT` in hex), the remote address, a state code, and later the
/// inode. TCP counts state `0A` (LISTEN). UDP has no listen state, so it counts state
/// `07` (bound) with no remote peer (remote port `0000`); a connected UDP socket is a
/// client, not a server.
pub fn listen_sockets(table: &str, udp: bool) -> Vec<(u16, u64)> {
    table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            let port = u16::from_str_radix(f.get(1)?.rsplit(':').next()?, 16).ok()?;
            let listening = match udp {
                false => f.get(3)? == &"0A",
                true => f.get(3)? == &"07" && f.get(2)?.ends_with(":0000"),
            };
            listening.then(|| Some((port, f.get(9)?.parse().ok()?)))?
        })
        .collect()
}

/// Map each socket inode to the pids that hold it open, for every process whose
/// `/proc/<pid>/fd` we are allowed to read.
///
/// More than one pid can hold a socket (a parent that forked, a supervisor). A process we
/// cannot read, or one that exits during the scan, is skipped; its sockets then have no
/// owner in the map.
fn socket_owners() -> HashMap<u64, Vec<u32>> {
    let mut owners: HashMap<u64, Vec<u32>> = HashMap::new();
    let Ok(dir) = fs::read_dir("/proc") else {
        return owners;
    };
    for entry in dir.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse().ok()) else {
            continue;
        };
        // Unreadable (other users') or exited mid-scan: skip; its sockets show as ownerless.
        let Ok(fds) = fs::read_dir(entry.path().join("fd")) else {
            continue;
        };
        for fd in fds.flatten() {
            let Ok(link) = fs::read_link(fd.path()) else {
                continue;
            };
            let inode = link
                .to_str()
                .and_then(|l| l.strip_prefix("socket:[")?.strip_suffix(']')?.parse().ok());
            if let Some(inode) = inode {
                let pids = owners.entry(inode).or_default();
                if !pids.contains(&pid) {
                    pids.push(pid);
                }
            }
        }
    }
    owners
}

/// Every listener as `(port, pid)`, sorted; `pid` is `None` when the owner is unreadable.
///
/// A process that listens on both IPv4 and IPv6 yields the same pair twice; duplicates are
/// removed.
pub fn listeners(udp: bool) -> Vec<(u16, Option<u32>)> {
    let proto = if udp { "udp" } else { "tcp" };
    let sockets: Vec<(u16, u64)> = ["", "6"]
        .iter()
        .filter_map(|v| fs::read_to_string(format!("/proc/net/{proto}{v}")).ok())
        .flat_map(|t| listen_sockets(&t, udp))
        .collect();
    let owners = socket_owners();
    let mut out: Vec<(u16, Option<u32>)> = sockets
        .iter()
        .flat_map(|(port, inode)| match owners.get(inode) {
            Some(pids) => pids.iter().map(|&p| (*port, Some(p))).collect(),
            None => vec![(*port, None)],
        })
        .collect();
    out.sort();
    out.dedup(); // the same process often listens on both IPv4 and IPv6
    out
}

/// `(parent pid, start time in clock ticks since boot)` from a `/proc/<pid>/stat` line.
///
/// The line is `pid (comm) state ppid ... starttime ...`. The command name `comm` can
/// itself contain spaces and parentheses, so the fields are counted from the *last* `)`.
/// After it, index 1 is the parent pid and index 19 the start time (fields 4 and 22 of
/// proc(5)).
pub fn parse_stat(stat: &str) -> Option<(u32, u64)> {
    // comm (field 2) may contain spaces and parens, so split after the last ')'.
    let rest: Vec<&str> = stat[stat.rfind(')')? + 1..].split_whitespace().collect();
    Some((rest.get(1)?.parse().ok()?, rest.get(19)?.parse().ok()?))
}

/// The user name for a numeric id, from `/etc/passwd`; the number itself if not found
/// (for example users that only exist in LDAP).
fn user_name(uid: u32) -> String {
    fs::read_to_string("/etc/passwd")
        .ok()
        .and_then(|p| {
            p.lines().find_map(|l| {
                let f: Vec<&str> = l.split(':').collect();
                (f.get(2)?.parse::<u32>().ok()? == uid).then(|| f[0].to_string())
            })
        })
        .unwrap_or_else(|| uid.to_string())
}

/// Details of one process, or `None` if it cannot be read (it exited, or is not ours to
/// see).
///
/// `started` is the process start time in clock ticks since boot; the uptime is derived
/// from it and `/proc/uptime`. The command line is read as raw bytes and decoded leniently,
/// because arguments are arbitrary bytes and need not be valid UTF-8.
pub fn info(pid: u32) -> Option<Info> {
    let base = format!("/proc/{pid}");
    let (ppid, start_ticks) = parse_stat(&fs::read_to_string(format!("{base}/stat")).ok()?)?;
    // Arguments are arbitrary bytes, not necessarily UTF-8.
    let cmd =
        String::from_utf8_lossy(&fs::read(format!("{base}/cmdline")).ok()?).replace('\0', " ");
    let cmd = match cmd.trim() {
        "" => fs::read_to_string(format!("{base}/comm"))
            .ok()?
            .trim()
            .to_string(),
        c => c.to_string(),
    };
    let boot_secs: f64 = fs::read_to_string("/proc/uptime")
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    // SAFETY: sysconf only reads a system constant; _SC_CLK_TCK is always valid.
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
    Some(Info {
        pid,
        ppid,
        user: user_name(fs::metadata(&base).ok()?.uid()),
        cmd,
        cwd: fs::read_link(format!("{base}/cwd")).map_or("?".into(), |p| p.display().to_string()),
        uptime_secs: (boot_secs - start_ticks as f64 / hz).max(0.0) as u64,
        started: start_ticks,
    })
}

/// Is the process still running?
///
/// A zombie (state `Z`) has already exited; it only lingers in `/proc` until its parent
/// collects it, so it counts as gone.
pub fn alive(pid: u32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|s| Some(s[s.rfind(')')? + 1..].split_whitespace().next()? != "Z"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    const TABLE: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000:0BB8 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 12345 1 0 100 0 0 10 0
   1: 0100007F:0BB8 0100007F:9C40 01 00000000:00000000 00:00000000 00000000  1000        0 67890 1 0 100 0 0 10 0
   2: 00000000:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 11111 1 0 100 0 0 10 0
";

    #[test]
    fn listen_sockets_keeps_only_listen_state() {
        // 0x0BB8 = 3000, 0x1F90 = 8080; the ESTABLISHED row (inode 67890) is skipped
        assert_eq!(
            listen_sockets(TABLE, false),
            vec![(3000, 12345), (8080, 11111)]
        );
    }

    #[test]
    fn listen_sockets_parses_ipv6_rows() {
        // /proc/net/tcp6: 32 hex digits of address; [::]:5432 listening, [::1]:443 established
        let t = "  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000000000000:1538 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000   999        0 33333 1 0 100 0 0 10 0
   1: 00000000000000000000000001000000:01BB 00000000000000000000000001000000:D431 01 00000000:00000000 00:00000000 00000000  1000        0 44444 1 0 100 0 0 10 0
";
        assert_eq!(listen_sockets(t, false), vec![(5432, 33333)]);
    }

    #[test]
    fn udp_keeps_only_unconnected_bound_sockets() {
        let t = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
 1: 3500007F:0035 00000000:0000 07 00000000:00000000 00:00000000 00000000   991        0 12444 2 0 0
 2: 0100007F:D431 0100007F:0035 01 00000000:00000000 00:00000000 00000000  1000        0 22222 2 0 0
";
        // port 53 server kept; connected client socket (state 01, remote :0035) dropped
        assert_eq!(listen_sockets(t, true), vec![(53, 12444)]);
    }

    #[test]
    fn parse_stat_handles_parens_in_comm() {
        let stat = "42 (my (weird) app) S 7 42 42 0 -1 4194560 0 0 0 0 0 0 0 0 20 0 1 0 99999 1 2";
        assert_eq!(parse_stat(stat), Some((7, 99999)));
    }

    #[test]
    fn finds_own_listener() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        assert!(listeners(false).contains(&(port, Some(std::process::id()))));
        assert_eq!(info(std::process::id()).unwrap().pid, std::process::id());
    }
}
