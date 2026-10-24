//! End-to-end tests: run the real `whoisport` binary against real listening sockets.
//!
//! Each test opens its own listener (a Rust `TcpListener`/`UdpSocket`, or a small Python
//! child process when the listener must be killable) on an ephemeral port, runs the binary
//! the way a user would, and checks the output and exit code. Tests that kill a process
//! only ever kill children they started themselves.
//!
//! Platform differences are handled with `cfg`: the polite/forced kill steps have
//! different names on Windows, and a few tests only make sense on one platform.

use std::net::TcpListener;
use std::process::Command;

/// Run the built `whoisport` binary with `args`.
///
/// Returns stdout and stderr joined together, and the exit code (`None` if the process was
/// ended by a signal).
fn run(args: &[&str]) -> (String, Option<i32>) {
    let out = Command::new(env!("CARGO_BIN_EXE_whoisport"))
        .args(args)
        .output()
        .unwrap();
    (
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr),
        out.status.code(),
    )
}

/// A port with a listener is reported with its owner's pid, and the exit code is 0.
#[test]
fn reports_listener_and_exits_zero() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port().to_string();
    let (stdout, code) = run(&[&port]);
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains(&format!("pid     {}", std::process::id())),
        "{stdout}"
    );
}

/// Asking about a port nobody listens on says so and exits 1.
#[test]
fn exits_one_when_nothing_listening() {
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port().to_string()
    };
    let (stdout, code) = run(&[&port]);
    assert_eq!(code, Some(1));
    assert!(stdout.contains("nothing listening"), "{stdout}");
}

/// With no arguments the table lists every listener, including this test's own.
#[test]
fn no_args_lists_all_listeners() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let (stdout, code) = run(&[]);
    assert_eq!(code, Some(0));
    let row = format!("{port:<6} {:<8}", std::process::id());
    assert!(stdout.lines().any(|l| l.starts_with(&row)), "{stdout}");
}

/// Several ports at once: found ports are reported, a missing one is named, and the exit
/// code is 1 because something asked for was not found.
#[test]
fn multiple_ports_exit_one_if_any_missing() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let live = l.local_addr().unwrap().port().to_string();
    let dead = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port().to_string()
    };
    let (stdout, code) = run(&[&live, &dead]);
    assert_eq!(code, Some(1));
    assert!(stdout.contains(&format!("port    {live}")), "{stdout}");
    assert!(
        stdout.contains(&format!("nothing listening on :{dead}")),
        "{stdout}"
    );
}

/// `--kill` with no port is a usage error (exit 2), so it can never mean "kill everything".
#[test]
fn kill_requires_a_port() {
    let (_, code) = run(&["--kill"]);
    assert_eq!(code, Some(2)); // clap usage error; must never mean "kill everything"
}

/// Name of the polite kill step in output (`exited after <ASK>`), per platform.
#[cfg(windows)]
const ASK: &str = "Ctrl+C/close";
/// Name of the forced kill step in output (`exited after <FORCE>`), per platform.
#[cfg(windows)]
const FORCE: &str = "TerminateProcess";
/// Name of the polite kill step in output (`exited after <ASK>`), per platform.
#[cfg(not(windows))]
const ASK: &str = "SIGTERM";
/// Name of the forced kill step in output (`exited after <FORCE>`), per platform.
#[cfg(not(windows))]
const FORCE: &str = "SIGKILL";

/// Spawn a python listener (optionally ignoring polite exit requests); returns it and its port.
fn spawn_listener(stubborn: bool) -> (std::process::Child, String) {
    let ign = if stubborn {
        "import signal;signal.signal(signal.SIGTERM,signal.SIG_IGN);signal.signal(signal.SIGINT,signal.SIG_IGN);"
    } else {
        ""
    };
    let script = format!(
        "{ign}import socket,time;s=socket.socket();s.bind(('127.0.0.1',0));s.listen();print(s.getsockname()[1],flush=True);time.sleep(60)"
    );
    let mut cmd = Command::new(if cfg!(windows) { "python" } else { "python3" });
    cmd.args(["-c", &script])
        .stdout(std::process::Stdio::piped());
    // On Windows the Ctrl+C goes to the target's whole console; give it its own so the
    // test runner (and cargo) never receive it.
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x10); // CREATE_NEW_CONSOLE
    let mut child = cmd.spawn().unwrap();
    let mut line = String::new();
    std::io::BufRead::read_line(
        &mut std::io::BufReader::new(child.stdout.take().unwrap()),
        &mut line,
    )
    .unwrap();
    (child, line.trim().to_string())
}

