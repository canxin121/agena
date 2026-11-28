use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
    time::{Duration, SystemTime},
};

use agena_plugin_sdk::{
    SessionFileChange,
    host_api::{ImportedPluginFileEvents, ImportedPluginWatchPath},
};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::{
    sync::{oneshot, watch},
    task::AbortHandle,
};

static WATCH_IO: agena_async::BlockingPool = agena_async::BlockingPool::new(4);
const COALESCE: Duration = Duration::from_millis(50);
const RECONCILE: Duration = Duration::from_secs(15);

#[derive(Default)]
pub struct FileWatches(Mutex<WatchSessions>);

#[derive(Default)]
struct WatchSessions {
    closed: bool,
    active: BTreeMap<i64, SessionWatch>,
}

struct SessionWatch {
    dynamic: watch::Sender<WatchedPaths>,
    reconciled: watch::Receiver<u64>,
    task: AbortHandle,
}

#[derive(Clone)]
struct WatchedPaths {
    paths: Vec<PathBuf>,
    revision: u64,
}

impl Drop for SessionWatch {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl SessionWatch {
    fn replace_paths(&self, paths: Vec<PathBuf>) {
        self.dynamic.send_if_modified(|dynamic| {
            if dynamic.paths == paths {
                return false;
            }
            dynamic.paths = paths;
            dynamic.revision = dynamic.revision.saturating_add(1);
            true
        });
    }

    fn refresh(&self) -> u64 {
        let mut revision = 0;
        self.dynamic.send_modify(|dynamic| {
            dynamic.revision = dynamic.revision.saturating_add(1);
            revision = dynamic.revision;
        });
        revision
    }
}

impl FileWatches {
    pub fn replace_paths(&self, session: i64, paths: Vec<PathBuf>) {
        let sessions = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(watch) = sessions.active.get(&session) {
            watch.replace_paths(paths);
        }
    }

    pub async fn configure(
        &self,
        session: i64,
        has_static_paths: bool,
        paths: Option<Vec<PathBuf>>,
        events: Arc<dyn ImportedPluginFileEvents>,
    ) -> Result<(), String> {
        let ready = {
            let mut sessions = self.0.lock().unwrap_or_else(|error| error.into_inner());
            if sessions.closed {
                return Ok(());
            }
            if let Some(watch) = sessions
                .active
                .get(&session)
                .filter(|watch| !watch.task.is_finished())
            {
                if let Some(paths) = paths {
                    watch.replace_paths(paths);
                }
                return Ok(());
            }
            let paths = paths.unwrap_or_default();
            if !has_static_paths && paths.is_empty() {
                return Ok(());
            }
            let (dynamic, receiver) = watch::channel(WatchedPaths { paths, revision: 0 });
            let (reconciled, observed) = watch::channel(0);
            let (ready, registered) = oneshot::channel();
            let task = tokio::spawn(run(events, receiver, reconciled, ready));
            sessions.active.insert(
                session,
                SessionWatch {
                    dynamic,
                    reconciled: observed,
                    task: task.abort_handle(),
                },
            );
            registered
        };
        ready
            .await
            .map_err(|_| "file hook watcher stopped during registration".to_owned())
    }

    pub fn refresh(&self) {
        let sessions = self.0.lock().unwrap_or_else(|error| error.into_inner());
        for watch in sessions.active.values() {
            watch.refresh();
        }
    }

    pub fn refresh_session(&self, session: i64) {
        let sessions = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(watch) = sessions.active.get(&session) {
            watch.refresh();
        }
    }

    /// Wait until the live watch list has applied the caller's latest grants.
    pub async fn refresh_session_and_wait(&self, session: i64) {
        let pending = {
            let sessions = self.0.lock().unwrap_or_else(|error| error.into_inner());
            sessions
                .active
                .get(&session)
                .map(|watch| (watch.refresh(), watch.reconciled.clone()))
        };
        if let Some((revision, mut reconciled)) = pending {
            let _ = reconciled.wait_for(|applied| *applied >= revision).await;
        }
    }

    pub fn end_session(&self, session: i64) {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active
            .remove(&session);
    }

