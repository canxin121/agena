use std::{collections::HashMap, fs, path::PathBuf, time::SystemTime};

#[derive(Debug, Clone, PartialEq, Eq)]
/// A path stamp tracked by the file watcher.
pub struct WatchPathStamp {
    pub exists: bool,
    pub modified: Option<SystemTime>,
}

pub fn capture_watch_path_stamps(paths: &[PathBuf]) -> HashMap<PathBuf, WatchPathStamp> {
    paths
        .iter()
        .cloned()
        .map(|path| {
            let stamp = match fs::metadata(&path) {
                Ok(metadata) => WatchPathStamp {
                    exists: true,
                    modified: metadata.modified().ok(),
                },
                Err(_) => WatchPathStamp {
                    exists: false,
                    modified: None,
                },
            };
            (path, stamp)
        })
        .collect()
}

pub fn diff_watch_path_stamps(
    previous: &HashMap<PathBuf, WatchPathStamp>,
    current: &HashMap<PathBuf, WatchPathStamp>,
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
