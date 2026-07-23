use agena_runtime::{RuntimeBackgroundTaskOrigin, RuntimeReloadCause};

use super::builder::AgenaRuntime;

pub(crate) async fn run(runtime: AgenaRuntime) {
    let initial_paths =
        agena_runtime::WatchPathSet::from_paths(runtime.current_snapshot().watch_paths().to_vec());
    let interval_runtime = runtime.clone();
    let paths_runtime = runtime.clone();
    agena_runtime::run_reload_watch_loop(
        runtime.task_control_handle(),
        initial_paths,
        move || interval_runtime.current_snapshot().reload_poll_interval(),
        move || {
            agena_runtime::WatchPathSet::from_paths(
                paths_runtime.current_snapshot().watch_paths().to_vec(),
            )
        },
        move |paths| {
            let runtime = runtime.clone();
            async move {
                runtime
                    .start_runtime_reload_task(
                        RuntimeReloadCause::WatchedPathsChanged { paths },
                        RuntimeBackgroundTaskOrigin::System,
                    )
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            }
        },
    )
    .await;
}
