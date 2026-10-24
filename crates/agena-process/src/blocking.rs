//! Managed process trees for synchronous launchers running on blocking workers.
use std::io;
use std::process::{Command, ExitStatus, Output};
use std::time::{Duration, Instant};

#[cfg(windows)]
use process_wrap::std::JobObject;
#[cfg(unix)]
use process_wrap::std::ProcessSession;
use process_wrap::std::{ChildWrapper, CommandWrap};

#[derive(Debug)]
pub struct ManagedChild {
    inner: Box<dyn ChildWrapper>,
    initial_id: Option<u32>,
    tree_killed: bool,
}

pub fn spawn(command: Command) -> io::Result<ManagedChild> {
    let mut wrapped = CommandWrap::from(command);
    #[cfg(unix)]
    wrapped.wrap(ProcessSession);
    #[cfg(windows)]
    wrapped.wrap(JobObject);
    let inner = wrapped.spawn()?;
    Ok(ManagedChild {
        initial_id: Some(inner.id()),
        inner,
        tree_killed: false,
    })
}

impl ManagedChild {
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let status = self.inner.try_wait()?;
        if status.is_some() {
            // Observing the leader exit also ends its remaining descendants.
            // Record the kill so subsequent polls never signal a reused PGID.
            self.start_kill()?;
        }
        Ok(status)
    }

    fn start_kill(&mut self) -> io::Result<()> {
        if self.tree_killed {
            return Ok(());
        }
        match self.inner.start_kill() {
            Ok(()) => {}
            Err(error) if super::process_tree_is_finished(&error, self.initial_id) => {}
            Err(error) => return Err(error),
        }
        self.tree_killed = true;
        Ok(())
    }

    pub fn terminate(&mut self, timeout: Duration) -> io::Result<ExitStatus> {
        self.start_kill()?;
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.inner.try_wait()? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "managed process did not exit after tree termination",
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// Bounded synchronous collection using the async managed process-tree and
/// pipe policy. A dedicated thread avoids nesting Tokio on the caller's
/// thread. Pipe readers never outlive the operation.
pub fn output(command: Command, timeout: Duration, maximum_bytes: usize) -> io::Result<Output> {
    std::thread::Builder::new()
        .name("agena-process-output".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(super::output(command.into(), timeout, maximum_bytes))
        })?
        .join()
        .map_err(|_| io::Error::other("managed output worker panicked"))?
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        if let Err(error) = self.terminate(Duration::from_secs(2)) {
            tracing::error!(%error, "failed to clean up a synchronous managed process tree");
        }
    }
}
