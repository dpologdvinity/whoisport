//! Scanning: find the listeners and gather everything known about each one.
//!
//! [`Scan::new`] is the only place that talks to the operating system and to Docker;
//! everything after it (output, killing) works on the plain data in [`Scan`], which makes
//! that code easy to test.

use crate::{Info, docker, proc};

/// One listening socket, together with what we could find out about its owner.
pub struct Row {
    /// The port number the socket is bound to.
    pub port: u16,
    /// The owning process id, or `None` when the owner cannot be seen (Linux: a process of
    /// another user, without `sudo`).
    pub pid: Option<u32>,
    /// Details of that process. `None` when `pid` is `None`, or when the process could not
    /// be read (for example it exited since the scan).
    pub info: Option<Info>,
    /// The Docker container publishing this port, when this row is Docker's port forwarder.
    pub container: Option<docker::Container>,
}

/// The result of scanning for listeners.
pub struct Scan {
    /// The listeners to report: every listener, or only those on the requested ports.
    /// Sorted by port; a port can appear more than once (several processes share a socket).
    pub rows: Vec<Row>,
    /// Requested ports that have no listener at all.
    pub missing: Vec<u16>,
}

impl Scan {
    /// Scan the machine. With an empty `ports`, keep every listener; otherwise only those
    /// on the requested ports. `udp` selects UDP instead of TCP.
    pub fn new(ports: &[u16], udp: bool) -> Scan {
        let mut rows: Vec<Row> = proc::listeners(udp)
            .into_iter()
            .filter(|(port, _)| ports.is_empty() || ports.contains(port))
            .map(|(port, pid)| Row {
                port,
                pid,
                info: pid.and_then(proc::info),
                container: None,
            })
            .collect();
        // Docker's port forwarder usually runs as root, so a row we cannot read might be
        // Docker too. Ask Docker only when some row could be its forwarder.
        if rows.iter().any(|r| could_be_docker(&r.info)) {
            let published = docker::published(udp);
            for row in rows.iter_mut().filter(|r| could_be_docker(&r.info)) {
                row.container = published
                    .iter()
                    .find(|(port, _)| *port == row.port)
                    .map(|(_, container)| container.clone());
            }
        }
        let missing = missing_ports(ports, &rows);
        Scan { rows, missing }
    }

    /// The rows for one port, in scan order.
    pub fn rows_for(&self, port: u16) -> Vec<&Row> {
        self.rows.iter().filter(|r| r.port == port).collect()
    }

    /// The exit code before any kill is attempted: `0` when everything asked for was
    /// found, `1` when nothing is listening or some requested port has no listener.
    pub fn exit_code(&self) -> u8 {
        if self.rows.is_empty() || !self.missing.is_empty() {
            1
        } else {
            0
        }
    }
}

/// Could this listener be Docker's port forwarder? True when its process is known to be
/// the forwarder, or when the process is unreadable (the forwarder usually is).
fn could_be_docker(info: &Option<Info>) -> bool {
    info.as_ref().is_none_or(|i| docker::is_docker(&i.cmd))
}

/// The requested ports that have no row.
fn missing_ports(requested: &[u16], rows: &[Row]) -> Vec<u16> {
    requested
        .iter()
        .copied()
        .filter(|port| !rows.iter().any(|r| r.port == *port))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row with nothing known about its owner.
    fn hidden(port: u16) -> Row {
        Row {
            port,
            pid: None,
            info: None,
            container: None,
        }
    }

    /// Details for a process whose command line is `cmd`.
    fn info(cmd: &str) -> Info {
        Info {
            pid: 7,
            ppid: 1,
            user: "u".into(),
            cmd: cmd.into(),
            cwd: "/".into(),
            uptime_secs: 0,
            started: 0,
        }
    }

    #[test]
    fn missing_ports_lists_requested_ports_without_a_row() {
        let rows = [hidden(80), hidden(443)];
        assert_eq!(missing_ports(&[80, 8080, 443, 9], &rows), vec![8080, 9]);
        assert!(missing_ports(&[], &rows).is_empty());
    }

    #[test]
    fn exit_code_is_one_when_empty_or_a_requested_port_is_missing() {
        let found = Scan {
            rows: vec![hidden(80)],
            missing: vec![],
        };
        assert_eq!(found.exit_code(), 0);
        let some_missing = Scan {
            rows: vec![hidden(80)],
            missing: vec![9],
        };
        assert_eq!(some_missing.exit_code(), 1);
        let empty = Scan {
            rows: vec![],
            missing: vec![],
        };
        assert_eq!(empty.exit_code(), 1);
    }

    #[test]
    fn rows_for_returns_every_holder_of_a_port() {
        let scan = Scan {
            rows: vec![hidden(80), hidden(81), hidden(80)],
            missing: vec![],
        };
        assert_eq!(scan.rows_for(80).len(), 2);
        assert!(scan.rows_for(82).is_empty());
    }

    #[test]
    fn only_docker_or_unreadable_rows_could_be_docker() {
        assert!(could_be_docker(&None)); // unreadable: the root-owned forwarder looks like this
        assert!(could_be_docker(&Some(info(
            "/usr/bin/docker-proxy -proto tcp"
        ))));
        assert!(!could_be_docker(&Some(info("python3 -m http.server"))));
    }
}
