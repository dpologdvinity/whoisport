//! Windows backend.
//!
//! * **Finding listeners:** the IP Helper API (`GetExtendedTcpTable` /
//!   `GetExtendedUdpTable`) returns every socket together with the pid that owns it.
//!   Unlike Linux, Windows reports the owner of every socket, so no row is ever hidden.
//! * **Process details:** shared with macOS, in [`crate::sysinfo_info`].
//! * **Stopping a process:** Windows has no `SIGTERM`, so asking politely takes two
//!   attempts ([`ask_exit`]): `WM_CLOSE` to the program's windows (what clicking the X
//!   does) and Ctrl+C in its console (what pressing Ctrl+C in that terminal does). Forcing
//!   is `TerminateProcess` ([`terminate`]).
//!
//! # Why a helper process sends Ctrl+C
//!
//! A Windows console event can only be sent from a process attached to the target's
//! console, and attaching moves the process off its own console. So whoisport runs *itself*
//! as a short-lived console-less helper (`__whoisport_ctrl_c <pid>`), which attaches,
//! sends Ctrl+C, and exits; whoisport's own console is never touched. Ctrl+C reaches *every*
//! process on that console, so the helper first refuses if the console is shared with
//! programs that are not the target ([`console_shared`]).

pub use crate::sysinfo_info::info;
use std::io;
use std::os::windows::process::CommandExt;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_INSUFFICIENT_BUFFER, HWND, LPARAM, NO_ERROR, WAIT_TIMEOUT,
};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, GetExtendedUdpTable, MIB_TCP6TABLE_OWNER_PID, MIB_TCPTABLE_OWNER_PID,
    MIB_UDP6TABLE_OWNER_PID, MIB_UDPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
    UDP_TABLE_OWNER_PID,
};
use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};
use windows_sys::Win32::System::Console::{
    AttachConsole, CTRL_C_EVENT, FreeConsole, GenerateConsoleCtrlEvent, GetConsoleProcessList,
    SetConsoleCtrlHandler,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
};
use windows_sys::core::BOOL;

/// Name of the polite step, shown in messages such as `exited after Ctrl+C/close`.
pub const GRACEFUL: &str = "Ctrl+C/close";
/// Name of the forced step, shown in messages such as `exited after TerminateProcess`.
pub const FORCEFUL: &str = "TerminateProcess";

/// The hidden first argument that makes whoisport behave as the Ctrl+C helper.
const HELPER_ARG: &str = "__whoisport_ctrl_c";
/// `CreationFlags` value: start the helper with no console, so it can attach to another.
const DETACHED_PROCESS: u32 = 0x0000_0008;
/// The helper's exit code meaning "the console is shared; Ctrl+C was not sent".
const HELPER_SHARED_CONSOLE: i32 = 3;

/// The raw `GetExtended{Tcp,Udp}Table` output for one address family.
///
/// The API is called twice: first with no buffer, to learn the size needed
/// (`ERROR_INSUFFICIENT_BUFFER`), then with a buffer of that size. The table can grow
/// between the calls, so this loops until the call succeeds. The buffer is a `Vec<u32>`
/// rather than bytes so that it is 4-byte aligned, as the table structs require. An empty
/// vector means the call failed.
fn socket_table(family: u16, udp: bool) -> Vec<u32> {
    let mut size = 0u32;
    let mut buf: Vec<u32> = Vec::new();
    loop {
        let (ptr, af) = (buf.as_mut_ptr().cast(), family as u32);
        // SAFETY: `ptr` is null (size query) or points to `buf`, which is at least `size`
        // bytes as reported by the previous call; `size` is a valid out-parameter.
        let r = unsafe {
            match udp {
                false => {
                    GetExtendedTcpTable(ptr, &mut size, 0, af, TCP_TABLE_OWNER_PID_LISTENER, 0)
                }
                true => GetExtendedUdpTable(ptr, &mut size, 0, af, UDP_TABLE_OWNER_PID, 0),
            }
        };
        match r {
            NO_ERROR => return buf,
            ERROR_INSUFFICIENT_BUFFER => buf = vec![0; (size as usize).div_ceil(4)],
            _ => return Vec::new(),
        }
    }
}

/// Turn a table buffer from [`socket_table`] into `(port, pid)` pairs.
///
/// `$table` is the Windows struct describing the buffer's layout: a row count
/// (`dwNumEntries`) followed by that many rows, each with `dwLocalPort` and
/// `dwOwningPid`. It is a macro because the TCP, TCP6, UDP and UDP6 structs are different
/// types with the same field names.
macro_rules! rows {
    ($buf:expr, $table:ty) => {{
        let buf = $buf;
        if buf.is_empty() {
            Vec::new()
        } else {
            let t = buf.as_ptr() as *const $table;
            // SAFETY: a successful `socket_table` call filled `buf` with a valid `$table`
            // (aligned, `dwNumEntries` rows long), and `buf` outlives this slice.
            let rows = unsafe {
                std::slice::from_raw_parts((*t).table.as_ptr(), (*t).dwNumEntries as usize)
            };
            // Ports are in network byte order in the low 16 bits of a u32.
            rows.iter()
                .map(|r| (u16::from_be(r.dwLocalPort as u16), Some(r.dwOwningPid)))
                .collect::<Vec<_>>()
        }
    }};
}

