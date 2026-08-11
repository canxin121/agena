//! Cross-platform Tokio subprocess lifecycle policy.
//!
//! `process-wrap` supplies the OS integration: Unix process groups, Windows
//! Job Objects, and Tokio kill-on-drop. Agena adds one small policy layer so
//! every long-lived child uses the same wrappers and bounded termination.

use std::io;
use std::process::{ExitStatus, Output, Stdio};
use std::time::Duration;

#[cfg(windows)]
use process_wrap::tokio::JobObject;
#[cfg(unix)]
use process_wrap::tokio::ProcessSession;
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout, Command};

/// Apply Agena's standard process-tree wrappers to a configured command.
///
/// This is public so transports such as `rmcp`, which already accept a
/// `CommandWrap`, can keep ownership of their own child while sharing the
/// exact same process-tree behavior.
pub fn wrap_command(command: Command) -> CommandWrap {
    let mut command = CommandWrap::from(command);
    command.wrap(KillOnDrop);
    #[cfg(unix)]
    command.wrap(ProcessSession);
    #[cfg(windows)]
    command.wrap(JobObject);
    command
}

/// Spawn a configured Tokio command as one managed process tree.
pub fn spawn(command: Command) -> io::Result<ManagedChild> {
    Ok(ManagedChild {
        inner: wrap_command(command).spawn()?,
    })
}

async fn read_bounded<R>(mut reader: R, maximum_bytes: usize) -> io::Result<(Vec<u8>, bool)>
where
    R: AsyncRead + Unpin,
{
    let mut retained = Vec::new();
    let mut exceeded = false;
    let mut chunk = [0_u8; 16 * 1024];
    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        let remaining = maximum_bytes.saturating_sub(retained.len());
        retained.extend_from_slice(&chunk[..read.min(remaining)]);
        exceeded |= read > remaining;
    }
    Ok((retained, exceeded))
}

/// Run a non-interactive command with bounded stdout/stderr and a process-tree
/// timeout. Both streams continue to be drained after reaching the limit so a
/// noisy child cannot deadlock on a full pipe.
pub async fn output(
    mut command: Command,
    deadline: Duration,
    maximum_bytes_per_stream: usize,
) -> io::Result<Output> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = spawn(command)?;
    let stdout = child
        .stdout()
        .take()
        .ok_or_else(|| io::Error::other("managed child stdout is unavailable"))?;
    let stderr = child
        .stderr()
        .take()
        .ok_or_else(|| io::Error::other("managed child stderr is unavailable"))?;
    let stdout_task = tokio::spawn(read_bounded(stdout, maximum_bytes_per_stream));
    let stderr_task = tokio::spawn(read_bounded(stderr, maximum_bytes_per_stream));

    let status = match tokio::time::timeout(deadline, child.wait()).await {
        Ok(result) => result?,
        Err(_) => {
            let _ = child.terminate(Duration::from_millis(100)).await;
            stdout_task.abort();
            stderr_task.abort();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "managed process timed out",
            ));
        }
    };

    // A command can exit after leaving a descendant that still owns its pipes.
    // End the managed process tree before waiting for EOF from both readers.
    let _ = child.start_kill();
    let join_readers = async {
        let stdout = stdout_task
            .await
            .map_err(|error| io::Error::other(format!("stdout reader failed: {error}")))??;
        let stderr = stderr_task
            .await
            .map_err(|error| io::Error::other(format!("stderr reader failed: {error}")))??;
        Ok::<_, io::Error>((stdout, stderr))
    };
    let ((stdout, stdout_exceeded), (stderr, stderr_exceeded)) =
        tokio::time::timeout(Duration::from_secs(2), join_readers)
            .await
            .map_err(|_| {
                io::Error::new(io::ErrorKind::TimedOut, "process pipes did not close")
            })??;

    if stdout_exceeded || stderr_exceeded {
        return Err(io::Error::new(
            io::ErrorKind::FileTooLarge,
            format!("process output exceeded {maximum_bytes_per_stream} bytes per stream"),
        ));
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// A child whose signals target the complete Unix process group or Windows
/// Job Object. Dropping the handle requests a non-blocking tree kill.
#[derive(Debug)]
pub struct ManagedChild {
    inner: Box<dyn ChildWrapper>,
}

impl ManagedChild {
    pub fn id(&self) -> Option<u32> {
        self.inner.id()
    }

    pub fn stdin(&mut self) -> &mut Option<ChildStdin> {
        self.inner.stdin()
    }

    pub fn stdout(&mut self) -> &mut Option<ChildStdout> {
        self.inner.stdout()
    }

    pub fn stderr(&mut self) -> &mut Option<ChildStderr> {
        self.inner.stderr()
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.inner.try_wait()
    }

    pub async fn wait(&mut self) -> io::Result<ExitStatus> {
        self.inner.wait().await
    }

    /// Request immediate termination of the entire process tree.
    pub fn start_kill(&mut self) -> io::Result<()> {
        self.inner.start_kill()
    }

    /// Give the process tree a short graceful window, then force-kill and
    /// reap it. The deadline prevents shutdown from waiting forever.
    pub async fn terminate(&mut self, grace: Duration) -> io::Result<ExitStatus> {
        #[cfg(unix)]
        let _ = self.inner.signal(libc::SIGTERM);
        #[cfg(not(unix))]
        let _ = self.inner.start_kill();

        match tokio::time::timeout(grace, self.inner.wait()).await {
            Ok(status) => {
                let status = status?;
                // The direct child may exit on SIGTERM while a descendant in
                // the same group ignores it. A final group/job kill closes
                // that race; an already-empty group simply returns ESRCH.
                let _ = self.inner.start_kill();
                Ok(status)
            }
            Err(_) => {
                let _ = self.inner.start_kill();
                self.inner.wait().await
            }
        }
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        // The wrapper translates this to killpg(2) or TerminateJobObject,
        // unlike Tokio's raw Child kill-on-drop which only targets one PID.
        let _ = self.inner.start_kill();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::{output, spawn};
    use std::io;
    use std::process::Stdio;
    use std::time::Duration;
    use tokio::process::Command;

    #[tokio::test]
    async fn termination_is_bounded() {
        let mut command = Command::new("sh");
        command
            .args(["-c", "trap '' TERM; sleep 30"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = spawn(command).expect("spawn managed child");

        let status = tokio::time::timeout(
            Duration::from_secs(2),
            child.terminate(Duration::from_millis(25)),
        )
        .await
        .expect("termination deadline")
        .expect("terminate child");

        assert!(!status.success());
    }

    #[tokio::test]
    async fn output_timeout_terminates_descendants() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 30 & wait"]).stdin(Stdio::null());

        let error = output(command, Duration::from_millis(25), 1024)
            .await
            .expect_err("command must time out");

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[tokio::test]
    async fn output_is_bounded_while_draining_pipes() {
        let mut command = Command::new("sh");
        command.args(["-c", "printf 12345"]).stdin(Stdio::null());

        let error = output(command, Duration::from_secs(1), 4)
            .await
            .expect_err("output must exceed the limit");

        assert_eq!(error.kind(), io::ErrorKind::FileTooLarge);
    }
}