/// `--kill` asks the listener to exit and reports success (exit 0).
#[test]
fn kill_terminates_listener() {
    // On Linux the child stays a zombie until wait(), so this also covers zombie-as-exited.
    let (mut child, port) = spawn_listener(false);
    let (stdout, code) = run(&[&port, "--kill"]);
    let status = child.wait().unwrap();
    assert_eq!(code, Some(0), "{stdout}");
    assert!(stdout.contains(&format!("exited after {ASK}")), "{stdout}");
    assert!(!status.success());
}

/// `--kill --force` forces the exit when the process ignores the polite request.
#[test]
fn force_escalates_when_polite_request_ignored() {
    let (mut child, port) = spawn_listener(true);
    let (stdout, code) = run(&[&port, "--kill", "--force"]);
    child.wait().unwrap();
    assert_eq!(code, Some(0), "{stdout}");
    assert!(
        stdout.contains(&format!("exited after {FORCE}")),
        "{stdout}"
    );
}

/// Without `--force`, a process that ignores the polite request is still running: exit 3.
#[test]
fn failed_kill_exits_three() {
    let (mut child, port) = spawn_listener(true);
    let (stdout, code) = run(&[&port, "--kill"]);
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(code, Some(3), "{stdout}");
    assert!(stdout.contains("still running"), "{stdout}");
}

/// `--json` prints a JSON array with the listener's port and pid.
#[test]
fn json_output_lists_listener() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let (stdout, code) = run(&[&port.to_string(), "--json"]);
    assert_eq!(code, Some(0));
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v[0]["port"], port);
    assert_eq!(v[0]["pid"], std::process::id());
}

/// `--json` together with `--kill` is rejected as a usage error (exit 2).
#[test]
fn json_conflicts_with_kill() {
    let (_, code) = run(&["1", "--json", "--kill"]);
    assert_eq!(code, Some(2));
}

/// `--udp` finds a bound UDP socket, and the same port is not a TCP listener.
#[test]
fn udp_flag_finds_bound_socket() {
    let s = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = s.local_addr().unwrap().port().to_string();
    let (stdout, code) = run(&[&port, "--udp"]);
    assert_eq!(code, Some(0), "{stdout}");
    assert!(stdout.contains(&format!("port    {port}/udp")), "{stdout}");
    assert!(
        stdout.contains(&format!("pid     {}", std::process::id())),
        "{stdout}"
    );
    // ...and it is not a TCP listener
    assert_eq!(run(&[&port]).1, Some(1));
}

/// A parent that shares the socket with its child is never signalled; the child is
/// stopped, and because the parent still holds the port the run exits 3.
#[cfg(unix)]
#[test]
fn kill_spares_parent_that_shares_the_socket() {
    // The parent listens, then forks; the child inherits the socket, as with a supervisor.
    let script = "import os,socket,time;s=socket.socket();s.bind(('127.0.0.1',0));s.listen();\
                  pid=os.fork();\nif pid==0:\n time.sleep(60)\nelse:\n print(s.getsockname()[1],pid,flush=True);time.sleep(60)";
    let mut parent = Command::new("python3")
        .args(["-c", script])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    std::io::BufRead::read_line(
        &mut std::io::BufReader::new(parent.stdout.take().unwrap()),
        &mut line,
    )
    .unwrap();
    let mut f = line.split_whitespace();
    let (port, child) = (f.next().unwrap().to_string(), f.next().unwrap().to_string());
    let (out, code) = run(&[&port, "--kill"]);
    let parent_alive = parent.try_wait().unwrap().is_none();
    parent.kill().unwrap();
    parent.wait().unwrap();
    // The child is stopped, but the skipped parent still holds the port, so this is a failure.
    assert_eq!(code, Some(3), "{out}");
    assert!(out.contains(&format!("pid {child}: exited after")), "{out}");
    assert!(out.contains("not killed: it is a parent of pid"), "{out}");
    assert!(out.contains("still held by pid"), "{out}");
    assert!(parent_alive, "{out}");
}

