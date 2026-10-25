//! Stopping the processes that hold a port, as safely as possible.
//!
//! Safety rules, in the order they apply:
//!
//! 1. **Only the process that was shown is signalled.** Each target is identified by its
//!    pid *and* start time ([`same_process`]); if the process exited and the operating
//!    system gave its pid to something else, that something else is left alone.
//! 2. **Service managers and parents are never signalled.** When several processes share a
//!    socket, stopping a supervisor (or `systemd`) takes down far more than one port
//!    ([`skip_reason`]).
//! 3. **Ask first.** Each target is asked to exit; it is forced only with `--force`
//!    ([`kill_with`]).

use crate::scan::Row;
use crate::{Info, output, proc};
use std::time::{Duration, Instant};

/// How long to wait for a process to exit after each signal.
const WAIT: Duration = Duration::from_secs(3);

/// A way of stopping a process: signal `pid` and report whether the request was accepted.
type KillStep = fn(u32) -> std::io::Result<()>;

/// Is `pid` still the process that started at `started`?
///
/// A pid alone is not an identity: after a process exits, its pid can be handed to an
/// unrelated one. The start time tells them apart.
pub fn same_process(pid: u32, started: u64) -> bool {
    proc::alive(pid) && proc::info(pid).is_some_and(|i| i.started == started)
}

/// Wait up to [`WAIT`] for the process to exit; true if it did.
fn wait_exit(pid: u32, started: u64) -> bool {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if !same_process(pid, started) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    !same_process(pid, started)
}

/// Ask the process to exit; with `force`, escalate if that fails or is ignored.
///
/// Returns true when the process is gone. Progress is reported on stdout/stderr.
fn kill(pid: u32, started: u64, force: bool) -> bool {
    kill_with(
        &[
            (proc::ask_exit, proc::GRACEFUL),
            (proc::terminate, proc::FORCEFUL),
        ],
        pid,
        started,
        force,
    )
}

/// The engine behind [`kill`], with the steps passed in so it can be tested.
///
/// `steps` run in order: the first asks politely, the second forces (and is skipped
/// unless `force`). A step that errors (Windows refuses Ctrl+C on a shared console, a
/// signal is denied) counts as not having worked, so `--force` still moves on to the
/// next step. Before every step the target is re-identified with [`same_process`].
fn kill_with(steps: &[(KillStep, &str)], pid: u32, started: u64, force: bool) -> bool {
    // Did any step run successfully? (If every step errored, it already explained itself.)
    let mut asked = false;
    for (step, name) in steps.iter().take(if force { steps.len() } else { 1 }) {
        // Re-check right before each signal: the pid may have been reused meanwhile.
        if !same_process(pid, started) {
            println!("pid {pid}: already exited");
            return true;
        }
        match step(pid) {
            Err(e) => eprintln!("pid {pid}: {name} failed: {e}"),
            Ok(()) => {
                asked = true;
                if wait_exit(pid, started) {
                    println!("pid {pid}: exited after {name}");
                    return true;
                }
            }
        }
    }
    // Only a step that ran and was ignored leaves the process "still running".
    if asked && force {
        eprintln!("pid {pid}: still running");
    } else if asked {
        eprintln!(
            "pid {pid}: still running (use --force for {})",
            proc::FORCEFUL
        );
    }
    false
}

/// Is this the system or user service manager? It can hold a socket it activates.
///
/// `svchost.exe` and `services.exe` count too: one `svchost.exe` hosts many Windows
/// services (RPC on port 135, DNS, ...), so stopping it stops all of them.
fn is_service_manager(pid: u32, cmd: &str) -> bool {
    if pid == 1 {
        return true;
    }
    let trimmed = cmd.trim();
    let exe = if let Some(inside) = trimmed.strip_prefix('"') {
        inside.split('"').next().unwrap_or("")
    } else {
        trimmed.split_whitespace().next().unwrap_or("")
    };
    let name = exe.rsplit(['/', '\\']).next().unwrap_or(exe);
    let name = name.trim_matches('"').to_ascii_lowercase();
    matches!(
        name.as_str(),
        "systemd" | "launchd" | "svchost.exe" | "services.exe"
    )
}

/// Is `ancestor` somewhere up `pid`'s chain of parents?
///
/// The walk is bounded (64 steps) and stops at pid 1 or a process that is its own parent,
/// so a corrupted or cyclic parent chain cannot loop forever.
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

/// Why this holder of the socket must not be signalled, or `None` if it may be.
///
/// Several processes can hold one listening socket when a supervisor or
/// socket-activating service manager hands it to the real server. Signalling the
/// supervisor can take down far more than the server, so service managers and any holder
/// that is a parent of another holder (`holders` lists every holder's pid) are skipped.
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

