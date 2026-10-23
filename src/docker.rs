use std::process::Command;

/// Does this command line belong to Docker's port forwarder (Linux proxy or Docker Desktop)?
pub fn is_docker(cmd: &str) -> bool {
    cmd.contains("docker-proxy") || cmd.contains("com.docker.backend")
}

/// (host port, container name, image) for every port published by a running container.
pub fn published(udp: bool) -> Vec<(u16, String, String)> {
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
}
