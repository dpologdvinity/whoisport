//! Naming the Docker container behind a published port.
//!
//! Docker publishes a container port on the host through a forwarder process
//! (`docker-proxy` on Linux, `com.docker.backend` in Docker Desktop). That process alone
//! says nothing about *which* container it serves, so we ask the local Docker daemon
//! (`docker ps`). The lookup is best-effort: if Docker is missing, down, remote, or too
//! slow, the names are simply left out and everything else still works.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A running Docker container that publishes a host port.
#[derive(Clone)]
pub struct Container {
    /// The container's name, e.g. `web`.
    pub name: String,
    /// The image it runs, e.g. `nginx:alpine`.
    pub image: String,
}

/// How long a `docker` call may take. A stuck daemon (Docker Desktop right after wake,
/// a dead remote) would otherwise hang whoisport, the tool people reach for when
/// something is stuck.
const DOCKER_TIMEOUT: Duration = Duration::from_secs(2);

/// Run `program args...` and return its stdout, or `None` if it cannot start, exits
/// non-zero, or is still running after `timeout` (in which case it is killed).
fn run_with_timeout(program: &str, args: &[&str], timeout: Duration) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // Drain stdout on its own thread so a large output cannot fill the pipe and stall
    // the child while we poll for its exit.
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        let _ = tx.send(String::from_utf8_lossy(&bytes).into_owned());
    });
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let time_left = deadline.saturating_duration_since(Instant::now());
                let out = rx.recv_timeout(time_left).ok()?;
                return status.success().then_some(out);
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            // Timed out (or wait failed): kill it. The reader thread ends when the pipe
            // closes; it is not joined, in case a grandchild still holds the pipe open.
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// `docker args...` with the standard timeout.
fn docker(args: &[&str]) -> Option<String> {
    run_with_timeout("docker", args, DOCKER_TIMEOUT)
}

/// Does this command line belong to Docker's port forwarder (Linux proxy or Docker Desktop)?
pub fn is_docker(cmd: &str) -> bool {
    cmd.contains("docker-proxy") || cmd.contains("com.docker.backend")
}

/// Is this Docker endpoint a daemon on this machine? Only a local daemon's containers
/// can be publishing this machine's ports.
pub fn is_local_endpoint(host: &str) -> bool {
    let tcp_host = host
        .strip_prefix("tcp://")
        .map(|rest| rest.rsplit_once(':').map_or(rest, |(h, _)| h));
    host.starts_with("unix://")
        || host.starts_with("npipe://")
        || matches!(tcp_host, Some("localhost" | "127.0.0.1" | "[::1]"))
}

/// The endpoint `docker` would talk to: $DOCKER_HOST, else the current context's.
fn endpoint() -> Option<String> {
    if let Ok(host) = std::env::var("DOCKER_HOST") {
        return Some(host);
    }
    let out = docker(&[
        "context",
        "inspect",
        "--format",
        "{{.Endpoints.docker.Host}}",
    ])?;
    Some(out.trim().to_string())
}

/// Every host port published by a running container, with that container.
///
/// Empty when the Docker daemon is not local, not running, not allowed, or too slow.
pub fn published(udp: bool) -> Vec<(u16, Container)> {
    // A remote context (ssh://, a remote tcp:// host) lists another machine's containers.
    if !endpoint().is_some_and(|e| is_local_endpoint(&e)) {
        return Vec::new();
    }
    // docker missing, daemon down, no permission, or too slow: just skip the names.
    let Some(out) = docker(&["ps", "--format", "{{.Names}}\t{{.Image}}\t{{.Ports}}"]) else {
        return Vec::new();
    };
    parse_ps(&out, udp)
}

/// Parse `docker ps --format '{{.Names}}\t{{.Image}}\t{{.Ports}}'` output into
/// `(host port, container)` pairs for the requested protocol.
fn parse_ps(output: &str, udp: bool) -> Vec<(u16, Container)> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            Some((fields.next()?, fields.next()?, fields.next()?))
        })
        .flat_map(|(name, image, ports)| {
            host_ports(ports, udp).into_iter().map(|port| {
                let container = Container {
                    name: name.to_string(),
                    image: image.to_string(),
                };
                (port, container)
            })
        })
        .collect()
}