/// Windows: Ctrl+C reaches every program on a console, so with an unrelated program on the
/// same console `--kill` refuses (exit 3) and leaves it alone; `--kill --force` then works.
#[cfg(windows)]
#[test]
fn ctrl_c_is_not_sent_to_a_shared_console() {
    use std::os::windows::process::CommandExt;
    // cmd runs the listener and an unrelated `ping` on one new console; Ctrl+C there
    // would also stop the ping, so whoisport must refuse and say so.
    let dir = std::env::temp_dir().join(format!("whoisport-shared-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let port_file = dir.join("port");
    let script = format!(
        "import socket,time;s=socket.socket();s.bind(('127.0.0.1',0));s.listen();\
         open(r'{}','w').write(str(s.getsockname()[1]));time.sleep(60)",
        port_file.display()
    );
    let mut cmd = Command::new("cmd.exe");
    cmd.raw_arg(format!(
        "/c start /b python -c \"{script}\" & ping -n 60 127.0.0.1 >nul"
    ))
    .creation_flags(0x10); // CREATE_NEW_CONSOLE
    let mut shell = cmd.spawn().unwrap();
    let port = (0..100)
        .find_map(|_| {
            std::thread::sleep(std::time::Duration::from_millis(100));
            std::fs::read_to_string(&port_file)
                .ok()
                .filter(|p| !p.is_empty())
        })
        .expect("listener did not start");
    let (out, code) = run(&[&port, "--kill"]);
    let shell_alive = shell.try_wait().unwrap().is_none();
    // The refusal says "use --force"; doing exactly that must work.
    let (forced_out, forced_code) = run(&[&port, "--kill", "--force"]);
    let _ = Command::new("taskkill")
        .args(["/F", "/T", "/PID", &shell.id().to_string()])
        .output();
    let _ = shell.wait();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(code, Some(3), "{out}");
    assert!(
        out.contains("console is shared with other programs"),
        "{out}"
    );
    assert!(
        shell_alive,
        "the shell's ping must not have been interrupted: {out}"
    );
    assert_eq!(forced_code, Some(0), "{forced_out}");
    assert!(
        forced_out.contains("exited after TerminateProcess"),
        "{forced_out}"
    );
}

/// A command line containing an escape sequence and invalid UTF-8 is shown with the
/// control characters escaped, and the process is still listed with its real pid.
#[cfg(unix)]
#[test]
fn hostile_argv_is_shown_escaped_not_hidden() {
    use std::os::unix::ffi::OsStrExt;
    let script = "import socket,time;s=socket.socket();s.bind(('127.0.0.1',0));s.listen();\
                  print(s.getsockname()[1],flush=True);time.sleep(60)";
    // An escape sequence plus a byte that is not valid UTF-8.
    let arg = std::ffi::OsStr::from_bytes(b"\x1b[2Jfake\xff");
    let mut child = Command::new("python3")
        .args(["-c", script])
        .arg(arg)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut port = String::new();
    std::io::BufRead::read_line(
        &mut std::io::BufReader::new(child.stdout.take().unwrap()),
        &mut port,
    )
    .unwrap();
    let (out, code) = run(&[port.trim()]);
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(code, Some(0), "{out}");
    assert!(out.contains(&format!("pid     {}", child.id())), "{out}");
    assert!(out.contains("\\u{1b}[2Jfake"), "{out}");
    assert!(
        !out.contains('\x1b'),
        "raw escape reached the terminal: {out}"
    );
}

/// One process listening on two requested ports is signalled exactly once.
#[test]
fn process_on_two_requested_ports_is_killed_once() {
    let script = "import socket,time\nss=[socket.socket() for _ in range(2)]\n\
                  [s.bind(('127.0.0.1',0)) or s.listen() for s in ss]\n\
                  print(*[s.getsockname()[1] for s in ss],flush=True);time.sleep(60)";
    let mut cmd = Command::new(if cfg!(windows) { "python" } else { "python3" });
    cmd.args(["-c", script])
        .stdout(std::process::Stdio::piped());
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x10); // own console
    let mut child = cmd.spawn().unwrap();
    let mut line = String::new();
    std::io::BufRead::read_line(
        &mut std::io::BufReader::new(child.stdout.take().unwrap()),
        &mut line,
    )
    .unwrap();
    let ports: Vec<&str> = line.split_whitespace().collect();
    let (out, code) = run(&[ports[0], ports[1], "--kill"]);
    child.wait().unwrap();
    assert_eq!(code, Some(0), "{out}");
    assert_eq!(out.matches("exited after").count(), 1, "{out}");
    assert!(!out.contains("failed"), "{out}");
}
