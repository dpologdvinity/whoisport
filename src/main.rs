#[cfg(target_os = "linux")]
mod proc;
#[cfg(windows)]
#[path = "proc_windows.rs"]
mod proc;
#[cfg(target_os = "macos")]
#[path = "proc_macos.rs"]
mod proc;
#[cfg(any(windows, target_os = "macos"))]
mod sysinfo_info;
#[cfg(unix)]
mod unix_kill;

mod docker;

use clap::Parser;
use serde_json::json;
use std::process::ExitCode;
use std::time::{Duration, Instant};

pub struct Info {
    pub pid: u32,
    pub ppid: u32,
    pub user: String,
    pub cmd: String,
    pub cwd: String,
    pub uptime_secs: u64,
    /// Process start time (platform units). With the pid it identifies one process,
    /// so a pid that is reused after the target exits is never mistaken for it.
    pub started: u64,
}

/// Is `pid` still the process that started at `started`?
fn same_process(pid: u32, started: u64) -> bool {
    proc::alive(pid) && proc::info(pid).is_some_and(|i| i.started == started)
}

#[derive(Parser)]
#[command(version, about = "Show what is listening on a TCP or UDP port")]
struct Args {
    /// Port(s) to inspect; omit to list every listening port
    ports: Vec<u16>,
    /// Ask the listener(s) on the given port(s) to exit and wait up to 3s
    #[arg(long, requires = "ports")]
    kill: bool,
    /// With --kill: forcibly terminate if asking did not work
    #[arg(long, requires = "kill")]
    force: bool,
    /// Print results as a JSON array
    #[arg(long, conflicts_with = "kill")]
    json: bool,
    /// Look at UDP instead of TCP (bound sockets with no remote peer; on Windows, every bound socket)
    #[arg(long)]
    udp: bool,
}

/// (container name, image) of the container publishing a host port.
type Container = (String, String);

const HIDDEN: &str = "owned by another user; run with sudo to see it";

fn fmt_duration(s: u64) -> String {
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m {}s", s / 60, s % 60),
        3600..=86399 => format!("{}h {}m", s / 3600, s % 3600 / 60),
        _ => format!("{}d {}h", s / 86400, s % 86400 / 3600),
    }
}

/// Text from other processes (argv, paths) can contain escape sequences that would
/// rewrite the terminal; show control characters escaped instead.
fn shown(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c.is_control() {
            true => c.escape_default().collect::<Vec<_>>(),
            false => vec![c],
        })
        .collect()
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max - 1).chain(['…']).collect()
}

fn wait_exit(pid: u32, started: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if !same_process(pid, started) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    !same_process(pid, started)
}

type KillStep = fn(u32) -> std::io::Result<()>;

fn kill(pid: u32, started: u64, force: bool) -> bool {
    let steps: [(KillStep, &str); 2] = [
        (proc::ask_exit, proc::GRACEFUL),
        (proc::terminate, proc::FORCEFUL),
    ];
    for (step, name) in steps.into_iter().take(if force { 2 } else { 1 }) {
        // Re-check right before each signal: the pid may have been reused meanwhile.
        if !same_process(pid, started) {
            println!("pid {pid}: already exited");
            return true;
        }
        if let Err(e) = step(pid) {
            eprintln!("pid {pid}: {name} failed: {e}");
            return false;
        }
        if wait_exit(pid, started) {
            println!("pid {pid}: exited after {name}");
            return true;
        }
    }
    if force {
        eprintln!("pid {pid}: still running");
    } else {
        eprintln!(
            "pid {pid}: still running (use --force for {})",
            proc::FORCEFUL
        );
    }
    false
}

/// Is this the system or user service manager? It can hold a socket it activates.
fn is_service_manager(pid: u32, cmd: &str) -> bool {
    let exe = cmd.split_whitespace().next().unwrap_or("");
    let name = exe.rsplit(['/', '\\']).next().unwrap_or(exe);
    // svchost.exe and services.exe host many Windows services (RPC on 135, DNS, ...).
    let name = name.to_ascii_lowercase();
    pid == 1
        || matches!(
            name.as_str(),
            "systemd" | "launchd" | "svchost.exe" | "services.exe"
        )
}

