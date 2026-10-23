//! SIGTERM / SIGKILL, shared by the Linux and macOS backends.
use std::io;

pub const GRACEFUL: &str = "SIGTERM";
pub const FORCEFUL: &str = "SIGKILL";

fn signal(pid: u32, sig: libc::c_int) -> io::Result<()> {
    match unsafe { libc::kill(pid as i32, sig) } {
        0 => Ok(()),
        _ => Err(io::Error::last_os_error()),
    }
}

pub fn ask_exit(pid: u32) -> io::Result<()> {
    signal(pid, libc::SIGTERM)
}

pub fn terminate(pid: u32) -> io::Result<()> {
    signal(pid, libc::SIGKILL)
}
