use std::{future::Future, path::PathBuf, sync::Arc, time::Duration};

use crate::{
    TaskControl, WatchPathSet, capture_watch_path_stamps, diff_watch_path_stamps,
    wait_for_tick_or_shutdown,
};

/// Run the generic watched-path polling loop used by runtime composition.
///
/// The concrete reload operation stays with the composition owner; this
/// function owns only path snapshots, change detection, polling, and
/// cancellation through [`TaskControl`].
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
    Fut: Future<Output = Result<(), String>> + Send,
{
    let mut known_stamps = capture_watch_path_stamps(initial_paths.as_slice());

    loop {
        if wait_for_tick_or_shutdown(&task_control, interval()).await {
            break;
        }

        let observed_paths = paths();
        let observed = capture_watch_path_stamps(observed_paths.as_slice());
        let changed_paths = diff_watch_path_stamps(&known_stamps, &observed);
        if !changed_paths.is_empty()
            && let Err(error) = reload(changed_paths.clone()).await
        {
            tracing::warn!(
                error = %error,
                changed_paths = changed_paths.len(),
                "runtime reload failed"
            );
        }
        known_stamps = observed;
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use super::run_reload_watch_loop;
    use crate::{TaskControl, WatchPathSet};

    #[tokio::test]
    async fn exits_without_polling_after_shutdown() {
        let control = Arc::new(TaskControl::default());
        control.shutdown();
        let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_reload = Arc::clone(&called);
        run_reload_watch_loop(
            control,
            WatchPathSet::default(),
            || Duration::from_millis(1),
            WatchPathSet::default,
            move |_| {
                called_reload.store(true, std::sync::atomic::Ordering::SeqCst);
                async { Ok::<_, String>(()) }
            },
        )
        .await;
        assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
    }
}