/// Stop every process holding one port. Returns true only if the port is now free of
/// holders we were responsible for.
///
/// `killed` records pids already handled, so a process holding several requested ports is
/// signalled once. Holders that are skipped (supervisors, parents) or cannot be inspected
/// make the result false: they may respawn what was killed, so the port is not known to
/// be free.
pub fn kill_port(rows: &[&Row], label: &str, force: bool, killed: &mut Vec<u32>) -> bool {
    let holders: Vec<u32> = rows
        .iter()
        .filter_map(|r| r.info.as_ref().map(|i| i.pid))
        .collect();
    let mut ok = true;
    let mut skipped: Vec<u32> = Vec::new();
    for row in rows {
        // Killing docker-proxy only breaks the port mapping; the container keeps running.
        if let Some(container) = &row.container {
            let name = output::shown(&container.name);
            eprintln!(
                "port {label}: published by container {name}; to stop it, run: docker stop {name}"
            );
            ok = false;
            continue;
        }
        let Some(info) = &row.info else {
            eprintln!("port {label}: not killed: its owner could not be inspected");
            ok = false;
            continue;
        };
        if let Some(reason) = skip_reason(info, &holders) {
            eprintln!("pid {}: not killed: {reason}", info.pid);
            skipped.push(info.pid);
            continue;
        }
        if !killed.contains(&info.pid) {
            killed.push(info.pid);
            if !kill(info.pid, info.started, force) {
                ok = false;
            }
        }
    }
    if !skipped.is_empty() {
        let pids: Vec<String> = skipped.iter().map(u32::to_string).collect();
        eprintln!(
            "port {label}: still held by pid {}, which was not killed",
            pids.join(", ")
        );
        ok = false;
    }
    ok
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
        assert!(is_service_manager(
            1653,
            r#""C:\Program Files\Services\svchost.exe" -k RPCSS"#
        ));
        assert!(!is_service_manager(4242, "python3 -m http.server 3000"));
        assert!(!is_service_manager(
            4243,
            "/usr/lib/systemd/systemd-resolved"
        ));
    }

    #[test]
    fn same_process_requires_matching_start_time() {
        let me = std::process::id();
        let started = proc::info(me).unwrap().started;
        assert!(same_process(me, started));
        assert!(!same_process(me, started + 1)); // same pid, different process
    }

    #[test]
    fn a_process_is_not_its_own_ancestor() {
        let me = std::process::id();
        assert!(!is_ancestor(me, me));
        // pid 0 / u32::MAX do not exist: the walk ends instead of looping.
        assert!(!is_ancestor(me, u32::MAX));
    }

    #[test]
    fn skip_reason_spares_a_service_manager() {
        let manager = Info {
            pid: 1,
            ppid: 0,
            user: "root".into(),
            cmd: "/sbin/init".into(),
            cwd: "/".into(),
            uptime_secs: 0,
            started: 0,
        };
        assert_eq!(
            skip_reason(&manager, &[1]).as_deref(),
            Some("it is a service manager")
        );
    }

    #[test]
    fn container_port_is_spared_and_marked_not_ok() {
        use crate::docker::Container;
        let row = Row {
            port: 8080,
            pid: Some(1234),
            info: Some(Info {
                pid: 1234,
                ppid: 1,
                user: "root".into(),
                cmd: "docker-proxy".into(),
                cwd: "/".into(),
                uptime_secs: 10,
                started: 100,
            }),
            container: Some(Container {
                name: "web".into(),
                image: "nginx".into(),
            }),
        };
        let mut killed = Vec::new();
        let rows = [&row];
        assert!(!kill_port(&rows, "8080", false, &mut killed));
        assert!(killed.is_empty());
    }

    #[cfg(unix)]
    mod kill_steps {
        use super::*;
        use std::process::{Child, Command, Stdio};

        /// A polite step that is refused outright.
        fn failing_ask(_: u32) -> std::io::Result<()> {
            Err(std::io::Error::other("refused"))
        }

        /// A polite step that is accepted but ignored: the process keeps running.
        fn ignored_ask(_: u32) -> std::io::Result<()> {
            Ok(())
        }

        /// Start a long-running child and return it with its start time.
        fn sleeper() -> (Child, u64) {
            let child = Command::new("sleep")
                .arg("60")
                .stdout(Stdio::null())
                .spawn()
                .unwrap();
            let started = proc::info(child.id()).unwrap().started;
            (child, started)
        }

        /// Stop and reap a child the test is done with.
        fn reap(mut c: Child) {
            let _ = c.kill();
            let _ = c.wait();
        }

        #[test]
        fn force_runs_the_forced_step_after_the_polite_one_errors() {
            let (mut child, started) = sleeper();
            let steps: [(KillStep, &str); 2] = [(failing_ask, "ASK"), (proc::terminate, "FORCE")];
            assert!(kill_with(&steps, child.id(), started, true));
            assert!(child.wait().is_ok_and(|s| !s.success())); // it was killed
        }

        #[test]
        fn without_force_an_erroring_polite_step_is_the_end() {
            let (child, started) = sleeper();
            let steps: [(KillStep, &str); 2] = [(failing_ask, "ASK"), (proc::terminate, "FORCE")];
            assert!(!kill_with(&steps, child.id(), started, false));
            assert!(same_process(child.id(), started)); // still running: nothing forced it
            reap(child);
        }

        #[test]
        fn force_escalates_when_the_polite_step_is_ignored() {
            let (mut child, started) = sleeper();
            let steps: [(KillStep, &str); 2] = [(ignored_ask, "ASK"), (proc::terminate, "FORCE")];
            assert!(kill_with(&steps, child.id(), started, true));
            child.wait().unwrap();
        }

        #[test]
        fn a_reused_pid_is_never_signalled() {
            let (child, started) = sleeper();
            let steps: [(KillStep, &str); 2] = [(ignored_ask, "ASK"), (proc::terminate, "FORCE")];
            // Pretend the process we meant to kill started at a different time: the pid now
            // belongs to "someone else", so nothing may be sent and it reports as gone.
            assert!(kill_with(&steps, child.id(), started + 1, true));
            assert!(same_process(child.id(), started)); // untouched
            reap(child);
        }
    }
}
