use std::net::TcpListener;
use std::process::Command;

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

#[test]
fn no_args_lists_all_listeners() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let (stdout, code) = run(&[]);
    assert_eq!(code, Some(0));
    let row = format!("{port:<6} {:<8}", std::process::id());
    assert!(stdout.lines().any(|l| l.starts_with(&row)), "{stdout}");
}

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

#[test]
fn kill_requires_a_port() {
    let (_, code) = run(&["--kill"]);
    assert_eq!(code, Some(2)); // clap usage error; must never mean "kill everything"
}

#[cfg(windows)]
const ASK: &str = "Ctrl+C/close";
#[cfg(windows)]
const FORCE: &str = "TerminateProcess";
#[cfg(not(windows))]
const ASK: &str = "SIGTERM";
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

#[test]
fn failed_kill_exits_three() {
    let (mut child, port) = spawn_listener(true);
    let (stdout, code) = run(&[&port, "--kill"]);
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(code, Some(3), "{stdout}");
    assert!(stdout.contains("still running"), "{stdout}");
}

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

#[test]
fn json_conflicts_with_kill() {
    let (_, code) = run(&["1", "--json", "--kill"]);
    assert_eq!(code, Some(2));
}

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