    pub fn close(&self) {
        let mut sessions = self.0.lock().unwrap_or_else(|error| error.into_inner());
        sessions.closed = true;
        sessions.active.clear();
    }
}

async fn run(
    binding: Arc<dyn ImportedPluginFileEvents>,
    mut dynamic: watch::Receiver<WatchedPaths>,
    reconciled: watch::Sender<u64>,
    ready: oneshot::Sender<()>,
) {
    let (changed, mut filesystem) = watch::channel(());
    let mut state = WatchState {
        watcher: None,
        signal: changed,
        interests: Arc::new(RwLock::new(BTreeSet::new())),
        directories: BTreeMap::new(),
        files: BTreeMap::new(),
    };
    let mut ready = Some(ready);
    loop {
        let request = dynamic.borrow_and_update().clone();
        let paths = match binding.resolve_paths(request.paths).await {
            Ok(paths) => paths,
            Err(error) => {
                tracing::debug!(%error, "could not resolve plugin file watches");
                Vec::new()
            }
        };
        tracing::debug!(paths = paths.len(), "resolved session file watches");
        let Ok((next, changes)) = WATCH_IO
            .run(move || {
                let changes = state.reconcile(paths);
                (state, changes)
            })
            .await
        else {
            return;
        };
        state = next;
        reconciled.send_replace(request.revision);
        if let Some(ready) = ready.take() {
            let _ = ready.send(());
        }
        for (path, change) in changes {
            if let Err(error) = binding.changed(path, change).await {
                tracing::warn!(%error, "plugin file hook failed");
            }
        }
        tokio::select! {
            biased;
            result = dynamic.changed() => if result.is_err() { return; },
            _ = filesystem.changed() => {
                tokio::time::sleep(COALESCE).await;
                filesystem.borrow_and_update();
            }
            _ = tokio::time::sleep(RECONCILE) => {}
        }
    }
}

struct WatchState {
    watcher: Option<RecommendedWatcher>,
    signal: watch::Sender<()>,
    interests: Arc<RwLock<BTreeSet<PathBuf>>>,
    directories: BTreeMap<PathBuf, DirectoryIdentity>,
    files: BTreeMap<PathBuf, (PathBuf, Option<FileStamp>)>,
}

impl WatchState {
    fn reconcile(
        &mut self,
        paths: Vec<ImportedPluginWatchPath>,
    ) -> Vec<(ImportedPluginWatchPath, SessionFileChange)> {
        let interests = paths
            .iter()
            .flat_map(|path| [&path.requested, &path.resolved])
            .cloned()
            .collect::<BTreeSet<_>>();
        let parents = interests
            .iter()
            .filter_map(|path| {
                path.ancestors().skip(1).find_map(|parent| {
                    let metadata = fs::metadata(parent).ok()?;
                    metadata
                        .is_dir()
                        .then(|| (parent.to_path_buf(), DirectoryIdentity::new(&metadata)))
                })
            })
            .collect::<BTreeMap<_, _>>();
        *self
            .interests
            .write()
            .unwrap_or_else(|error| error.into_inner()) = interests;
        self.watch_directories(parents);

        let mut previous = std::mem::take(&mut self.files);
        let mut changes = Vec::new();
        for path in paths {
            let current = FileStamp::read(&path.resolved);
            let change = previous
                .remove(&path.requested)
                .and_then(|(resolved, old)| match (&old, &current) {
                    (None, Some(_)) => Some(SessionFileChange::Add),
                    (Some(_), None) => Some(SessionFileChange::Unlink),
                    (Some(old), Some(new)) if old != new || resolved != path.resolved => {
                        Some(SessionFileChange::Change)
                    }
                    _ => None,
                });
            self.files
                .insert(path.requested.clone(), (path.resolved.clone(), current));
            if let Some(change) = change {
                changes.push((path, change));
            }
        }
        changes
    }

