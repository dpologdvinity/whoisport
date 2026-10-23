#[cfg(target_os = "linux")]
mod proc;
#[cfg(windows)]
#[path = "proc_windows.rs"]
mod proc;

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
}

#[derive(Parser)]
#[command(version, about = "Show what is listening on a TCP port")]
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
}

const HIDDEN: &str = "owned by another user; run with sudo to see it";

fn fmt_duration(s: u64) -> String {
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m {}s", s / 60, s % 60),
        3600..=86399 => format!("{}h {}m", s / 3600, s % 3600 / 60),
        _ => format!("{}d {}h", s / 86400, s % 86400 / 3600),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max - 1).chain(['…']).collect()
}

fn wait_exit(pid: u32) -> bool {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if !proc::alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    !proc::alive(pid)
}

type KillStep = fn(u32) -> std::io::Result<()>;

fn kill(pid: u32, force: bool) -> bool {
    let steps: [(KillStep, &str); 2] = [
        (proc::ask_exit, proc::GRACEFUL),
        (proc::terminate, proc::FORCEFUL),
    ];
    for (step, name) in steps.into_iter().take(if force { 2 } else { 1 }) {
        if let Err(e) = step(pid) {
            eprintln!("pid {pid}: {name} failed: {e}");
            return false;
        }
        if wait_exit(pid) {
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

fn to_json(port: u16, pid: Option<u32>, info: &Option<Info>) -> serde_json::Value {
    match info {
        Some(i) => json!({
            "port": port, "pid": i.pid, "ppid": i.ppid, "user": i.user,
            "cmd": i.cmd, "cwd": i.cwd, "uptime_secs": i.uptime_secs,
        }),
        None => json!({
            "port": port, "pid": pid, "ppid": null, "user": null,
            "cmd": null, "cwd": null, "uptime_secs": null,
        }),
    }
}

fn main() -> ExitCode {
    #[cfg(windows)]
    proc::run_helper_if_requested();

    let args = Args::parse();
    let all = proc::listeners();
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
    let mut code = if rows.is_empty() || !missing.is_empty() {
        1
    } else {
        0
    };

    if args.json {
        let out: Vec<_> = rows
            .iter()
            .map(|(port, pid, i)| to_json(*port, *pid, i))
            .collect();
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return ExitCode::from(code);
    }

    if args.ports.is_empty() {
        if rows.is_empty() {
            println!("nothing listening");
            return ExitCode::from(1);
        }
        println!("{:<6} {:<8} {:<10} CMD", "PORT", "PID", "USER");
        for (port, _, info) in &rows {
            match info {
                Some(i) => println!(
                    "{port:<6} {:<8} {:<10} {}",
                    i.pid,
                    truncate(&i.user, 10),
                    truncate(&i.cmd, 80)
                ),
                None => println!("{port:<6} {:<8} {:<10} ?", "?", "?"),
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
            println!("nothing listening on :{port}\n");
        }
        for (_, _, info) in rows.iter().filter(|r| r.0 == *port) {
            let Some(i) = info else {
                println!("port    {port}\npid     ? ({HIDDEN})\n");
                if args.kill {
                    code = 3;
                }
                continue;
            };
            println!(
                "port    {port}\npid     {}\nparent  {}\nuser    {}\ncmd     {}\ncwd     {}\nuptime  {}\n",
                i.pid,
                i.ppid,
                i.user,
                i.cmd,
                i.cwd,
                fmt_duration(i.uptime_secs)
            );
            if args.kill && !killed.contains(&i.pid) {
                killed.push(i.pid); // one process can hold several requested ports
                if !kill(i.pid, args.force) {
                    code = 3;
                }
            }
        }
    }
    ExitCode::from(code)
}