/// Every listener as `(port, pid)`, sorted. Windows reports the owner of every socket, so
/// `pid` is always `Some`.
///
/// For TCP, only sockets in the LISTEN state are returned. UDP has no listen state, and
/// the Windows UDP table has no remote address either, so every bound UDP socket counts,
/// including short-lived client sockets. A process listening on both IPv4 and IPv6 is
/// listed once.
pub fn listeners(udp: bool) -> Vec<(u16, Option<u32>)> {
    let mut out = match udp {
        false => [
            rows!(socket_table(AF_INET, false), MIB_TCPTABLE_OWNER_PID),
            rows!(socket_table(AF_INET6, false), MIB_TCP6TABLE_OWNER_PID),
        ],
        true => [
            rows!(socket_table(AF_INET, true), MIB_UDPTABLE_OWNER_PID),
            rows!(socket_table(AF_INET6, true), MIB_UDP6TABLE_OWNER_PID),
        ],
    }
    .concat();
    out.sort();
    out.dedup(); // the same process often listens on both IPv4 and IPv6
    out
}

/// Is the process still running?
///
/// Opens a handle with the right to wait on the process and polls it without blocking
/// (timeout 0): the handle is "signalled" once the process has exited. If the handle
/// cannot be opened, the process is gone, unless the error is "access denied" (error 5),
/// which means it exists but is protected.
pub fn alive(pid: u32) -> bool {
    // SAFETY: plain Win32 calls; the handle is closed before returning.
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

/// `EnumWindows` callback: post `WM_CLOSE` to the visible windows owned by process `pid`.
///
/// Returns 1 so the enumeration continues through every window. `WM_CLOSE` is a request,
/// not a command: the program may ask "save changes?" or ignore it. Console windows are
/// skipped: they report the program running *inside* them as their owner, but closing one
/// closes the whole terminal and every process in it. Console programs are asked with
/// Ctrl+C instead.
///
/// # Safety
///
/// Only called by Windows as an `EnumWindows` callback with a valid window handle.
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

/// Ask the process to exit, the two ways Windows programs listen for: `WM_CLOSE` to its
/// windows (GUI apps) and Ctrl+C in its console (command-line apps, dev servers).
///
/// Succeeds once the request has been sent; the caller waits to see whether the process
/// actually exits. Fails when the process's console is shared with other programs, because
/// Ctrl+C would stop them too (see the module docs).
pub fn ask_exit(pid: u32) -> io::Result<()> {
    // SAFETY: the callback matches the required signature and only reads window state.
    unsafe {
        EnumWindows(Some(close_if_owned), pid as LPARAM);
    }
    // A console-less child attaches to the target's console and sends Ctrl+C there,
    // like pressing Ctrl+C in that terminal. Done in a child so our own console is untouched.
    let status = std::process::Command::new(std::env::current_exe()?)
        .args([HELPER_ARG, &pid.to_string()])
        .creation_flags(DETACHED_PROCESS)
        .status()?;
    match status.code() {
        Some(HELPER_SHARED_CONSOLE) => Err(io::Error::other(
            "its console is shared with other programs, so Ctrl+C was not sent (use --force)",
        )),
        // No console to attach to (GUI app or service): WM_CLOSE was the only way to ask.
        _ => Ok(()),
    }
}

/// Would Ctrl+C on the console listing `on_console` reach anything besides `target`, its
/// descendants, plain shells, and this helper?
///
/// Ctrl+C goes to every process on a console, so an unrelated program there (another dev
/// server, a parent like `concurrently`, or the user's own whoisport) would be interrupted
/// too. Shells (`cmd`, PowerShell) are tolerated because they only host the target.
fn console_shared(target: u32, on_console: &[u32]) -> bool {
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    let descends_from_target = |mut pid: u32| {
        for _ in 0..64 {
            if pid == target {
                return true;
            }
            match sys.process(Pid::from_u32(pid)).and_then(|p| p.parent()) {
                Some(parent) if parent.as_u32() != pid => pid = parent.as_u32(),
                _ => return false,
            }
        }
        false
    };
    let is_shell = |pid: u32| {
        sys.process(Pid::from_u32(pid)).is_some_and(|p| {
            let name = p.name().to_string_lossy().to_ascii_lowercase();
            matches!(name.as_str(), "cmd.exe" | "powershell.exe" | "pwsh.exe")
        })
    };
    let me = std::process::id();
    on_console
        .iter()
        .any(|&pid| pid != me && !descends_from_target(pid) && !is_shell(pid))
}

/// Run the Ctrl+C helper if this process was started as one; otherwise return at once.
///
/// Called first thing in `main`. As the helper (`whoisport __whoisport_ctrl_c <pid>`) it
/// attaches to the target's console, refuses if that console is shared, otherwise sends
/// Ctrl+C, and exits. Exit codes: `0` sent, `1` could not attach or send (for example the
/// target has no console), `2` bad arguments, [`HELPER_SHARED_CONSOLE`] refused.
pub fn run_helper_if_requested() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some(HELPER_ARG) {
        return;
    }
    let Some(pid) = args.get(2).and_then(|p| p.parse().ok()) else {
        std::process::exit(2);
    };
    // SAFETY: plain Win32 console calls; `ids` is a valid buffer of the length passed.
    unsafe {
        FreeConsole();
        if AttachConsole(pid) == 0 {
            std::process::exit(1);
        }
        let mut ids = [0u32; 64];
        let n = GetConsoleProcessList(ids.as_mut_ptr(), ids.len() as u32) as usize;
        // n > len means more processes than fit: certainly shared.
        if n == 0 || n > ids.len() || console_shared(pid, &ids[..n]) {
            std::process::exit(HELPER_SHARED_CONSOLE);
        }
        // Ignore Ctrl+C in this helper itself, so the event it sends cannot stop it first.
        let sent =
            SetConsoleCtrlHandler(None, 1) != 0 && GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0) != 0;
        std::process::exit(if sent { 0 } else { 1 });
    }
}

/// Make the process exit now (`TerminateProcess`); it gets no chance to clean up.
pub fn terminate(pid: u32) -> io::Result<()> {
    // SAFETY: plain Win32 calls; the handle is closed before returning.
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
