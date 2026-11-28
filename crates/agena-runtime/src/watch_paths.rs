use std::path::{Path, PathBuf};

use agena_plugin_host::{PluginHost, PluginPackage, PluginsConfig};

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

    pub fn as_slice(&self) -> &[PathBuf] {
        &self.paths
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
    workspace_root: &Path,
    plugins: &PluginsConfig,
    global_plugins: &PluginsConfig,
    hosts: [&PluginHost; 2],
) -> WatchPathSet {
    let mut paths = vec![config_path.to_path_buf(), project_config_path.to_path_buf()];
    paths.extend(
        agena_runtime_tools::claude_project_settings_paths(workspace_root).map(|(_, path)| path),
    );
    let base = config_path.parent().unwrap_or_else(|| Path::new("."));

    for entry in plugins.list.values().chain(global_plugins.list.values()) {
        match &entry.package {
            PluginPackage::Cdylib { path, .. } => paths.push(base.join(path)),
            PluginPackage::AgentPlugin { path, bridge, .. } if entry.enabled => {
                paths.extend(agena_plugin_host::agent_plugin::manifest_watch_paths(
                    &base.join(path),
                ));
                if let Some(bridge) = bridge
                    .as_deref()
                    .filter(|bridge| bridge.follow_installation)
                {
                    paths.extend(
                        agena_plugin_host::agent_plugin::native_installation_watch_paths(bridge),
                    );
                }
            }
            _ => {}
        }
    }
    for host in hosts {
        for package in host
            .plugins()
            .iter()
            .filter_map(|plugin| plugin.agent_plugin())
        {
            paths.extend(package.watch_paths.iter().cloned());
        }
    }
    WatchPathSet::from_paths(paths)
}
