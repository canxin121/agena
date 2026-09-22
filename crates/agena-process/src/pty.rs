//! Owned pseudo-terminal with bounded, nonblocking I/O and process-session cleanup.
//!
//! The caller drives `read`, `write` and `try_wait` from a dedicated worker.
//! Terminal key bytes and out-of-band signals deliberately remain distinct.

use portable_pty::{Child, CommandBuilder, ExitStatus, MasterPty, PtySize};
use std::{collections::HashMap, io, path::Path};

mod platform;
use platform::PtyIo;

pub struct PtyProcess {
    master: Option<Box<dyn MasterPty + Send>>,
    child: Option<Box<dyn Child + Send + Sync>>,
    io: Option<PtyIo>,
    pid: u32,
    #[cfg(windows)]
    job: platform::Job,
    cleaned: bool,
}

impl PtyProcess {
    pub fn spawn(
        command: &[String],
        cwd: &Path,
        env: &HashMap<String, String>,
        rows: u16,
        cols: u16,
    ) -> io::Result<Self> {
        let (program, args) = command
            .split_first()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty terminal command"))?;
        let pair = portable_pty::native_pty_system()
            .openpty(size(rows, cols))
            .map_err(other)?;
        let mut builder = CommandBuilder::new(program);
        builder.args(args);
        builder.cwd(cwd);
        builder.env_clear();
        for (key, value) in env {
            builder.env(key, value);
        }
        let io = PtyIo::new(pair.master.as_ref())?;
        let mut child = pair.slave.spawn_command(builder).map_err(other)?;
        drop(pair.slave);
        let pid = child.process_id().ok_or_else(|| {
            let _ = child.kill();
            io::Error::other("PTY child did not provide a process identity")
        })?;
        #[cfg(windows)]
        let job = match platform::Job::attach(child.as_ref()) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        Ok(Self {
            master: Some(pair.master),
            child: Some(child),
            io: Some(io),
            pid,
            #[cfg(windows)]
            job,
            cleaned: false,
        })
    }

    pub fn id(&self) -> u32 {
        self.pid
    }

    /// Returns WouldBlock when no output is currently available and zero at EOF.
    pub fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.io.as_mut().ok_or_else(closed)?.read(buffer)
    }

    /// A partial write is not retried automatically. The caller must track the
    /// acknowledged byte count and stop the process on an ambiguous delivery failure.
    pub fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.io.as_mut().ok_or_else(closed)?.write(bytes)
    }

    pub fn resize(&self, rows: u16, cols: u16) -> io::Result<()> {
        self.master
            .as_ref()
            .ok_or_else(closed)?
            .resize(size(rows, cols))
            .map_err(other)
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.as_mut().ok_or_else(closed)?.try_wait()
    }

    /// Signal the current foreground process group, without destroying the shell.
    pub fn interrupt(&mut self) -> io::Result<()> {
        #[cfg(unix)]
        {
            let group = self
                .master
                .as_ref()
                .and_then(|m| m.process_group_leader())
                .unwrap_or(self.pid as libc::pid_t);
            platform::signal_group(group, libc::SIGINT)
        }
        #[cfg(windows)]
        {
            self.io.as_mut().ok_or_else(closed)?.interrupt()
        }
    }

    pub fn terminate(&mut self) -> io::Result<()> {
        #[cfg(unix)]
        {
            platform::signal_session(self.pid, libc::SIGTERM)
        }
        #[cfg(windows)]
        {
            self.job.terminate()
        }
    }

    pub fn kill(&mut self) -> io::Result<()> {
        #[cfg(unix)]
        let result = platform::signal_session(self.pid, libc::SIGKILL);
        #[cfg(windows)]
        let result = self.job.terminate();
        if result.is_ok() {
            self.cleaned = true;
        }
        result
    }
}

impl Drop for PtyProcess {
    fn drop(&mut self) {
        if !self.cleaned
            && let Err(error) = self.kill()
        {
            tracing::error!(pid = self.pid, %error, "PTY process-session cleanup failed");
            if let Some(child) = self.child.as_mut()
                && let Err(error) = child.kill()
            {
                tracing::error!(pid = self.pid, %error, "PTY child cleanup fallback failed");
            }
        }
        // Release queues/readers before ClosePseudoConsole, which must be able
        // to finish draining its output. Unix nonblocking descriptors also close here.
        self.io.take();
        self.master.take();
        // Normally the driver has already reaped the child. A cancelled driver
        // must still arrange reaping without blocking the runtime's destructor.
        if let Some(mut child) = self.child.take()
            && !matches!(child.try_wait(), Ok(Some(_)))
        {
            let pid = self.pid;
            if let Err(error) = std::thread::Builder::new()
                .name("agena-pty-reap".into())
                .spawn(move || {
                    if let Err(error) = child.wait() {
                        tracing::error!(pid, %error, "PTY fallback reaping failed");
                    }
                })
            {
                tracing::error!(pid, %error, "could not start PTY fallback reaper");
            }
        }
    }
}

fn size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}
fn other(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}
fn closed() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "terminal is closed")
}
