//! Stopping a process on Unix (Linux and macOS) with signals.
//!
//! The two steps the kill logic uses are exposed with the same names on every platform
//! (see `proc_windows.rs` for the Windows equivalents): [`ask_exit`] sends `SIGTERM`, a
//! polite request a program can handle (flush files, close connections, then exit), and
//! [`terminate`] sends `SIGKILL`, which the kernel enforces and a program cannot ignore.

use std::io;

/// Name of the polite step, shown in messages such as `exited after SIGTERM`.
pub const GRACEFUL: &str = "SIGTERM";
/// Name of the forced step, shown in messages such as `exited after SIGKILL`.
pub const FORCEFUL: &str = "SIGKILL";

/// Send `sig` to `pid`; the operating system's error (for example `EPERM` for another
/// user's process) is returned as-is.
fn signal(pid: u32, sig: libc::c_int) -> io::Result<()> {
    if pid == 0 || pid > i32::MAX as u32 {
        return Err(io::Error::from_raw_os_error(libc::EINVAL));
    }
    // SAFETY: kill(2) takes plain integers and has no memory-safety preconditions.
    match unsafe { libc::kill(pid as i32, sig) } {
        0 => Ok(()),
        _ => Err(io::Error::last_os_error()),
    }
}

/// Ask the process to exit (`SIGTERM`). Success means the request was delivered, not that
/// the process has exited.
pub fn ask_exit(pid: u32) -> io::Result<()> {
    signal(pid, libc::SIGTERM)
}

/// Make the process exit now (`SIGKILL`); it gets no chance to clean up.
pub fn terminate(pid: u32) -> io::Result<()> {
    signal(pid, libc::SIGKILL)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `kill(0, ..)` signals the caller's whole process group and a negative pid signals
    /// another group, so pids that would become 0 or negative as an `i32` must be refused.
    /// Signal 0 is used: it delivers nothing, so even if the guard regressed this test
    /// could not harm anything (it would fail instead).
    #[test]
    fn pids_that_would_address_a_process_group_are_refused() {
        for pid in [0, i32::MAX as u32 + 1, u32::MAX] {
            let err = signal(pid, 0).unwrap_err();
            assert_eq!(err.raw_os_error(), Some(libc::EINVAL), "pid {pid}");
        }
    }

    #[test]
    fn a_real_pid_can_be_probed() {
        assert!(signal(std::process::id(), 0).is_ok());
    }
}