    fn watch_directories(&mut self, next: BTreeMap<PathBuf, DirectoryIdentity>) {
        if next.is_empty() {
            self.watcher = None;
            self.directories.clear();
            return;
        }
        if self.watcher.is_none() {
            let signal = self.signal.clone();
            let interests = self.interests.clone();
            self.watcher =
                notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    let relevant = event.map_or(true, |event| {
                        !matches!(event.kind, EventKind::Access(_))
                            && (event.need_rescan()
                                || event.paths.iter().any(|path| {
                                    interests
                                        .read()
                                        .unwrap_or_else(|error| error.into_inner())
                                        .iter()
                                        .any(|target| target.starts_with(path))
                                }))
                    });
                    if relevant {
                        signal.send_replace(());
                    }
                })
                .map_err(|error| tracing::warn!(%error, "native file hook watcher unavailable"))
                .ok();
        }
        let Some(watcher) = self.watcher.as_mut() else {
            return;
        };
        self.directories.retain(|path, identity| {
            if next.get(path) == Some(identity) {
                true
            } else {
                let _ = watcher.unwatch(path);
                false
            }
        });
        for (path, identity) in next {
            if !self.directories.contains_key(&path) {
                match watcher.watch(&path, RecursiveMode::NonRecursive) {
                    Ok(()) => {
                        self.directories.insert(path, identity);
                    }
                    Err(error) => tracing::debug!(%error, "file hook directory watch unavailable"),
                }
            }
        }
    }
}

#[derive(PartialEq, Eq)]
struct DirectoryIdentity {
    created_at: Option<SystemTime>,
    #[cfg(unix)]
    inode: (u64, u64),
}

impl DirectoryIdentity {
    fn new(metadata: &fs::Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Self {
            created_at: metadata.created().ok(),
            #[cfg(unix)]
            inode: (metadata.dev(), metadata.ino()),
        }
    }
}

#[derive(PartialEq, Eq)]
struct FileStamp {
    identity: DirectoryIdentity,
    size: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    changed: (i64, i64),
}

impl FileStamp {
    fn read(path: &Path) -> Option<Self> {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::metadata(path).ok()?;
        metadata.is_file().then(|| Self {
            identity: DirectoryIdentity::new(&metadata),
            size: metadata.len(),
            modified: metadata.modified().ok(),
            #[cfg(unix)]
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
        seen: tokio::sync::mpsc::UnboundedSender<(PathBuf, SessionFileChange)>,
    }

    #[async_trait::async_trait]
    impl ImportedPluginFileEvents for Fixture {
        async fn resolve_paths(
            &self,
            absolute: Vec<PathBuf>,
        ) -> agena_plugin_sdk::Result<Vec<ImportedPluginWatchPath>> {
            Ok(std::iter::once(self.root.join(".envrc"))
                .chain(absolute)
                .map(|path| ImportedPluginWatchPath {
                    requested: path.clone(),
                    resolved: path,
                })
                .collect())
        }

        async fn changed(
            &self,
            path: ImportedPluginWatchPath,
            change: SessionFileChange,
        ) -> agena_plugin_sdk::Result<()> {
            self.seen.send((path.requested, change)).unwrap();
            Ok(())
        }
    }

    #[tokio::test]
    async fn native_watcher_reports_external_creation_replacement_and_deletion() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let filename = PathBuf::from(".envrc");
        let path = root.join(&filename);
        let (seen, mut changes) = tokio::sync::mpsc::unbounded_channel();
        let watches = FileWatches::default();
        watches
            .configure(1, true, None, Arc::new(Fixture { root, seen }))
            .await
            .unwrap();
        async fn next(
            changes: &mut tokio::sync::mpsc::UnboundedReceiver<(PathBuf, SessionFileChange)>,
        ) -> (PathBuf, SessionFileChange) {
            tokio::time::timeout(Duration::from_secs(5), changes.recv())
                .await
                .unwrap()
                .unwrap()
        }
        fs::write(&path, "one").unwrap();
        assert_eq!(
            next(&mut changes).await,
            (path.clone(), SessionFileChange::Add)
        );
        tempfile::NamedTempFile::new_in(dir.path())
            .unwrap()
            .persist(&path)
            .unwrap();
        assert_eq!(
            next(&mut changes).await,
            (path.clone(), SessionFileChange::Change)
        );
        fs::remove_file(&path).unwrap();
        assert_eq!(
            next(&mut changes).await,
            (path.clone(), SessionFileChange::Unlink)
        );

        watches.end_session(1);
        fs::write(&path, "after session end").unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(1), changes.recv())
                .await
                .unwrap()
                .is_none()
        );
    }
}
