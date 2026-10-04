use crate::unix::process_ids;
pub(crate) use crate::unix::signal_group;
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
