//! Shared Unix process-group signalling and bounded process inspection.
use std::io;

pub(crate) fn signal_group(group: libc::pid_t, signal: libc::c_int) -> io::Result<()> {
    if group <= 1 {
        return Err(io::Error::other("refusing invalid process group"));
    }
    // SAFETY: a validated positive PGID is negated to address only its group.
    if unsafe { libc::kill(-group, signal) } == -1 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            // Darwin returns EPERM for an existing group consisting solely of
            // zombies (not ESRCH). Do not swallow a genuine permission error:
            // verify that there are no live group members before accepting it.
            #[cfg(target_os = "macos")]
            if error.raw_os_error() == Some(libc::EPERM) && zombie_only_group(group)? {
                return Ok(());
            }
            return Err(error);
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) fn zombie_only_group(group: libc::pid_t) -> io::Result<bool> {
    for pid in process_ids()? {
        // SAFETY: this is a metadata query, not a signal.
        if pid <= 1 || unsafe { libc::getpgid(pid) } != group {
            continue;
        }
        let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
        let size = std::mem::size_of::<libc::proc_bsdinfo>();
        // SAFETY: the destination has exactly the size advertised to libproc.
        let read = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast(),
                size as libc::c_int,
            )
        };
        if read != size as libc::c_int {
            // It may have been reaped meanwhile. Unreadable metadata for a
            // still-present member is not proof that signalling is unnecessary.
            if unsafe { libc::getpgid(pid) } == group {
                return Ok(false);
            }
            continue;
        }
        // SAFETY: a successful full-size libproc read initialized the struct.
        let info = unsafe { info.assume_init() };
        if info.pbi_pgid == group as u32 && info.pbi_status != libc::SZOMB {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(target_os = "macos")]
pub(crate) fn process_ids() -> io::Result<Vec<libc::pid_t>> {
    let mut ids = vec![0; 4096];
    loop {
        // SAFETY: ids provides exactly the advertised initialized buffer capacity.
        let count = unsafe {
            libc::proc_listallpids(
                ids.as_mut_ptr().cast(),
                std::mem::size_of_val(ids.as_slice()) as libc::c_int,
            )
        };
        if count < 0 {
            return Err(io::Error::last_os_error());
        }
        if (count as usize) < ids.len() {
            ids.truncate(count as usize);
            return Ok(ids);
        }
        if ids.len() >= 262_144 {
            return Err(io::Error::other("process table exceeds cleanup bound"));
        }
        ids.resize(ids.len() * 2, 0);
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn process_ids() -> io::Result<Vec<libc::pid_t>> {
    let mut ids = Vec::new();
    for entry in std::fs::read_dir("/proc")? {
        if let Ok(entry) = entry
            && let Some(name) = entry.file_name().to_str()
            && let Ok(pid) = name.parse()
        {
            ids.push(pid);
        }
    }
    Ok(ids)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn process_ids() -> io::Result<Vec<libc::pid_t>> {
    Ok(Vec::new())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::os::unix::process::CommandExt;

    #[test]
    fn repeated_kill_of_own_zombie_group_is_idempotent() {
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("30")
            .process_group(0)
            .spawn()
            .unwrap();
        let group = child.id() as libc::pid_t;
        let first = signal_group(group, libc::SIGKILL);
        std::thread::sleep(std::time::Duration::from_millis(50));
        let repeated = signal_group(group, libc::SIGKILL);
        let status = child.wait();
        first.unwrap();
        repeated.unwrap();
        assert!(!status.unwrap().success());
    }
}
