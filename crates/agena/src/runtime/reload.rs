use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    time::{Duration, SystemTime},
};

use chrono::{DateTime, Utc};

use super::{RuntimeBackgroundTaskOrigin, builder::AgenaRuntime};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeReloadCause {
    Manual,
    WatchedPathsChanged { paths: Vec<PathBuf> },
}

#[derive(Debug, Clone)]
pub struct RuntimeReloadReport {
    pub cause: RuntimeReloadCause,
    pub previous_generation: u64,
    pub generation: u64,
    pub loaded_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PathStamp {
    exists: bool,
    modified: Option<SystemTime>,
}

pub(crate) async fn run(runtime: AgenaRuntime) {
    let mut known_stamps = capture_path_stamps(runtime.current_snapshot().watch_paths());

    loop {
        let snapshot = runtime.current_snapshot();
        let interval = snapshot.reload_poll_interval();
        if wait_for_tick_or_shutdown(&runtime, interval).await {
            break;
        }

        let snapshot = runtime.current_snapshot();
        let observed = capture_path_stamps(snapshot.watch_paths());
        let changed_paths = diff_changed_paths(&known_stamps, &observed);

        if snapshot.reload_enabled()
            && !changed_paths.is_empty()
            && let Err(err) = runtime.start_runtime_reload_task(
                RuntimeReloadCause::WatchedPathsChanged {
                    paths: changed_paths.clone(),
                },
                RuntimeBackgroundTaskOrigin::System,
            )
        {
            tracing::warn!(error = %err, changed_paths = changed_paths.len(), "runtime reload failed");
        }

        known_stamps = observed;
    }
}

async fn wait_for_tick_or_shutdown(runtime: &AgenaRuntime, interval: Duration) -> bool {
    if runtime.is_shutdown() {
        return true;
    }

    tokio::select! {
        _ = tokio::time::sleep(interval) => {}
        _ = runtime.task_control().notify().notified() => {}
    }

    runtime.is_shutdown()
}

fn capture_path_stamps(paths: &[PathBuf]) -> HashMap<PathBuf, PathStamp> {
    paths
        .iter()
        .cloned()
        .map(|path| {
            let stamp = match fs::metadata(&path) {
                Ok(metadata) => PathStamp {
                    exists: true,
                    modified: metadata.modified().ok(),
                },
                Err(_) => PathStamp {
                    exists: false,
                    modified: None,
                },
            };
            (path, stamp)
        })
        .collect()
}

fn diff_changed_paths(
    previous: &HashMap<PathBuf, PathStamp>,
    current: &HashMap<PathBuf, PathStamp>,
) -> Vec<PathBuf> {
    let mut changed = current
        .iter()
        .filter_map(|(path, stamp)| (previous.get(path) != Some(stamp)).then_some(path.clone()))
        .collect::<Vec<_>>();

    changed.extend(
        previous
            .keys()
            .filter(|path| !current.contains_key(*path))
            .cloned(),
    );
    changed.sort();
    changed.dedup();
    changed
}