/// Is `ancestor` somewhere up `pid`'s parent chain?
fn is_ancestor(ancestor: u32, mut pid: u32) -> bool {
    for _ in 0..64 {
        match proc::info(pid).map(|i| i.ppid) {
            Some(parent) if parent == ancestor => return true,
            Some(parent) if parent > 1 && parent != pid => pid = parent,
            _ => return false,
        }
    }
    false
}

/// Why a holder of the socket must not be signalled. Several processes can hold one
/// listening socket when a supervisor or socket-activating service manager hands it to
/// the real server; signalling the supervisor can take down far more than the server.
fn skip_reason(i: &Info, holders: &[u32]) -> Option<String> {
    if is_service_manager(i.pid, &i.cmd) {
        return Some("it is a service manager".into());
    }
    let child = holders
        .iter()
        .find(|&&other| other != i.pid && is_ancestor(i.pid, other))?;
    Some(format!(
        "it is a parent of pid {child}, which also holds this port"
    ))
}

fn to_json(
    port: u16,
    proto: &str,
    pid: Option<u32>,
    info: &Option<Info>,
    c: Option<&Container>,
) -> serde_json::Value {
    let (container, image) = c.map_or((None, None), |(n, i)| (Some(n), Some(i)));
    match info {
        Some(i) => json!({
            "port": port, "proto": proto, "pid": i.pid, "ppid": i.ppid, "user": i.user,
            "cmd": i.cmd, "cwd": i.cwd, "uptime_secs": i.uptime_secs,
            "container": container, "image": image,
        }),
        None => json!({
            "port": port, "proto": proto, "pid": pid, "ppid": null, "user": null,
            "cmd": null, "cwd": null, "uptime_secs": null,
            "container": container, "image": image,
        }),
    }
}

