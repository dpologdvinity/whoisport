mod proc;

use clap::Parser;
use std::process::ExitCode;
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(version, about = "Show what is listening on a TCP port")]
struct Args {
    /// Port(s) to inspect; omit to list every listening port
    ports: Vec<u16>,
    /// Send SIGTERM to the listener(s) on the given port(s) and wait up to 3s for exit
    #[arg(long, requires = "ports")]
    kill: bool,
    /// With --kill: escalate to SIGKILL if SIGTERM did not work
    #[arg(long, requires = "kill")]
    force: bool,
}

fn fmt_duration(s: u64) -> String {
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m {}s", s / 60, s % 60),
        3600..=86399 => format!("{}h {}m", s / 3600, s % 3600 / 60),
        _ => format!("{}d {}h", s / 86400, s % 86400 / 3600),
    }
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

fn kill(pid: u32, force: bool) -> bool {
    for (sig, name) in [(libc::SIGTERM, "SIGTERM"), (libc::SIGKILL, "SIGKILL")] {
        if sig == libc::SIGKILL && !force {
            break;
        }
        if unsafe { libc::kill(pid as i32, sig) } != 0 {
            eprintln!(
                "pid {pid}: {name} failed: {}",
                std::io::Error::last_os_error()
            );
            return false;
        }
        if wait_exit(pid) {
            println!("pid {pid}: exited after {name}");
            return true;
        }
    }
    eprintln!("pid {pid}: still running (use --force for SIGKILL)");
    false
}

const HIDDEN: &str = "owned by another user; run with sudo to see it";

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max - 1).chain(['…']).collect()
}

fn list(all: &[(u16, Option<u32>)]) -> u8 {
    if all.is_empty() {
        println!("nothing listening");
        return 1;
    }
    println!("{:<6} {:<8} {:<10} CMD", "PORT", "PID", "USER");
    for (port, pid) in all {
        match pid.and_then(proc::info) {
            Some(i) => println!(
                "{port:<6} {:<8} {:<10} {}",
                i.pid,
                i.user,
                truncate(&i.cmd, 80)
            ),
            None => println!("{port:<6} {:<8} {:<10} ?", "?", "?"),
        }
    }
    if all.iter().any(|(_, pid)| pid.is_none()) {
        eprintln!("ports marked ? are {HIDDEN}");
    }
    0
}

fn main() -> ExitCode {
    let args = Args::parse();
    let all = proc::listeners();
    if args.ports.is_empty() {
        return ExitCode::from(list(&all));
    }
    let mut code = 0; // 2 if a kill failed, else 1 if some port had no listener
    for &port in &args.ports {
        let here: Vec<Option<u32>> = all
            .iter()
            .filter(|(p, _)| *p == port)
            .map(|(_, pid)| *pid)
            .collect();
        if here.is_empty() {
            println!("nothing listening on :{port}\n");
            code = code.max(1);
        }
        for pid in here {
            let Some(i) = pid.and_then(proc::info) else {
                println!("port    {port}\npid     ? ({HIDDEN})\n");
                if args.kill {
                    code = 2;
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
            if args.kill && !kill(i.pid, args.force) {
                code = 2;
            }
        }
    }
    ExitCode::from(code)
}
