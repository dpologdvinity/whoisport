use crate::Info;
use std::io;
use std::os::windows::process::CommandExt;
use std::sync::OnceLock;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind, Users};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_INSUFFICIENT_BUFFER, HWND, LPARAM, NO_ERROR, WAIT_TIMEOUT,
};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCP6TABLE_OWNER_PID, MIB_TCPTABLE_OWNER_PID,
    TCP_TABLE_OWNER_PID_LISTENER,
};
use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};
use windows_sys::Win32::System::Console::{
    AttachConsole, CTRL_C_EVENT, FreeConsole, GenerateConsoleCtrlEvent, SetConsoleCtrlHandler,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
};
use windows_sys::core::BOOL;

pub const GRACEFUL: &str = "Ctrl+C/close";
pub const FORCEFUL: &str = "TerminateProcess";

const HELPER_ARG: &str = "__whoisport_ctrl_c";
const DETACHED_PROCESS: u32 = 0x0000_0008;

/// Raw GetExtendedTcpTable output for one address family, in a u32 buffer for alignment.
fn tcp_table(family: u16) -> Vec<u32> {
    let mut size = 0u32;
    let mut buf: Vec<u32> = Vec::new();
    loop {
        let r = unsafe {
            GetExtendedTcpTable(
                buf.as_mut_ptr().cast(),
                &mut size,
                0,
                family as u32,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        };
        match r {
            NO_ERROR => return buf,
            ERROR_INSUFFICIENT_BUFFER => buf = vec![0; (size as usize).div_ceil(4)],
            _ => return Vec::new(),
        }
    }
}

/// Every TCP listener as (port, pid), sorted. Windows reports the owner of every socket.
pub fn listeners() -> Vec<(u16, Option<u32>)> {
    let mut out = Vec::new();
    // Ports are in network byte order in the low 16 bits of a u32.
    let port = |p: u32| u16::from_be(p as u16);
    let v4 = tcp_table(AF_INET);
    if !v4.is_empty() {
        let t = v4.as_ptr() as *const MIB_TCPTABLE_OWNER_PID;
        let rows =
            unsafe { std::slice::from_raw_parts((*t).table.as_ptr(), (*t).dwNumEntries as usize) };
        out.extend(
            rows.iter()
                .map(|r| (port(r.dwLocalPort), Some(r.dwOwningPid))),
        );
    }
    let v6 = tcp_table(AF_INET6);
    if !v6.is_empty() {
        let t = v6.as_ptr() as *const MIB_TCP6TABLE_OWNER_PID;
        let rows =
            unsafe { std::slice::from_raw_parts((*t).table.as_ptr(), (*t).dwNumEntries as usize) };
        out.extend(
            rows.iter()
                .map(|r| (port(r.dwLocalPort), Some(r.dwOwningPid))),
        );
    }
    out.sort();
    out.dedup(); // the same process often listens on both IPv4 and IPv6
    out
}

fn users() -> &'static Users {
    static USERS: OnceLock<Users> = OnceLock::new();
    USERS.get_or_init(Users::new_with_refreshed_list)
}

pub fn info(pid: u32) -> Option<Info> {
    let p = Pid::from_u32(pid);
    let mut sys = System::new();
    let kind = ProcessRefreshKind::nothing()
        .with_cmd(UpdateKind::Always)
        .with_cwd(UpdateKind::Always)
        .with_exe(UpdateKind::Always)
        .with_user(UpdateKind::Always);
    sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[p]), true, kind);
    let proc = sys.process(p)?;
    let cmd: Vec<String> = proc
        .cmd()
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    // Elevated/system processes hide their command line; fall back to the exe path or name.
    let cmd = match cmd.join(" ") {
        c if !c.is_empty() => c,
        _ => proc
            .exe()
            .map(|e| e.display().to_string())
            .unwrap_or_else(|| proc.name().to_string_lossy().into_owned()),
    };
    let user = proc
        .user_id()
        .and_then(|u| users().get_user_by_id(u))
        .map_or("?".into(), |u| u.name().to_string());
    Some(Info {
        pid,
        ppid: proc.parent().map_or(0, |pp| pp.as_u32()),
        user,
        cmd,
        cwd: proc.cwd().map_or("?".into(), |c| c.display().to_string()),
        uptime_secs: proc.run_time(),
    })
}

pub fn alive(pid: u32) -> bool {
    unsafe {
        let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if h.is_null() {
            // Gone (invalid pid) vs. exists-but-protected (access denied).
            return io::Error::last_os_error().raw_os_error() == Some(5);
        }
        let running = WaitForSingleObject(h, 0) == WAIT_TIMEOUT;
        CloseHandle(h);
        running
    }
}

unsafe extern "system" fn close_if_owned(hwnd: HWND, pid: LPARAM) -> BOOL {
    let mut owner = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut owner);
        if owner != pid as u32 || IsWindowVisible(hwnd) == 0 {
            return 1;
        }
        // A console window reports the program inside it as its owner, but closing it closes
        // the whole terminal (and every process in it). Console apps get Ctrl+C instead.
        let mut class = [0u16; 64];
        let n = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32).max(0) as usize;
        let class = String::from_utf16_lossy(&class[..n]);
        if class != "ConsoleWindowClass" && class != "PseudoConsoleWindow" {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
        }
    }
    1
}

/// Windows has no SIGTERM. Ask politely the two ways programs actually listen for:
/// WM_CLOSE to its windows (GUI apps) and Ctrl+C in its console (CLI apps, dev servers).
pub fn ask_exit(pid: u32) -> io::Result<()> {
    unsafe {
        EnumWindows(Some(close_if_owned), pid as LPARAM);
        // If the target shares our console, the Ctrl+C reaches us too; ignore it.
        SetConsoleCtrlHandler(None, 1);
    }
    // A console-less child attaches to the target's console and sends Ctrl+C there,
    // like pressing Ctrl+C in that terminal. Done in a child so our own console is untouched.
    std::process::Command::new(std::env::current_exe()?)
        .args([HELPER_ARG, &pid.to_string()])
        .creation_flags(DETACHED_PROCESS)
        .status()?;
    Ok(())
}

/// Entry point for the Ctrl+C helper child spawned by `ask_exit`; exits if invoked as one.
pub fn run_helper_if_requested() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some(HELPER_ARG) {
        return;
    }
    let Some(pid) = args.get(2).and_then(|p| p.parse().ok()) else {
        std::process::exit(2);
    };
    let sent = unsafe {
        FreeConsole();
        AttachConsole(pid) != 0
            && SetConsoleCtrlHandler(None, 1) != 0
            && GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0) != 0
    };
    std::process::exit(if sent { 0 } else { 1 });
}

pub fn terminate(pid: u32) -> io::Result<()> {
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if h.is_null() {
            return Err(io::Error::last_os_error());
        }
        let ok = TerminateProcess(h, 1) != 0;
        let err = io::Error::last_os_error();
        CloseHandle(h);
        if ok { Ok(()) } else { Err(err) }
    }
}