fn main() -> ExitCode {
    #[cfg(windows)]
    proc::run_helper_if_requested();

    let args = Args::parse();
    let proto = if args.udp { "udp" } else { "tcp" };
    let label = |port: u16| match args.udp {
        true => format!("{port}/udp"),
        false => port.to_string(),
    };
    let all = proc::listeners(args.udp);
    let rows: Vec<(u16, Option<u32>, Option<Info>)> = all
        .into_iter()
        .filter(|(port, _)| args.ports.is_empty() || args.ports.contains(port))
        .map(|(port, pid)| (port, pid, pid.and_then(proc::info)))
        .collect();
    let missing: Vec<u16> = args
        .ports
        .iter()
        .copied()
        .filter(|p| !rows.iter().any(|r| r.0 == *p))
        .collect();
    // Docker's port forwarder usually runs as root, so a hidden owner may be docker too.
    // Only ask docker when some listener could be its forwarder.
    let maybe_docker =
        |info: &Option<Info>| info.as_ref().is_none_or(|i| docker::is_docker(&i.cmd));
    let published = match rows.iter().any(|r| maybe_docker(&r.2)) {
        true => docker::published(args.udp),
        false => Vec::new(),
    };
    let container = |port: u16, info: &Option<Info>| -> Option<Container> {
        if !maybe_docker(info) {
            return None;
        }
        let (_, name, image) = published.iter().find(|(p, _, _)| *p == port)?;
        Some((name.clone(), image.clone()))
    };
    let mut code = if rows.is_empty() || !missing.is_empty() {
        1
    } else {
        0
    };

    if args.json {
        let out: Vec<_> = rows
            .iter()
            .map(|(port, pid, i)| to_json(*port, proto, *pid, i, container(*port, i).as_ref()))
            .collect();
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return ExitCode::from(code);
    }

    if args.ports.is_empty() {
        if rows.is_empty() {
            println!("nothing listening");
            return ExitCode::from(1);
        }
        let w = if args.udp { 10 } else { 6 };
        println!("{:<w$} {:<8} {:<10} CMD", "PORT", "PID", "USER");
        for (port, _, info) in &rows {
            let port_label = label(*port);
            match info {
                Some(i) => {
                    let cmd = match container(*port, info) {
                        Some((name, image)) => format!("docker: {name} ({image})"),
                        None => i.cmd.clone(),
                    };
                    println!(
                        "{port_label:<w$} {:<8} {:<10} {}",
                        i.pid,
                        truncate(&shown(&i.user), 10),
                        truncate(&shown(&cmd), 80)
                    )
                }
                None => {
                    let cmd = container(*port, info).map_or("?".into(), |(name, image)| {
                        format!("docker: {name} ({image})")
                    });
                    println!("{port_label:<w$} {:<8} {:<10} {}", "?", "?", shown(&cmd))
                }
            }
        }
        if rows.iter().any(|r| r.2.is_none()) {
            eprintln!("ports marked ? are {HIDDEN}");
        }
        return ExitCode::from(0);
    }

    let mut killed = Vec::new();
    for port in &args.ports {
        if missing.contains(port) {
            println!("nothing listening on :{}\n", label(*port));
        }
        let holders: Vec<u32> = rows
            .iter()
            .filter(|r| r.0 == *port)
            .filter_map(|r| r.2.as_ref().map(|i| i.pid))
            .collect();
        let mut skipped: Vec<u32> = Vec::new();
        for (_, pid, info) in rows.iter().filter(|r| r.0 == *port) {
            let docker_line = container(*port, info).map_or(String::new(), |(n, img)| {
                format!("docker  {} ({})\n", shown(&n), shown(&img))
            });
            // Killing docker-proxy only breaks the port mapping; the container keeps running.
            if let (true, Some((name, _))) = (args.kill, container(*port, info)) {
                let name = shown(&name);
                eprintln!(
                    "port {}: published by container {name}; to stop it, run: docker stop {name}",
                    label(*port)
                );
            }
            let Some(i) = info else {
                let why = match pid {
                    Some(p) => format!("{p} (details unavailable; it may have just exited)"),
                    None => format!("? ({HIDDEN})"),
                };
                println!("port    {}\npid     {why}\n{docker_line}", label(*port));
                if args.kill {
                    eprintln!(
                        "port {}: not killed: its owner could not be inspected",
                        label(*port)
                    );
                    code = 3;
                }
                continue;
            };
            println!(
                "port    {}\npid     {}\nparent  {}\nuser    {}\ncmd     {}\n{docker_line}cwd     {}\nuptime  {}\n",
                label(*port),
                i.pid,
                i.ppid,
                shown(&i.user),
                shown(&i.cmd),
                shown(&i.cwd),
                fmt_duration(i.uptime_secs)
            );
            if !args.kill {
                continue;
            }
            if let Some(reason) = skip_reason(i, &holders) {
                eprintln!("pid {}: not killed: {reason}", i.pid);
                skipped.push(i.pid);
                continue;
            }
            if !killed.contains(&i.pid) {
                killed.push(i.pid); // one process can hold several requested ports
                if !kill(i.pid, i.started, args.force) {
                    code = 3;
                }
            }
        }
        // A skipped holder (a supervisor, say) may respawn what was killed or keep the
        // socket open itself, so the port is not known to be free.
        if args.kill && !skipped.is_empty() {
            let pids: Vec<String> = skipped.iter().map(u32::to_string).collect();
            eprintln!(
                "port {}: still held by pid {}, which was not killed",
                label(*port),
                pids.join(", ")
            );
            code = 3;
        }
    }
    ExitCode::from(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_managers_are_recognised() {
        assert!(is_service_manager(1, "/sbin/init"));
        assert!(is_service_manager(812, "/usr/lib/systemd/systemd --user"));
        assert!(is_service_manager(90, "/sbin/launchd"));
        assert!(is_service_manager(
            1652,
            r"C:\Windows\System32\svchost.exe -k RPCSS"
        ));
        assert!(!is_service_manager(4242, "python3 -m http.server 3000"));
        assert!(!is_service_manager(
            4243,
            "/usr/lib/systemd/systemd-resolved"
        ));
    }

    #[test]
    fn control_characters_are_escaped() {
        assert_eq!(
            shown("ok \u{1b}]52;c;aGk=\u{7}x"),
            "ok \\u{1b}]52;c;aGk=\\u{7}x"
        );
        assert_eq!(shown("héllo wörld"), "héllo wörld");
    }

    #[test]
    fn same_process_requires_matching_start_time() {
        let me = std::process::id();
        let started = proc::info(me).unwrap().started;
        assert!(same_process(me, started));
        assert!(!same_process(me, started + 1)); // same pid, different process
    }
}