/// Host ports from a `docker ps` Ports column, for TCP or UDP.
///
/// For example `"0.0.0.0:8080->80/tcp, [::]:8080->80/tcp, 0.0.0.0:9000-9001->9000-9001/udp,
/// 5432/tcp"` gives `[8080]` for TCP and `[9000, 9001]` for UDP. Ports a container only
/// exposes internally (`5432/tcp`, no `->`) are not on the host, so they are left out.
pub fn host_ports(ports: &str, udp: bool) -> Vec<u16> {
    let proto = if udp { "/udp" } else { "/tcp" };
    let mut out: Vec<u16> = ports
        .split(", ")
        .filter(|p| p.ends_with(proto))
        .filter_map(|p| p.split_once("->")) // unpublished ports have no "->"
        .filter_map(|(host, _)| host.rsplit_once(':'))
        .filter_map(|(_, range)| {
            let (lo, hi) = range.split_once('-').unwrap_or((range, range));
            Some(lo.parse::<u16>().ok()?..=hi.parse::<u16>().ok()?)
        })
        .flatten()
        .collect();
    out.sort();
    out.dedup(); // IPv4 and IPv6 bindings list the same port twice
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ps_pairs_each_host_port_with_its_container() {
        let out = "web\tnginx:alpine\t0.0.0.0:8080->80/tcp, [::]:8080->80/tcp\n\
                   db\tpostgres\t5432/tcp\n\
                   dns\tcoredns\t0.0.0.0:5353->53/udp\n";
        let tcp = parse_ps(out, false);
        assert_eq!(tcp.len(), 1); // db exposes 5432 only inside Docker
        assert_eq!(tcp[0].0, 8080);
        assert_eq!(tcp[0].1.name, "web");
        assert_eq!(tcp[0].1.image, "nginx:alpine");
        let udp = parse_ps(out, true);
        assert_eq!(udp.len(), 1);
        assert_eq!((udp[0].0, udp[0].1.name.as_str()), (5353, "dns"));
        assert!(parse_ps("not a valid line\n", false).is_empty());
    }

    #[test]
    fn is_docker_matches_the_forwarder_processes() {
        assert!(is_docker(
            "/usr/bin/docker-proxy -proto tcp -host-port 8080"
        ));
        assert!(is_docker("com.docker.backend"));
        assert!(!is_docker("python3 -m http.server"));
    }

    #[test]
    fn host_ports_parses_docker_ps_column() {
        let p =
            "0.0.0.0:8080->80/tcp, [::]:8080->80/tcp, 0.0.0.0:9000-9001->9000-9001/udp, 5432/tcp";
        assert_eq!(host_ports(p, false), vec![8080]);
        assert_eq!(host_ports(p, true), vec![9000, 9001]);
        assert!(host_ports("", false).is_empty());
    }

    /// A program that prints "hi", a failing one, and one that outlives any timeout.
    #[cfg(unix)]
    const ECHO: (&str, &[&str]) = ("echo", &["hi"]);
    #[cfg(unix)]
    const FAIL: (&str, &[&str]) = ("false", &[]);
    #[cfg(unix)]
    const SLOW: (&str, &[&str]) = ("sleep", &["30"]);
    #[cfg(windows)]
    const ECHO: (&str, &[&str]) = ("cmd", &["/c", "echo hi"]);
    #[cfg(windows)]
    const FAIL: (&str, &[&str]) = ("cmd", &["/c", "exit 1"]);
    #[cfg(windows)]
    const SLOW: (&str, &[&str]) = ("ping", &["-n", "30", "127.0.0.1"]);

    #[test]
    fn run_with_timeout_returns_stdout_of_a_successful_command() {
        let out = run_with_timeout(ECHO.0, ECHO.1, Duration::from_secs(10)).unwrap();
        assert_eq!(out.trim(), "hi");
    }

    #[test]
    fn run_with_timeout_returns_none_for_failure_or_a_missing_program() {
        assert_eq!(
            run_with_timeout(FAIL.0, FAIL.1, Duration::from_secs(10)),
            None
        );
        let missing = run_with_timeout("whoisport-no-such-program", &[], Duration::from_secs(10));
        assert_eq!(missing, None);
    }

    #[test]
    fn run_with_timeout_gives_up_on_a_command_that_hangs() {
        let start = Instant::now();
        assert_eq!(
            run_with_timeout(SLOW.0, SLOW.1, Duration::from_millis(300)),
            None
        );
        // It must return near the timeout, not after the 30 s the command wanted.
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{:?}",
            start.elapsed()
        );
    }

    #[test]
    fn only_local_endpoints_count() {
        assert!(is_local_endpoint("unix:///var/run/docker.sock"));
        assert!(is_local_endpoint("npipe:////./pipe/docker_engine"));
        assert!(is_local_endpoint("tcp://127.0.0.1:2375"));
        assert!(is_local_endpoint("tcp://localhost:2376"));
        assert!(!is_local_endpoint("ssh://me@build-box"));
        assert!(!is_local_endpoint("tcp://10.0.0.5:2376"));
        assert!(!is_local_endpoint(""));
    }
}
