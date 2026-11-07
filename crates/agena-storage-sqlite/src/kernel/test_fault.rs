//! Debug-only deterministic faults in real SQLite transaction and file I/O flow.
//! Each test opts in with a control file under its isolated data root. Release
//! builds never read the environment variable or install the VFS wrapper.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

#[cfg(debug_assertions)]
mod vfs;

#[cfg(debug_assertions)]
const ENV: &str = "AGENA_TEST_KERNEL_FAULT_FILE";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    Fail,
    Crash,
    Hold,
    DiskFull,
    IoError,
}

fn control_path() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    {
        std::env::var_os(ENV).map(PathBuf::from)
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

#[cfg(debug_assertions)]
pub(crate) fn configure_sqlite(
    options: sea_orm::sqlx::sqlite::SqliteConnectOptions,
) -> std::result::Result<sea_orm::sqlx::sqlite::SqliteConnectOptions, String> {
    match control_path() {
        Some(path) => Ok(options.vfs(vfs::register(path)?)),
        None => Ok(options),
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Default)]
struct Control {
    point: String,
    effect: String,
    remaining: u32,
    hits: u32,
    released: bool,
    #[serde(default)]
    arm_id: String,
    #[serde(default)]
    sqlite_code: Option<i32>,
    #[serde(default)]
    file: Option<String>,
}

fn publish(path: &Path, state: &Control) -> std::result::Result<(), String> {
    let bytes = serde_json::to_vec(state).map_err(|error| error.to_string())?;
    let temp = path.with_extension(format!("{}.kernel-swap", uuid::Uuid::new_v4()));
    std::fs::write(&temp, bytes)
        .map_err(|error| format!("record fault hit {}: {error}", temp.display()))?;
    if let Err(error) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("publish fault hit {}: {error}", path.display()));
    }
    Ok(())
}

/// Lock a stable sidecar, never the atomically replaced control-file inode.
/// The test driver uses the same lock across its read-modify-write operations.
fn control_lock(path: &Path) -> std::result::Result<std::fs::File, String> {
    let lock_path = path.with_extension("fault-lock");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| format!("open fault lock {}: {error}", lock_path.display()))?;
    file.lock()
        .map_err(|error| format!("acquire fault lock {}: {error}", lock_path.display()))?;
    Ok(file)
}

/// Serialize both pooled consumers and the external controller so one arm is
/// consumed exactly once and a stale hit cannot overwrite a newly armed fault.
fn take(
    path: &Path,
    point: &str,
    file: Option<&str>,
) -> std::result::Result<Option<(Effect, Control)>, String> {
    let _guard = control_lock(path)?;
    let raw = std::fs::read(path)
        .map_err(|error| format!("fault fixture {} is unreadable: {error}", path.display()))?;
    let mut state: Control = serde_json::from_slice(&raw)
        .map_err(|error| format!("fault fixture {} is malformed: {error}", path.display()))?;
    if state.point != point || state.remaining == 0 {
        return Ok(None);
    }
    let effect = match state.effect.as_str() {
        "fail" => Effect::Fail,
        "crash" => Effect::Crash,
        "hold" => Effect::Hold,
        "disk_full" => Effect::DiskFull,
        "io_error" => Effect::IoError,
        other => return Err(format!("unknown fault effect {other:?}")),
    };
    state.sqlite_code = match effect {
        Effect::DiskFull if point == "sqlite.write" => Some(libsqlite3_sys::SQLITE_FULL),
        Effect::IoError => Some(match point {
            "sqlite.read" => libsqlite3_sys::SQLITE_IOERR_READ,
            "sqlite.write" => libsqlite3_sys::SQLITE_IOERR_WRITE,
            "sqlite.sync" => libsqlite3_sys::SQLITE_IOERR_FSYNC,
            "sqlite.truncate" => libsqlite3_sys::SQLITE_IOERR_TRUNCATE,
            "sqlite.open" => libsqlite3_sys::SQLITE_CANTOPEN,
            _ => return Err(format!("invalid SQLite fault point {point:?}")),
        }),
        Effect::Fail | Effect::Hold | Effect::Crash if point == "kernel.before_commit" => None,
        _ => {
            return Err(format!(
                "invalid fault point/effect pair: {point:?} / {effect:?}"
            ));
        }
    };
    state.remaining -= 1;
    state.hits += 1;
    state.file = file.map(str::to_owned);
    publish(path, &state)?;
    Ok(Some((effect, state)))
}

