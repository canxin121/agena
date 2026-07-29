use std::path::{Path, PathBuf};

use agena_plugin_host::{PluginPackage, PluginsConfig};

/// Immutable, deduplicated paths watched by runtime reload tasks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchPathSet {
    paths: Vec<PathBuf>,
}

impl WatchPathSet {
    pub fn new() -> Self {
        Self { paths: Vec::new() }
    }

    pub fn from_paths(mut paths: Vec<PathBuf>) -> Self {
        paths.sort();
        paths.dedup();
        Self { paths }
    }

    /// Add a path while preserving the set invariant.
    pub fn insert(&mut self, path: PathBuf) {
        if !self.paths.iter().any(|existing| existing == &path) {
            self.paths.push(path);
            self.paths.sort();
        }
    }

    #[cfg(test)]
    pub fn extend<I>(&mut self, paths: I)
    where
        I: IntoIterator<Item = PathBuf>,
    {
        for path in paths {
            self.insert(path);
        }
    }

    pub fn as_slice(&self) -> &[PathBuf] {
        &self.paths
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

impl Default for WatchPathSet {
    fn default() -> Self {
        Self::new()
    }
}

/// Derive the complete reload watch set from resolved plugin configuration.
///
/// Configuration parsing remains outside Runtime, but path selection and
/// deduplication are lifecycle policy: a rebuilt runtime must watch its user
/// and project config plus local plugin artifacts.
pub fn runtime_watch_paths(
    config_path: &Path,
    project_config_path: &Path,
    plugins: &PluginsConfig,
) -> WatchPathSet {
    let mut paths = WatchPathSet::new();
    paths.insert(config_path.to_path_buf());
    paths.insert(project_config_path.to_path_buf());

    let base_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    for entry in plugins.list.values() {
        if let PluginPackage::Cdylib { path, .. } = &entry.package {
            paths.insert(if path.is_absolute() {
                path.clone()
            } else {
                base_dir.join(path)
            });
        }
    }
    paths
}
