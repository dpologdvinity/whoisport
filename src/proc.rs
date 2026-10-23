use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;

pub struct Info {
    pub pid: u32,
    pub ppid: u32,
    pub user: String,
    pub cmd: String,
    pub cwd: String,
    pub uptime_secs: u64,
}

/// Socket inodes of TCP sockets in LISTEN state on `port`, from a /proc/net/tcp{,6} dump.
pub fn listen_inodes(table: &str, port: u16) -> Vec<u64> {
    table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            let local_port = u16::from_str_radix(f.get(1)?.rsplit(':').next()?, 16).ok()?;
            (f.get(3)? == &"0A" && local_port == port).then(|| f.get(9)?.parse().ok())?
        })
        .collect()
}

/// PIDs owning any of `inodes`, plus whether some processes were unreadable (other users').
pub fn pids_for_inodes(inodes: &[u64]) -> (Vec<u32>, bool) {
    let targets: Vec<String> = inodes.iter().map(|i| format!("socket:[{i}]")).collect();
    let mut pids = Vec::new();
    let mut hidden = false;
    let Ok(dir) = fs::read_dir("/proc") else { return (pids, hidden) };
    for entry in dir.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        match fs::read_dir(entry.path().join("fd")) {
            Ok(fds) => {
                let owns = fds.flatten().any(|fd| {
                    fs::read_link(fd.path()).is_ok_and(|l| targets.iter().any(|t| l.to_str() == Some(t)))
                });
                if owns {
                    pids.push(pid);
                }
            }
            Err(e) if e.kind() == ErrorKind::PermissionDenied => hidden = true,
            Err(_) => {} // process exited mid-scan
        }
    }
    (pids, hidden)
}

pub fn find_listeners(port: u16) -> (Vec<u32>, bool) {
    let inodes: Vec<u64> = ["/proc/net/tcp", "/proc/net/tcp6"]
        .iter()
        .filter_map(|p| fs::read_to_string(p).ok())
        .flat_map(|t| listen_inodes(&t, port))
        .collect();
    if inodes.is_empty() {
        return (vec![], false);
    }
    pids_for_inodes(&inodes)
}

/// (ppid, start time in clock ticks since boot) from a /proc/<pid>/stat line.
pub fn parse_stat(stat: &str) -> Option<(u32, u64)> {
    // comm (field 2) may contain spaces and parens, so split after the last ')'.
    let rest: Vec<&str> = stat[stat.rfind(')')? + 1..].split_whitespace().collect();
    Some((rest.get(1)?.parse().ok()?, rest.get(19)?.parse().ok()?))
}

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

pub fn info(pid: u32) -> Option<Info> {
    let base = format!("/proc/{pid}");
    let (ppid, start_ticks) = parse_stat(&fs::read_to_string(format!("{base}/stat")).ok()?)?;
    let cmd = fs::read_to_string(format!("{base}/cmdline")).ok()?.replace('\0', " ");
    let cmd = match cmd.trim() {
        "" => fs::read_to_string(format!("{base}/comm")).ok()?.trim().to_string(),
        c => c.to_string(),
    };
    let boot_secs: f64 = fs::read_to_string("/proc/uptime").ok()?.split_whitespace().next()?.parse().ok()?;
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
    Some(Info {
        pid,
        ppid,
        user: user_name(fs::metadata(&base).ok()?.uid()),
        cmd,
        cwd: fs::read_link(format!("{base}/cwd")).map_or("?".into(), |p| p.display().to_string()),
        uptime_secs: (boot_secs - start_ticks as f64 / hz).max(0.0) as u64,
    })
}

pub fn alive(pid: u32) -> bool {
    fs::metadata(format!("/proc/{pid}")).is_ok()
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
    fn listen_inodes_matches_port_and_state() {
        assert_eq!(listen_inodes(TABLE, 3000), vec![12345]); // 0x0BB8; ESTABLISHED row 67890 skipped
        assert_eq!(listen_inodes(TABLE, 8080), vec![11111]);
        assert!(listen_inodes(TABLE, 1).is_empty());
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
        let (pids, _) = find_listeners(port);
        assert!(pids.contains(&std::process::id()));
        assert_eq!(info(std::process::id()).unwrap().pid, std::process::id());
    }
}
