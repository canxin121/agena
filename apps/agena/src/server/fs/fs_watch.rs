//! On-demand, non-recursive watches for the directories actually shown by a
//! files pane. The stream owns the watcher; disconnect drops every OS watch.
use std::{
    collections::BTreeSet,
    convert::Infallible,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    extract::Query,
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
};
use notify::{RecursiveMode, Watcher};
use serde::Deserialize;

use super::{
    ApiResult, AppError, ensure_within_base, has_parent_dir_component, resolve_path,
    resolve_project_directory, to_api_path,
};

static WATCH_SETUP: agena_async::BlockingPool = agena_async::BlockingPool::new(4);
// Include watchers waiting for close in admission. A reconnect burst cannot
// accumulate unlimited native resources while the close workers are occupied.
static WATCH_LIFETIMES: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(64);
type WatchClose = (
    notify::RecommendedWatcher,
    tokio::sync::SemaphorePermit<'static>,
);

fn watcher_closer() -> Result<std::sync::mpsc::Sender<WatchClose>, AppError> {
    static CLOSER: std::sync::OnceLock<Result<std::sync::mpsc::Sender<WatchClose>, String>> =
        std::sync::OnceLock::new();
    CLOSER
        .get_or_init(|| {
            let (sender, receiver) = std::sync::mpsc::channel::<WatchClose>();
            let receiver = Arc::new(Mutex::new(receiver));
            for index in 0..2 {
                let receiver = Arc::clone(&receiver);
                std::thread::Builder::new()
                    .name(format!("agena-watch-close-{index}"))
                    .spawn(move || {
                        loop {
                            let next = receiver
                                .lock()
                                .unwrap_or_else(|error| error.into_inner())
                                .recv();
                            let Ok((watcher, permit)) = next else { return };
                            // notify's shutdown may join a native thread. Keep both the
                            // resource and its admission permit on this close thread.
                            drop(watcher);
                            drop(permit);
                        }
                    })
                    .map_err(|error| format!("start filesystem watcher close thread: {error}"))?;
            }
            Ok(sender)
        })
        .as_ref()
        .cloned()
        .map_err(|error| AppError::internal(error.clone()))
}

struct OwnedWatcher {
    resource: Option<WatchClose>,
    closer: std::sync::mpsc::Sender<WatchClose>,
}

impl Drop for OwnedWatcher {
    fn drop(&mut self) {
        let Some(resource) = self.resource.take() else {
            return;
        };
        if let Err(error) = self.closer.send(resource) {
            // Exceptional worker failure: keep the resource off the async
            // thread even then. At most 64 close resources can exist.
            let resource = error.0;
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn_blocking(move || drop(resource));
            } else {
                drop(resource);
            }
        }
    }
}

#[derive(Deserialize)]
pub struct WatchQuery {
    directory: String,
    /// JSON array of expanded directory paths (including selected-file parent).
    paths: Option<String>,
}

#[derive(Default)]
struct Changes {
    paths: BTreeSet<String>,
    truncated: bool,
    reconnect: bool,
}

