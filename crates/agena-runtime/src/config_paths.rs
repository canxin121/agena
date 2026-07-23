//! Schema-neutral paths used by configuration/bootstrap adapters.

use std::path::{Path, PathBuf};

use crate::ConfigEnvironment;

const DEFAULT_CONFIG_DIR_NAME: &str = "agena";
const DEFAULT_CONFIG_FILE_NAME: &str = "agena.json";

pub fn default_config_path(env: &impl ConfigEnvironment) -> PathBuf {
    let mut base = home_dir(env).unwrap_or_else(|| PathBuf::from("."));
    base.push(DEFAULT_CONFIG_DIR_NAME);
    base.push(DEFAULT_CONFIG_FILE_NAME);
    base
}

pub(crate) fn default_workspace_root() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

pub(crate) fn project_config_path(workspace_root: &Path) -> PathBuf {
    workspace_root
        .join(format!(".{DEFAULT_CONFIG_DIR_NAME}"))
        .join(DEFAULT_CONFIG_FILE_NAME)
}

fn home_dir(env: &impl ConfigEnvironment) -> Option<PathBuf> {
    env.var("HOME")
        .or_else(|| env.var("USERPROFILE"))
        .map(PathBuf::from)
}
