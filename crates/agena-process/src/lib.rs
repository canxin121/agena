//! Cross-platform Tokio subprocess lifecycle policy.
//!
//! `process-wrap` supplies the OS integration: Unix process groups, Windows
//! Job Objects, and Tokio kill-on-drop. Agena adds one small policy layer so
//! every long-lived child uses the same wrappers and bounded termination.

use std::io;
use std::process::{ExitStatus, Output, Stdio};
use std::time::Duration;

pub mod blocking;
pub mod pty;
pub mod shell;
#[cfg(unix)]
mod unix;

#[cfg(windows)]
use process_wrap::tokio::JobObject;
#[cfg(unix)]
use process_wrap::tokio::ProcessSession;
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout, Command};

fn process_tree_is_finished(error: &io::Error, initial_id: Option<u32>) -> bool {
    #[cfg(not(target_os = "macos"))]
    let _ = initial_id;
    if matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::InvalidInput
    ) {
        return true;
    }

    // macOS maps killpg(2)'s ESRCH to `Uncategorized`, so checking only the
    // portable ErrorKind turns an already-empty process group into a false
    // termination failure after the direct child has been reaped.
    #[cfg(unix)]
    if error.raw_os_error() == Some(libc::ESRCH) {
        return true;
    }

    // Darwin can retain an all-zombie group after the leader is reaped and
    // report EPERM instead of ESRCH. Accept that only after bounded metadata
    // inspection proves no live members remain; errors preserve the original
    // failure. The initial ID survives Tokio clearing Child::id() after exit.
    #[cfg(target_os = "macos")]
    if error.raw_os_error() == Some(libc::EPERM)
        && let Some(group) = initial_id.and_then(|id| libc::pid_t::try_from(id).ok())
        && group > 1
        && matches!(unix::zombie_only_group(group), Ok(true))
    {
        return true;
    }

    false
}

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
    let inner = wrap_command(command).spawn()?;
    Ok(ManagedChild {
        initial_id: inner.id(),
        inner,
        tree_killed: false,
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
    command: Command,
    deadline: Duration,
    maximum_bytes_per_stream: usize,
) -> io::Result<Output> {
    output_inner(command, None, deadline, maximum_bytes_per_stream).await
}

/// Send structured input over stdin instead of exposing it in argv. Input
/// delivery and process completion share the same deadline; stdout/stderr are
/// drained concurrently. Callers bound the request size before invoking this.
pub async fn output_with_input(
    command: Command,
    input: &[u8],
    deadline: Duration,
    maximum_bytes_per_stream: usize,
) -> io::Result<Output> {
    output_inner(command, Some(input), deadline, maximum_bytes_per_stream).await
}

async fn output_inner(
    mut command: Command,
    input: Option<&[u8]>,
    deadline: Duration,
    maximum_bytes_per_stream: usize,
) -> io::Result<Output> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = spawn(command)?;
    let stdin = child.stdin().take();
    let stdout = child
        .stdout()
        .take()
        .ok_or_else(|| io::Error::other("managed child stdout is unavailable"))?;
    let stderr = child
        .stderr()
        .take()
        .ok_or_else(|| io::Error::other("managed child stderr is unavailable"))?;

    // Keep all pipe futures scoped to this call. Cancellation drops them and
    // the managed child together instead of detaching reader tasks.
    let completion = async {
        let (wait, write) = tokio::join!(
            async {
                let status = child.wait().await?;
                // A descendant can retain stdin/out/err after its parent exits.
                // Kill the tree before waiting for the writer or EOF readers.
                child.start_kill()?;
                Ok::<_, io::Error>(status)
            },
            async {
                if let Some(input) = input {
                    let mut stdin = stdin
                        .ok_or_else(|| io::Error::other("managed child stdin is unavailable"))?;
                    stdin.write_all(input).await?;
                    stdin.shutdown().await?;
                }
                Ok::<_, io::Error>(())
            }
        );
        let status = wait?;
        // Failed children can close stdin early. Preserve their actual status
        // and diagnostics; a successful child must have accepted the input.
        if status.success() {
            write?;
        }
        Ok::<_, io::Error>(status)
    };
    let collected = tokio::time::timeout(deadline, async {
        tokio::try_join!(
            completion,
            read_bounded(stdout, maximum_bytes_per_stream),
            read_bounded(stderr, maximum_bytes_per_stream)
        )
    })
    .await;
    let (status, (stdout, stdout_exceeded), (stderr, stderr_exceeded)) = match collected {
        Ok(Ok(result)) => result,
        result => {
            let error = match result {
                Ok(Err(error)) => error,
                Err(error) => io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("managed process timed out: {error}"),
                ),
                Ok(Ok(_)) => unreachable!(),
            };
            if let Err(termination) = child.terminate(Duration::from_millis(100)).await {
                return Err(io::Error::new(
                    error.kind(),
                    format!(
                        "{error}; additionally, failed to terminate process tree: {termination}"
                    ),
                ));
            }
            return Err(error);
        }
    };

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
    initial_id: Option<u32>,
    tree_killed: bool,
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
        if self.tree_killed {
            return Ok(());
        }
        match self.inner.start_kill() {
            Ok(()) => {
                self.tree_killed = true;
                Ok(())
            }
            Err(error)
                if process_tree_is_finished(&error, self.initial_id)
                    && matches!(self.inner.try_wait(), Ok(Some(_))) =>
            {
                // Killing an already-reaped process tree is idempotent.
                self.tree_killed = true;
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// Give the process tree a short graceful window, then force-kill and
    /// reap it. The deadline prevents shutdown from waiting forever.
    pub async fn terminate(&mut self, grace: Duration) -> io::Result<ExitStatus> {
        #[cfg(unix)]
        let graceful_signal_error = self.inner.signal(libc::SIGTERM).err();
        #[cfg(not(unix))]
        let graceful_signal_error = self.start_kill().err();

        match tokio::time::timeout(grace, self.inner.wait()).await {
            Ok(status) => {
                let status = status.map_err(|wait_error| {
                    if let Some(signal_error) = graceful_signal_error {
                        io::Error::other(format!(
                            "failed to wait for the process after graceful termination failed: {wait_error}; additionally, failed to signal the process tree: {signal_error}"
                        ))
                    } else {
                        wait_error
                    }
                })?;
                // The direct child may exit on SIGTERM while a descendant in
                // the same group ignores it. A final group/job kill closes
                // that race; an already-empty group simply returns ESRCH.
                self.start_kill().map_err(|error| {
                    io::Error::new(
                        error.kind(),
                        format!("cleanup after graceful leader exit failed: {error}"),
                    )
                })?;
                Ok(status)
            }
            Err(timeout_error) => {
                self.start_kill().map_err(|kill_error| {
                    let mut diagnostic = format!(
                        "failed to force-kill the process tree after the graceful termination window expired: {kill_error}; termination timeout: {timeout_error}"
                    );
                    if let Some(signal_error) = graceful_signal_error {
                        diagnostic.push_str(&format!(
                            "; additionally, failed to send the graceful termination signal: {signal_error}"
                        ));
                    }
                    io::Error::other(diagnostic)
                })?;
                tokio::time::timeout(Duration::from_secs(2), self.inner.wait()).await
                    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "process tree was killed but reaping exceeded two seconds"))?
                    .map_err(|wait_error| {
                    io::Error::other(format!(
                        "failed to reap the force-killed process tree: {wait_error}; graceful termination timeout: {timeout_error}"
                    ))
                })
            }
        }
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        // The wrapper translates this to killpg(2) or TerminateJobObject,
        // unlike Tokio's raw Child kill-on-drop which only targets one PID.
        if let Err(error) = self.start_kill()
            && !process_tree_is_finished(&error, self.initial_id)
        {
            tracing::error!(
                error = %error,
                "failed to kill a managed process tree while dropping its handle"
            );
        }
    }
}
