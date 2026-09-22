use portable_pty::MasterPty;
use std::{
    collections::BTreeSet,
    io::{self, Read, Write},
};

pub(crate) struct PtyIo {
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
}

impl PtyIo {
    pub(crate) fn new(master: &dyn MasterPty) -> io::Result<Self> {
        let fd = master
            .as_raw_fd()
            .ok_or_else(|| io::Error::other("PTY has no descriptor"))?;
        // SAFETY: master owns a live descriptor. O_NONBLOCK is shared by its
        // cloned reader/writer, but not by the independent slave descriptor.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            reader: master.try_clone_reader().map_err(super::super::other)?,
            writer: master.take_writer().map_err(super::super::other)?,
        })
    }

    pub(crate) fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self.reader.read(buffer) {
            // Linux reports EIO when the PTY slave closes; macOS reports EOF.
            Err(error) if error.raw_os_error() == Some(libc::EIO) => Ok(0),
            result => result,
        }
    }

    pub(crate) fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writer.write(bytes)
    }
}

pub(crate) fn signal_group(group: libc::pid_t, signal: libc::c_int) -> io::Result<()> {
    if group <= 1 {
        return Err(io::Error::other("refusing invalid PTY process group"));
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

pub(crate) fn signal_session(pid: u32, signal: libc::c_int) -> io::Result<()> {
    let session = libc::pid_t::try_from(pid)
        .ok()
        .filter(|p| *p > 1)
        .ok_or_else(|| io::Error::other("invalid PTY session id"))?;
    // An interactive shell can put foreground AND background jobs into their
    // own process groups. Killing only the shell's group would leak those jobs.
    let mut groups = BTreeSet::from([session]);
    for candidate in process_ids()? {
        // SAFETY: these calls inspect process metadata only. Revalidate the
        // session immediately before collecting a group to exclude unrelated jobs.
        if candidate > 1 && unsafe { libc::getsid(candidate) } == session {
            let group = unsafe { libc::getpgid(candidate) };
            if group > 1 {
                groups.insert(group);
            }
        }
    }
    // Keep the shell alive until its jobs receive the signal, where possible.
    let mut error = None;
    for group in groups
        .into_iter()
        .filter(|group| *group != session)
        .chain([session])
    {
        if let Err(next) = signal_group(group, signal) {
            error.get_or_insert(next);
        }
    }
    error.map_or(Ok(()), Err)
}

#[cfg(target_os = "macos")]
fn zombie_only_group(group: libc::pid_t) -> io::Result<bool> {
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
fn process_ids() -> io::Result<Vec<libc::pid_t>> {
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
fn process_ids() -> io::Result<Vec<libc::pid_t>> {
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
fn process_ids() -> io::Result<Vec<libc::pid_t>> {
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
