//! `whoisport`: show what is listening on a TCP or UDP port, and optionally stop it.
//!
//! Run with no arguments to list every listener, with ports to see details of the
//! processes behind them, and with `--kill` to ask those processes to exit.
//!
//! # Layout
//!
//! | Module | Responsibility |
//! |--------|----------------|
//! | [`cli`] | The command-line arguments |
//! | [`scan`] | Find listeners and gather what is known about each |
//! | `proc` | The operating-system backend (one of three files, chosen at compile time) |
//! | [`docker`] | Name the Docker container behind a published port |
//! | [`output`] | Format results as a table, detail blocks, or JSON |
//! | [`kill`] | Stop the processes safely |
//!
//! See `ARCHITECTURE.md` for how data flows between them.
//!
//! # Exit codes
//!
//! `0` success; `1` nothing is listening on a requested port (or on any port); `2` invalid
//! arguments (clap); `3` a kill failed or a holder was deliberately not killed.

// Every item, including private ones, must be documented (enforced in CI via clippy).
#![warn(clippy::missing_docs_in_private_items)]

mod cli;
mod docker;
mod kill;
mod output;
mod scan;

// Exactly one `proc` backend is compiled in, each exposing the same functions:
// `listeners`, `info`, `alive`, `ask_exit`, `terminate` and the `GRACEFUL`/`FORCEFUL` names.
#[cfg(target_os = "linux")]
mod proc;
#[cfg(target_os = "macos")]
#[path = "proc_macos.rs"]
mod proc;
#[cfg(windows)]
#[path = "proc_windows.rs"]
mod proc;

// Shared helpers: process details via `sysinfo` (Windows, macOS) and signals (Linux, macOS).
#[cfg(any(windows, target_os = "macos"))]
mod sysinfo_info;
#[cfg(unix)]
mod unix_kill;

use clap::Parser;
use cli::Args;
use scan::Scan;
use std::process::ExitCode;

/// What is known about one running process. Filled in by the `proc` backend.
pub struct Info {
    /// Process id.
    pub pid: u32,
    /// Parent process id (0 when unknown).
    pub ppid: u32,
    /// Name of the user running the process (or a numeric id if the name is unknown).
    pub user: String,
    /// Full command line, arguments joined by spaces.
    pub cmd: String,
    /// Working directory, or `?` when it cannot be read.
    pub cwd: String,
    /// Seconds since the process started.
    pub uptime_secs: u64,
    /// Process start time, in platform units (Linux: clock ticks since boot; elsewhere:
    /// seconds since the epoch). Together with the pid it identifies one process, so a pid
    /// that is reused after the target exits is never mistaken for it.
    pub started: u64,
}

/// Program entry point: parse the arguments, run, and turn the result into an exit code.
fn main() -> ExitCode {
    // On Windows the program sometimes re-runs itself as a tiny helper (see
    // `proc_windows::ask_exit`); if so, do that job and exit instead of the normal flow.
    #[cfg(windows)]
    proc::run_helper_if_requested();

    ExitCode::from(run(&Args::parse()))
}

/// Scan for listeners and report them (or kill them). Returns the process exit code.
fn run(args: &Args) -> u8 {
    let mut seen = std::collections::HashSet::new();
    let unique_ports: Vec<u16> = args
        .ports
        .iter()
        .copied()
        .filter(|p| seen.insert(*p))
        .collect();
    let scan = Scan::new(&unique_ports, args.udp);
    if args.json {
        output::print_json(&scan.rows, args.proto());
        return scan.exit_code();
    }
    if unique_ports.is_empty() {
        if scan.rows.is_empty() {
            println!("nothing listening");
            return 1;
        }
        output::print_table(&scan.rows, args.udp);
        return 0;
    }
    report_ports(&unique_ports, args, &scan)
}

/// Detail mode: print every requested port, and with `--kill` stop what holds it.
///
/// Returns the exit code: [`Scan::exit_code`], raised to `3` if any kill failed.
fn report_ports(ports: &[u16], args: &Args, scan: &Scan) -> u8 {
    let mut code = scan.exit_code();
    // Pids already handled, so a process holding several requested ports is stopped once.
    let mut killed = Vec::new();
    for &port in ports {
        let label = output::port_label(port, args.udp);
        if scan.missing.contains(&port) {
            println!("nothing listening on :{label}\n");
        }
        let rows = scan.rows_for(port);
        for row in &rows {
            output::print_details(row, args.udp);
        }
        if args.kill && !kill::kill_port(&rows, &label, args.force, &mut killed) {
            code = 3;
        }
    }
    code
}
