//! Schema-neutral paths used by configuration/bootstrap adapters.

pub use agena_storage_layout::project_config_path;
use std::path::PathBuf;

use crate::ConfigEnvironment;

pub fn default_config_path(env: &impl ConfigEnvironment) -> PathBuf {
    env.storage_layout().config_path()
}

pub fn try_default_workspace_root() -> Result<PathBuf, crate::ConfigError> {
    std::env::current_dir().map_err(|source| crate::ConfigError::CurrentDirectory { source })
}
