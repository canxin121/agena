//! One-shot faults in a case-owned debug Agena process.
//!
//! Transaction gates run inside the real uncommitted kernel batch. SQLite I/O
//! faults run at its VFS boundary and return real SQLite FULL/IOERR result codes;
//! they neither fill the host disk nor replace unrelated operating-system calls.

mod lifecycle;

use std::{fmt, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

pub const KERNEL_BEFORE_COMMIT: &str = "kernel.before_commit";

/// A database file operation at SQLite's VFS (operating-system) boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaultSyscall {
    Read,
    Write,
    Sync,
    Truncate,
    Open,
}

impl FaultSyscall {
    pub const fn point(self) -> &'static str {
        match self {
            Self::Read => "sqlite.read",
            Self::Write => "sqlite.write",
            Self::Sync => "sqlite.sync",
            Self::Truncate => "sqlite.truncate",
            Self::Open => "sqlite.open",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaultEffect {
    Fail,
    Crash,
    Hold,
    /// Return SQLITE_FULL from the next selected database/WAL write.
    DiskFull,
    /// Return the selected VFS operation's extended SQLite I/O error.
    IoError,
}

impl FaultEffect {
    fn as_str(self) -> &'static str {
        match self {
            Self::Fail => "fail",
            Self::Crash => "crash",
            Self::Hold => "hold",
            Self::DiskFull => "disk_full",
            Self::IoError => "io_error",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FaultState {
    pub point: String,
    pub effect: String,
    pub remaining: u32,
    pub hits: u32,
    pub released: bool,
    /// Identifies this arm, so a waiter cannot consume a replacement arm's hit.
    #[serde(default)]
    pub arm_id: String,
    /// Actual SQLite result code returned by an I/O fault, including subtype.
    #[serde(default)]
    pub sqlite_code: Option<i32>,
    /// Database, WAL or journal file on which the VFS operation failed.
    #[serde(default)]
    pub file: Option<String>,
}

#[derive(Clone)]
pub struct FaultDriver {
    path: PathBuf,
    gate: Arc<Mutex<()>>,
    owner: Option<crate::case::TestCase>,
}

impl fmt::Debug for FaultDriver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FaultDriver")
            .field("path", &self.path)
            .field("case_owned", &self.owner.is_some())
            .finish_non_exhaustive()
    }
}

impl FaultDriver {
    pub(crate) fn new(path: PathBuf, gate: Arc<Mutex<()>>) -> Self {
        Self {
            path,
            gate,
            owner: None,
        }
    }

    /// Arm one supported point/effect pair. An outstanding arm must be cleared
    /// first; a hit Hold must be released before another gate can be armed.
    pub async fn arm(&self, point: &str, effect: FaultEffect) -> Result<()> {
        let supported = match effect {
            FaultEffect::Fail | FaultEffect::Crash | FaultEffect::Hold => {
                point == KERNEL_BEFORE_COMMIT
            }
            FaultEffect::DiskFull => point == FaultSyscall::Write.point(),
            FaultEffect::IoError => [
                FaultSyscall::Read,
                FaultSyscall::Write,
                FaultSyscall::Sync,
                FaultSyscall::Truncate,
                FaultSyscall::Open,
            ]
            .into_iter()
            .any(|operation| operation.point() == point),
        };
        ensure!(
            supported,
            "unsupported fault point/effect pair: {point:?} / {effect:?}"
        );
        let _guard = self.gate.lock().await;
        let _file_guard = self.control_lock().await?;
        if tokio::fs::try_exists(&self.path).await? {
            let previous = self.state().await?;
            ensure!(
                previous.remaining == 0,
                "a fault is already armed; clear or observe it before rearming"
            );
            ensure!(
                previous.effect != "hold" || previous.hits == 0 || previous.released,
                "release or clear the hit Hold before arming another fault"
            );
        }
        self.write(&FaultState {
            point: point.to_owned(),
            effect: effect.as_str().to_owned(),
            remaining: 1,
            hits: 0,
            released: false,
            arm_id: uuid::Uuid::new_v4().to_string(),
            sqlite_code: None,
            file: None,
        })
        .await
    }

    /// Simulate ENOSPC at the next actual SQLite database/WAL write. SQLite
    /// receives SQLITE_FULL and runs its normal error and rollback machinery.
    pub async fn disk_full(&self) -> Result<()> {
        self.arm(FaultSyscall::Write.point(), FaultEffect::DiskFull)
            .await
    }

    /// Fail one selected SQLite file operation, without altering host syscalls.
    pub async fn fail_syscall(&self, operation: FaultSyscall) -> Result<()> {
        self.arm(operation.point(), FaultEffect::IoError).await
    }

    pub async fn state(&self) -> Result<FaultState> {
        let data = tokio::fs::read(&self.path)
            .await
            .context("read owned fault state")?;
        serde_json::from_slice(&data).context("decode owned fault state")
    }

    /// Wait on real time for the current arm to be consumed by production code.
    /// This observation deadline stays independent of a frozen product clock.
    pub async fn wait_hit(&self) -> Result<FaultState> {
        self.wait_hit_within(Duration::from_secs(8)).await
    }

    pub async fn wait_hit_within(&self, timeout: Duration) -> Result<FaultState> {
        let armed = self.state().await?;
        ensure!(!armed.point.is_empty(), "no fault is armed");
        self.wait_for_arm(armed, timeout).await
    }

    async fn wait_for_arm(&self, armed: FaultState, timeout: Duration) -> Result<FaultState> {
        tokio::time::timeout(timeout, async {
            loop {
                let state = self.state().await?;
                ensure!(
                    state.arm_id == armed.arm_id && state.point == armed.point,
                    "fault arm was cleared or replaced while waiting for its hit"
                );
                if state.hits > 0 {
                    ensure!(
                        state.remaining == 0 && state.hits == 1,
                        "fault hit did not consume exactly one one-shot arm"
                    );
                    return Ok(state);
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .context("fault was not reached by the production operation")?
    }

    /// Release the held gate. Await the original request separately to observe
    /// its transaction outcome; release does not promise that commit has finished.
    pub async fn release(&self) -> Result<()> {
        let _guard = self.gate.lock().await;
        let _file_guard = self.control_lock().await?;
        let mut state = self.state().await?;
        ensure!(
            state.point == KERNEL_BEFORE_COMMIT
                && state.effect == "hold"
                && state.hits == 1
                && state.remaining == 0,
            "release requires a previously hit Hold gate"
        );
        state.released = true;
        self.write(&state).await
    }

    /// Disarm a pending fault, or release and clear a hit Hold. A held operation
    /// may still be completing: await its request to observe the final outcome.
    pub async fn clear(&self) -> Result<()> {
        let _guard = self.gate.lock().await;
        let _file_guard = self.control_lock().await?;
        let arm_id = if tokio::fs::try_exists(&self.path).await? {
            self.state().await?.arm_id
        } else {
            String::new()
        };
        self.write(&FaultState {
            point: String::new(),
            effect: "fail".into(),
            remaining: 0,
            hits: 0,
            released: true,
            arm_id,
            sqlite_code: None,
            file: None,
        })
        .await
    }

    /// Match the product's stable sidecar lock. Polling try_lock keeps a
    /// contended external-process lock from blocking a Tokio runtime worker.
    async fn control_lock(&self) -> Result<std::fs::File> {
        let lock_path = self.path.with_extension("fault-lock");
        let lock = tokio::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .await
            .context("open owned fault lock")?
            .into_std()
            .await;
        tokio::time::timeout(Duration::from_secs(8), async move {
            loop {
                match lock.try_lock() {
                    Ok(()) => return Ok(lock),
                    Err(std::fs::TryLockError::WouldBlock) => {
                        tokio::time::sleep(Duration::from_millis(1)).await
                    }
                    Err(std::fs::TryLockError::Error(error)) => {
                        return Err(error).context("acquire owned fault lock");
                    }
                }
            }
        })
        .await
        .context("timed out waiting for owned fault control lock")?
    }

    async fn write(&self, state: &FaultState) -> Result<()> {
        let tmp = self
            .path
            .with_extension(format!("{}.swap", uuid::Uuid::new_v4()));
        tokio::fs::write(&tmp, serde_json::to_vec(state)?).await?;
        if let Err(error) = tokio::fs::rename(&tmp, &self.path).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(error).context("publish fault control");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn independent_driver_handles_share_the_product_sidecar_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fault.json");
        let first = FaultDriver::new(path.clone(), Arc::new(Mutex::new(())));
        let second = FaultDriver::new(path, Arc::new(Mutex::new(())));
        let held = first.control_lock().await.unwrap();
        let acquiring = second.control_lock();
        tokio::pin!(acquiring);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut acquiring)
                .await
                .is_err()
        );
        drop(held);
        let acquired = tokio::time::timeout(Duration::from_secs(1), acquiring)
            .await
            .unwrap()
            .unwrap();
        drop(acquired);
        second.disk_full().await.unwrap();
        assert!(
            first
                .arm(KERNEL_BEFORE_COMMIT, FaultEffect::Hold)
                .await
                .is_err()
        );
        assert_eq!(
            first.state().await.unwrap().point,
            FaultSyscall::Write.point()
        );
    }

    #[tokio::test]
    async fn fault_controller_rejects_unknown_points_and_can_reset_armed_state() {
        let dir = tempfile::tempdir().unwrap();
        let driver = FaultDriver::new(dir.path().join("fault.json"), Arc::new(Mutex::new(())));
        assert!(driver.arm("not-a-point", FaultEffect::Fail).await.is_err());
        assert!(
            driver
                .arm(KERNEL_BEFORE_COMMIT, FaultEffect::DiskFull)
                .await
                .is_err()
        );
        assert!(driver.arm("sqlite.sync", FaultEffect::Hold).await.is_err());
        driver
            .arm(KERNEL_BEFORE_COMMIT, FaultEffect::Hold)
            .await
            .unwrap();
        let state = driver.state().await.unwrap();
        assert_eq!((state.hits, state.remaining), (0, 1));
        assert!(!state.arm_id.is_empty());
        assert!(driver.disk_full().await.is_err());
        assert!(driver.release().await.is_err());
        driver.clear().await.unwrap();
        assert!(driver.state().await.unwrap().point.is_empty());
        assert!(driver.wait_hit().await.is_err());
        driver.disk_full().await.unwrap();
        let next = driver.state().await.unwrap();
        assert_ne!(next.arm_id, state.arm_id);
        assert_eq!(
            (next.point.as_str(), next.effect.as_str()),
            ("sqlite.write", "disk_full")
        );
    }

    #[tokio::test]
    async fn observation_rejects_replacement_arm_instead_of_accepting_its_hit() {
        let dir = tempfile::tempdir().unwrap();
        let driver = FaultDriver::new(dir.path().join("fault.json"), Arc::new(Mutex::new(())));
        driver.fail_syscall(FaultSyscall::Sync).await.unwrap();
        let armed = driver.state().await.unwrap();
        driver.clear().await.unwrap();
        driver.fail_syscall(FaultSyscall::Write).await.unwrap();
        let mut replacement = driver.state().await.unwrap();
        replacement.hits = 1;
        replacement.remaining = 0;
        driver.write(&replacement).await.unwrap();
        assert!(
            driver
                .wait_for_arm(armed, Duration::from_secs(1))
                .await
                .unwrap_err()
                .to_string()
                .contains("cleared or replaced")
        );
    }

    #[tokio::test]
    async fn hit_hold_cannot_be_overwritten_before_release() {
        let dir = tempfile::tempdir().unwrap();
        let driver = FaultDriver::new(dir.path().join("fault.json"), Arc::new(Mutex::new(())));
        driver
            .arm(KERNEL_BEFORE_COMMIT, FaultEffect::Hold)
            .await
            .unwrap();
        let mut state = driver.state().await.unwrap();
        state.remaining = 0;
        state.hits = 1;
        driver.write(&state).await.unwrap();
        assert!(driver.fail_syscall(FaultSyscall::Read).await.is_err());
        driver.release().await.unwrap();
        driver.fail_syscall(FaultSyscall::Read).await.unwrap();
    }
}
