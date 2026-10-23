use std::io;
use std::process::Command;
use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System};

pub use crate::sysinfo_info::info;
pub use crate::unix_kill::{FORCEFUL, GRACEFUL, ask_exit, terminate};

/// (port, pid) pairs from `lsof -F pn` output: "p<pid>" lines followed by "n<address>" lines.
/// Connected UDP sockets ("n1.2.3.4:5->6.7.8.9:53") and unbound ones ("n*:*") are skipped.
pub fn parse_lsof(out: &str) -> Vec<(u16, Option<u32>)> {
    let mut pid = None;
    let mut rows = Vec::new();
    for line in out.lines() {
        if let Some(p) = line.strip_prefix('p') {
            pid = p.parse().ok();
        } else if let Some(addr) = line.strip_prefix('n') {
            let port = addr.rsplit(':').next().and_then(|p| p.parse().ok());
            if let (Some(port), Some(pid), false) = (port, pid, addr.contains("->")) {
                rows.push((port, Some(pid)));
            }
        }
    }
    rows
}

/// Every listener as (port, pid), sorted. macOS has no /proc, so this asks the system `lsof`.
// shortcut: without root, lsof only reports the current user's processes, so other users'
// listeners are missing rather than shown as "?"; upgrade to libproc FFI if that matters.
pub fn listeners(udp: bool) -> Vec<(u16, Option<u32>)> {
    let filter: &[&str] = match udp {
        false => &["-iTCP", "-sTCP:LISTEN"],
        true => &["-iUDP"],
    };
    // lsof exits 1 when nothing matches, so read stdout regardless of the status.
    let Ok(out) = Command::new("lsof")
        .args(["-nP", "-F", "pn"])
        .args(filter)
        .output()
    else {
        return Vec::new();
    };
    let mut rows = parse_lsof(&String::from_utf8_lossy(&out.stdout));
    rows.sort();
    rows.dedup(); // the same process often listens on both IPv4 and IPv6
    rows
}

pub fn alive(pid: u32) -> bool {
    if unsafe { libc::kill(pid as i32, 0) } != 0 {
        // ESRCH: gone. EPERM: exists but belongs to someone else.
        return io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
    }
    // A zombie has already exited; it only lingers until its parent reaps it.
    let p = Pid::from_u32(pid);
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[p]),
        true,
        ProcessRefreshKind::nothing(),
    );
    sys.process(p)
        .is_some_and(|proc| proc.status() != ProcessStatus::Zombie)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_lsof_pairs_pids_with_ports() {
        let out = "p123\nf5\nn*:3000\nf6\nn[::1]:3000\np456\nf7\nn127.0.0.1:5353\n\
                   f8\nn10.0.0.2:50000->1.1.1.1:53\nf9\nn*:*\n";
        assert_eq!(
            parse_lsof(out),
            vec![(3000, Some(123)), (3000, Some(123)), (5353, Some(456))]
        );
    }
}
