mod proc;

use clap::Parser;
use std::process::ExitCode;
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(about = "Show what is listening on a TCP port")]
struct Args {
    port: u16,
    /// Send SIGTERM to the listener(s) and wait up to 3s for exit
    #[arg(long)]
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
            eprintln!("pid {pid}: {name} failed: {}", std::io::Error::last_os_error());
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

fn main() -> ExitCode {
    let args = Args::parse();
    let (pids, hidden) = proc::find_listeners(args.port);
    if pids.is_empty() {
        println!("nothing listening on :{}", args.port);
        if hidden {
            eprintln!("some processes are unreadable; run with sudo to see all");
        }
        return ExitCode::from(1);
    }
    let mut ok = true;
    for pid in pids {
        let Some(i) = proc::info(pid) else { continue };
        println!("pid     {}\nparent  {}\nuser    {}\ncmd     {}\ncwd     {}\nuptime  {}\n",
            i.pid, i.ppid, i.user, i.cmd, i.cwd, fmt_duration(i.uptime_secs));
        if args.kill {
            ok &= kill(pid, args.force);
        }
    }
    if ok { ExitCode::SUCCESS } else { ExitCode::from(2) }
}
