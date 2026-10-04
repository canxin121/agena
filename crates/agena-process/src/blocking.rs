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

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn zombie_group_cleanup_preserves_real_permission_failures() {
        let mut command = Command::new("/bin/sleep");
        command.arg("30");
        let mut child = spawn(command).unwrap();
        let group = child.initial_id.unwrap() as libc::pid_t;
        let permission_error = io::Error::from_raw_os_error(libc::EPERM);
        // A live member must never turn a genuine EPERM into success.
        assert!(!super::super::process_tree_is_finished(
            &permission_error,
            child.initial_id
        ));
        assert!(!super::super::process_tree_is_finished(
            &permission_error,
            None
        ));

        // Deliberately bypass ManagedChild's idempotence flag to exercise the
        // OS condition: leave the directly owned group leader unreaped.
        child.inner.start_kill().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !crate::unix::zombie_only_group(group).unwrap() {
            assert!(Instant::now() < deadline, "leader must become a zombie");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(super::super::process_tree_is_finished(
            &permission_error,
            child.initial_id
        ));
        child
            .start_kill()
            .expect("all-zombie group is already dead");
        let status = child.terminate(Duration::from_secs(2)).unwrap();
        assert!(!status.success());
        child.start_kill().expect("repeated cleanup is idempotent");
    }

    #[test]
    fn observing_an_exited_leader_cleans_up_descendants() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pid");
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                "sleep 30 & printf '%s' $! > \"$1\"; exit 0",
                "fixture",
            ])
            .arg(&path);
        let mut child = spawn(command).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        let pid: i32 = std::fs::read_to_string(path).unwrap().parse().unwrap();
        loop {
            // SAFETY: signal 0 only checks the positive fixture PID.
            if unsafe { libc::kill(pid, 0) } == -1
                && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
            {
                break;
            }
            assert!(Instant::now() < deadline, "descendant must exit");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            child
                .terminate(Duration::from_millis(100))
                .unwrap()
                .success()
        );
    }
}