pub async fn fs_watch(
    headers: HeaderMap,
    Query(query): Query<WatchQuery>,
) -> ApiResult<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>> {
    let base = resolve_project_directory(&headers, Some(&query.directory)).await?;
    let mut paths: Vec<String> = query
        .paths
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|_| AppError::bad_request("paths must be a JSON array of directory paths"))?
        .unwrap_or_default();
    if paths.len() > 128 {
        return Err(AppError::bad_request(
            "At most 128 visible directories may be watched",
        ));
    }
    paths.push(to_api_path(&base));
    let mut watched = BTreeSet::new();
    for path in paths {
        let path = resolve_path(&path);
        if !path.is_absolute() || has_parent_dir_component(&path) {
            return Err(AppError::bad_request(
                "Watch paths must be absolute and contain no parent traversal",
            ));
        }
        ensure_within_base(&base, &path)?;
        watched.insert(path);
    }
    let lifetime = WATCH_LIFETIMES
        .acquire()
        .await
        .expect("private watcher admission is never closed");
    let (watcher, mut events, changes) = WATCH_SETUP
        .run(move || {
            // Initialize native close threads on the setup worker, too.
            let closer = watcher_closer()?;
            let changes = Arc::new(Mutex::new(Changes::default()));
            let (wake, events) = tokio::sync::watch::channel(0_u64);
            let pending = changes.clone();
            let watch_paths = watched.clone();
            // notify may report canonical OS paths even when the UI opened a
            // symlinked workspace. Preserve every watched alias in the events.
            let aliases = watched
                .iter()
                .filter_map(|path| {
                    std::fs::canonicalize(path)
                        .ok()
                        .map(|canonical| (canonical, path.clone()))
                })
                .collect::<Vec<_>>();
            let mut watcher =
                notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    // Path mapping can be expensive during large OS event bursts.
                    // Prepare a bounded batch outside the mutex read by the stream.
                    let mut batch = Changes::default();
                    match event {
                        Ok(event) => {
                            if matches!(event.kind, notify::EventKind::Access(_)) {
                                return;
                            }
                            batch.truncated |= event.need_rescan();
                            // OS watches can be detached by directory deletion or
                            // replacement. Reopen the stream to watch the new inode.
                            let topology_change = matches!(
                                event.kind,
                                notify::EventKind::Create(_)
                                    | notify::EventKind::Remove(_)
                                    | notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
                            );
                            if event.paths.len() > 256 {
                                batch.truncated = true;
                                batch.reconnect |= topology_change;
                            }
                            for path in event.paths.into_iter().take(256) {
                                if batch.paths.len() >= 256 {
                                    batch.truncated = true;
                                    batch.reconnect |= topology_change;
                                    break;
                                }
                                let mut mapped = aliases
                                    .iter()
                                    .filter_map(|(canonical, alias)| {
                                        path.strip_prefix(canonical)
                                            .ok()
                                            .map(|relative| alias.join(relative))
                                    })
                                    .collect::<Vec<_>>();
                                if mapped.is_empty() {
                                    mapped.push(path);
                                }
                                for path in mapped {
                                    batch.reconnect |=
                                        topology_change && watch_paths.contains(&path);
                                    if batch.paths.len() < 256 {
                                        batch.paths.insert(to_api_path(&path));
                                    } else {
                                        batch.truncated = true;
                                    }
                                }
                            }
                        }
                        Err(_) => batch.truncated = true,
                    }
                    let mut pending = pending.lock().expect("filesystem changes lock");
                    pending.truncated |= batch.truncated;
                    pending.reconnect |= batch.reconnect;
                    for path in batch.paths {
                        if pending.paths.len() < 256 {
                            pending.paths.insert(path);
                        } else {
                            pending.truncated = true;
                        }
                    }
                    drop(pending);
                    wake.send_modify(|version| *version = version.wrapping_add(1));
                })
                .map_err(|error| {
                    AppError::internal_error_with_context("start filesystem watcher", &error)
                })?;
            for path in watched {
                // An expanded directory may just have been removed. Its parent watch
                // and the initial reconciliation will remove the stale row.
                if !path.is_dir() {
                    continue;
                }
                watcher
                    .watch(&path, RecursiveMode::NonRecursive)
                    .map_err(|error| {
                        AppError::internal_error_with_context("watch a visible directory", &error)
                    })?;
            }
            Ok::<_, AppError>((
                OwnedWatcher {
                    resource: Some((watcher, lifetime)),
                    closer,
                },
                events,
                changes,
            ))
        })
        .await
        .map_err(|error| {
            AppError::internal_error_with_context("initialize filesystem watches", &error)
        })??;
    let directory = to_api_path(&base);
    let stream = async_stream::stream! {
        let _watcher = watcher;
        yield Ok(Event::default().data("{\"type\":\"fs.ready\"}"));
        while events.changed().await.is_ok() {
            // Fixed trailing window: constant activity never resets the timer.
            tokio::time::sleep(Duration::from_millis(200)).await;
            events.borrow_and_update();
            let batch = std::mem::take(&mut *changes.lock().expect("filesystem changes lock"));
            if batch.paths.is_empty() && !batch.truncated { continue; }
            let payload = serde_json::json!({
                "type": "agena:fs-changed",
                "properties": { "directory": directory, "changeType": "changed",
                    "paths": batch.paths, "truncated": batch.truncated }
            });
            yield Ok(Event::default().data(payload.to_string()));
            if batch.reconnect { break; }
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(20))))
}
