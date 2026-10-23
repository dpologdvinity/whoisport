use std::process::Command;

/// Does this command line belong to Docker's port forwarder (Linux proxy or Docker Desktop)?
pub fn is_docker(cmd: &str) -> bool {
    cmd.contains("docker-proxy") || cmd.contains("com.docker.backend")
}

/// (host port, container name, image) for every port published by a running container.
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
    let out = Command::new("docker")
        .args([
            "context",
            "inspect",
            "--format",
            "{{.Endpoints.docker.Host}}",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn published(udp: bool) -> Vec<(u16, String, String)> {
    // A remote context (ssh://, a remote tcp:// host) lists another machine's containers.
    if !endpoint().is_some_and(|e| is_local_endpoint(&e)) {
        return Vec::new();
    }
    let out = Command::new("docker")
        .args(["ps", "--format", "{{.Names}}\t{{.Image}}\t{{.Ports}}"])
        .output();
    let Some(out) = out.ok().filter(|o| o.status.success()) else {
        return Vec::new(); // docker missing, daemon down, or no permission: just skip names
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut f = l.splitn(3, '\t');
            Some((f.next()?, f.next()?, f.next()?))
        })
        .flat_map(|(name, image, ports)| {
            host_ports(ports, udp)
                .into_iter()
                .map(|p| (p, name.to_string(), image.to_string()))
        })
        .collect()
}

/// Host ports from a `docker ps` Ports column, e.g.
/// "0.0.0.0:8080->80/tcp, [::]:8080->80/tcp, 0.0.0.0:9000-9001->9000-9001/udp, 5432/tcp".
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
    fn host_ports_parses_docker_ps_column() {
        let p =
            "0.0.0.0:8080->80/tcp, [::]:8080->80/tcp, 0.0.0.0:9000-9001->9000-9001/udp, 5432/tcp";
        assert_eq!(host_ports(p, false), vec![8080]);
        assert_eq!(host_ports(p, true), vec![9000, 9001]);
        assert!(host_ports("", false).is_empty());
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
