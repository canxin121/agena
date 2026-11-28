use std::{future::Future, path::PathBuf, sync::Arc, time::Duration};

use crate::{
    TaskControl, WatchPathSet, capture_watch_path_stamps, diff_watch_path_stamps,
    wait_for_tick_or_shutdown,
};

/// Run the generic watched-path polling loop used by runtime composition.
///
/// The concrete reload operation stays with the composition owner; this
/// function owns only path snapshots, change detection, polling, and
/// cancellation through [`TaskControl`]. The callback reports whether a new
/// task accepted the paths so coalescing with an older reload can be retried.
pub async fn run_reload_watch_loop<I, P, R, Fut>(
    task_control: Arc<TaskControl>,
    initial_paths: WatchPathSet,
    interval: I,
    paths: P,
    mut reload: R,
) where
    I: Fn() -> Duration + Send + 'static,
    P: Fn() -> WatchPathSet + Send + 'static,
    R: FnMut(Vec<PathBuf>) -> Fut + Send + 'static,
    Fut: Future<Output = Result<bool, String>> + Send,
{
    let Some(mut known_stamps) = capture_stamps(&task_control, initial_paths).await else {
        return;
    };

    loop {
        if wait_for_tick_or_shutdown(&task_control, interval()).await {
            break;
        }

        let observed_paths = paths();
        let Some(observed) = capture_stamps(&task_control, observed_paths).await else {
            break;
        };
        let changed_paths = diff_watch_path_stamps(&known_stamps, &observed);
        if changed_paths.is_empty() {
            continue;
        }
        let count = changed_paths.len();
        match reload(changed_paths).await {
            Ok(true) => known_stamps = observed,
            Ok(false) => {}
            Err(error) => tracing::warn!(
                error = %error,
                changed_paths = count,
                "could not schedule runtime reload"
            ),
        }
    }
}

async fn capture_stamps(
    control: &TaskControl,
    paths: WatchPathSet,
) -> Option<std::collections::HashMap<PathBuf, crate::watch::WatchPathStamp>> {
    tokio::select! {
        biased;
        _ = control.cancelled() => None,
        result = crate::blocking::FILE_OPERATIONS.run(move || capture_watch_path_stamps(paths.as_slice())) => {
            match result {
                Ok(stamps) => Some(stamps),
                Err(error) => {
                    tracing::error!(%error, "runtime path watch worker failed");
                    None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::{Notify, mpsc};

    #[tokio::test]
    async fn coalesced_reload_keeps_watched_paths_until_a_task_accepts_them() {
        let path =
            std::env::temp_dir().join(format!("agena-config-watch-{}", uuid::Uuid::new_v4()));
        let paths = WatchPathSet::from_paths(vec![path.clone()]);
        let control = Arc::new(TaskControl::default());
        let sleeping = Arc::new(Notify::new());
        let interval = {
            let sleeping = sleeping.clone();
            move || {
                sleeping.notify_one();
                Duration::from_secs(60)
            }
        };
        let (submitted, mut submissions) = mpsc::unbounded_channel();
        let mut coalesce = true;
        let watcher = tokio::spawn(run_reload_watch_loop(
            control.clone(),
            paths.clone(),
            interval,
            move || paths.clone(),
            move |paths| {
                submitted.send(paths).unwrap();
                std::future::ready(Ok(!std::mem::take(&mut coalesce)))
            },
        ));
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            sleeping.notified().await;
            tokio::fs::write(&path, b"{}").await.unwrap();
            for _ in 0..2 {
                control.notify().notify_one();
                assert_eq!(
                    submissions.recv().await.as_deref(),
                    Some(std::slice::from_ref(&path))
                );
                sleeping.notified().await;
            }
            control.notify().notify_one();
            sleeping.notified().await;
            assert!(matches!(
                submissions.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
            ));
        })
        .await;
        control.shutdown();
        watcher.await.unwrap();
        let _ = tokio::fs::remove_file(path).await;
        result.unwrap();
    }
}
