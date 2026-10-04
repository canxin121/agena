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

#[cfg(windows)]
use process_wrap::tokio::JobObject;
#[cfg(unix)]
use process_wrap::tokio::ProcessSession;
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout, Command};

fn process_tree_is_absent(error: &io::Error) -> bool {
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
        match self.inner.start_kill() {
            Ok(()) => Ok(()),
            Err(error)
                if process_tree_is_absent(&error)
                    && matches!(self.inner.try_wait(), Ok(Some(_))) =>
            {
                // Killing an already-reaped process tree is idempotent.
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
                self.start_kill()?;
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
                self.inner.wait().await.map_err(|wait_error| {
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
        if let Err(error) = self.inner.start_kill()
            && !process_tree_is_absent(&error)
        {
            tracing::error!(
                error = %error,
                "failed to kill a managed process tree while dropping its handle"
            );
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::{output, output_with_input, spawn};
    use std::io;
    use std::process::Stdio;
    use std::time::Duration;
    use tokio::process::Command;

    #[tokio::test]
    async fn structured_input_drains_large_unicode_output_concurrently() {
        let input = "浏览器表单😀\n".repeat(100_000).into_bytes();
        let result = output_with_input(
            Command::new("cat"),
            &input,
            Duration::from_secs(5),
            input.len(),
        )
        .await
        .expect("echo input larger than both pipes");
        assert!(result.status.success());
        assert_eq!(result.stdout, input);
        assert!(result.stderr.is_empty());
    }

    #[tokio::test]
    async fn failed_child_retains_status_and_diagnostics_with_unconsumed_input() {
        let mut command = Command::new("sh");
        // This descendant keeps stdin open even after its parent exits.
        command.args(["-c", "sleep 30 <&0 & printf failure >&2; exit 7"]);
        let result = output_with_input(
            command,
            &vec![b'x'; 1024 * 1024],
            Duration::from_secs(5),
            1024,
        )
        .await
        .expect("failed child should not turn into a pipe timeout");
        assert_eq!(result.status.code(), Some(7));
        assert_eq!(result.stderr, b"failure");
    }

    async fn assert_process_gone(pid: i32) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                // SAFETY: signal 0 only checks the positive fixture PID.
                if unsafe { libc::kill(pid, 0) } == -1
                    && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("fixture process must be reaped");
    }

    #[tokio::test]
    async fn blocked_input_timeout_and_cancellation_clean_up_the_tree() {
        for cancel in [false, true] {
            let fixture = tempfile::tempdir().unwrap();
            let pid_file = fixture.path().join("child.pid");
            let mut command = Command::new("sh");
            command.args([
                "-c",
                "sleep 30 <&0 & printf '%s' $! > \"$1\"; wait",
                "fixture",
            ]);
            command.arg(&pid_file);
            let task = tokio::spawn(async move {
                output_with_input(
                    command,
                    &vec![b'x'; 1024 * 1024],
                    Duration::from_millis(500),
                    1024,
                )
                .await
            });
            let pid = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if let Ok(contents) = std::fs::read_to_string(&pid_file)
                        && let Ok(pid) = contents.parse::<i32>()
                    {
                        break pid;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .expect("fixture must start");
            if cancel {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            } else {
                assert_eq!(
                    task.await.unwrap().unwrap_err().kind(),
                    io::ErrorKind::TimedOut
                );
            }
            assert_process_gone(pid).await;
        }
    }

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
