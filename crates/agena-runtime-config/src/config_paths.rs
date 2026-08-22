//! Schema-neutral paths used by configuration/bootstrap adapters.

use std::path::{Path, PathBuf};

use crate::ConfigEnvironment;

const DEFAULT_CONFIG_DIR_NAME: &str = "agena";
const DEFAULT_CONFIG_FILE_NAME: &str = "agena.json";

pub fn default_config_path(env: &impl ConfigEnvironment) -> PathBuf {
    let mut base = home_dir(env).unwrap_or_else(|| {
        tracing::error!(
            "runtime config home is unavailable because neither HOME nor USERPROFILE is set; using the current-directory compatibility path"
        );
        PathBuf::from(".")
    });
    base.push(DEFAULT_CONFIG_DIR_NAME);
    base.push(DEFAULT_CONFIG_FILE_NAME);
    base
}

pub fn default_workspace_root() -> PathBuf {
    match try_default_workspace_root() {
        Ok(path) => path,
        Err(error) => {
            tracing::error!(
                diagnostic = %agena_failure::diagnostic::format_error_chain(&error),
                "runtime configuration caller used the compatibility workspace fallback"
            );
            PathBuf::from(".")
        }
    }
}

pub fn try_default_workspace_root() -> Result<PathBuf, crate::ConfigError> {
    std::env::current_dir().map_err(|source| crate::ConfigError::CurrentDirectory { source })
}

pub fn project_config_path(workspace_root: &Path) -> PathBuf {
    workspace_root
        .join(format!(".{DEFAULT_CONFIG_DIR_NAME}"))
        .join(DEFAULT_CONFIG_FILE_NAME)
}

fn home_dir(env: &impl ConfigEnvironment) -> Option<PathBuf> {
    env.var("HOME")
        .or_else(|| env.var("USERPROFILE"))
        .map(PathBuf::from)
}
