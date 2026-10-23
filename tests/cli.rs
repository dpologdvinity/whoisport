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

/// Spawn a python listener (optionally ignoring SIGTERM); returns the child and its port.
fn spawn_listener(ignore_term: bool) -> (std::process::Child, String) {
    let ign = if ignore_term {
        "import signal;signal.signal(signal.SIGTERM,signal.SIG_IGN);"
    } else {
        ""
    };
    let script = format!(
        "{ign}import socket,time;s=socket.socket();s.bind(('127.0.0.1',0));s.listen();print(s.getsockname()[1],flush=True);time.sleep(60)"
    );
    let mut child = Command::new("python3")
        .args(["-c", &script])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
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
    // The child stays a zombie until wait(), so this also covers zombie-as-exited.
    let (mut child, port) = spawn_listener(false);
    let (stdout, code) = run(&[&port, "--kill"]);
    let status = child.wait().unwrap();
    assert_eq!(code, Some(0), "{stdout}");
    assert!(stdout.contains("exited after SIGTERM"), "{stdout}");
    assert!(!status.success());
}

#[test]
fn force_escalates_when_sigterm_ignored() {
    let (mut child, port) = spawn_listener(true);
    let (stdout, code) = run(&[&port, "--kill", "--force"]);
    child.wait().unwrap();
    assert_eq!(code, Some(0), "{stdout}");
    assert!(stdout.contains("exited after SIGKILL"), "{stdout}");
}