/// Runs after all batch commands execute, while the real SQL transaction is
/// still uncommitted. The writer owns this gate; readers see pre-commit data.
pub(crate) async fn before_commit() -> Option<Effect> {
    let path = control_path()?;
    let (effect, hit) =
        take(&path, "kernel.before_commit", None).unwrap_or_else(|error| panic!("{error}"))?;
    match effect {
        Effect::Hold => wait_release(&path, &hit)
            .await
            .unwrap_or_else(|error| panic!("{error}")),
        Effect::Crash => std::process::abort(),
        Effect::Fail => {}
        Effect::DiskFull | Effect::IoError => unreachable!("I/O effects are confined to the VFS"),
    }
    Some(effect)
}

async fn wait_release(path: &Path, hit: &Control) -> std::result::Result<(), String> {
    let gate = async {
        loop {
            let raw = tokio::fs::read(path)
                .await
                .map_err(|error| format!("read held fault {}: {error}", path.display()))?;
            let state: Control = serde_json::from_slice(&raw)
                .map_err(|error| format!("decode held fault {}: {error}", path.display()))?;
            // The driver permits replacing a hit Hold only after release/clear.
            // A newer identity therefore also releases this older transaction;
            // it must never be consumed or overwritten by this old waiter.
            if state.arm_id != hit.arm_id || state.released {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(20), gate)
        .await
        .map_err(|_| {
            "kernel.before_commit fault remained held for more than 20 seconds".to_owned()
        })?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_shot_fault_arm_is_consumed_exactly_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fault.json");
        publish(
            &path,
            &Control {
                point: "kernel.before_commit".into(),
                effect: "fail".into(),
                remaining: 1,
                ..Control::default()
            },
        )
        .unwrap();
        assert!(take(&path, "wrong.point", None).unwrap().is_none());
        assert_eq!(
            take(&path, "kernel.before_commit", None)
                .unwrap()
                .unwrap()
                .0,
            Effect::Fail
        );
        assert!(take(&path, "kernel.before_commit", None).unwrap().is_none());
        let state: Control = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!((state.hits, state.remaining), (1, 0));
    }

    #[test]
    fn concurrent_io_consumers_cannot_use_the_same_arm_twice() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fault.json");
        publish(
            &path,
            &Control {
                point: "sqlite.write".into(),
                effect: "disk_full".into(),
                remaining: 1,
                ..Control::default()
            },
        )
        .unwrap();
        let count: usize = std::thread::scope(|scope| {
            (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        take(&path, "sqlite.write", Some("owned.sqlite-wal"))
                            .unwrap()
                            .is_some() as usize
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .sum()
        });
        assert_eq!(count, 1);
        let state: Control = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(
            (state.hits, state.remaining, state.sqlite_code),
            (1, 0, Some(libsqlite3_sys::SQLITE_FULL))
        );
    }

    #[tokio::test]
    async fn released_hold_may_be_rearmed_before_the_writer_polls_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fault.json");
        let old_hit = Control {
            point: "kernel.before_commit".into(),
            effect: "hold".into(),
            hits: 1,
            arm_id: "old-hold".into(),
            ..Control::default()
        };
        // The writer has not polled between release() and the next arm().
        let new_arm = Control {
            point: "kernel.before_commit".into(),
            effect: "hold".into(),
            remaining: 1,
            arm_id: "new-hold".into(),
            ..Control::default()
        };
        publish(&path, &new_arm).unwrap();
        wait_release(&path, &old_hit).await.unwrap();
        let state: Control = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(state.arm_id, "new-hold");
        assert_eq!((state.remaining, state.hits, state.released), (1, 0, false));
    }
}
