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
    let (watcher, mut events, changes) = tokio::task::spawn_blocking(move || {
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
                let mut pending = pending.lock().expect("filesystem changes lock");
                match event {
                    Ok(event) => {
                        if matches!(event.kind, notify::EventKind::Access(_)) {
                            return;
                        }
                        pending.truncated |= event.need_rescan();
                        // OS watches can be detached by directory deletion or
                        // replacement. Reopen the stream to watch the new inode.
                        let topology_change = matches!(
                            event.kind,
                            notify::EventKind::Create(_)
                                | notify::EventKind::Remove(_)
                                | notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
                        );
                        for path in event.paths {
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
                                pending.reconnect |= topology_change && watch_paths.contains(&path);
                                if pending.paths.len() < 256 {
                                    pending.paths.insert(to_api_path(&path));
                                } else {
                                    pending.truncated = true;
                                }
                            }
                        }
                    }
                    Err(_) => pending.truncated = true,
                }
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
        Ok::<_, AppError>((watcher, events, changes))
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
