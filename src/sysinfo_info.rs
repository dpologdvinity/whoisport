//! Process details via sysinfo, for platforms without a /proc to read (Windows, macOS).
use crate::Info;
use std::sync::OnceLock;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind, Users};

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
