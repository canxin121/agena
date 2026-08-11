//! Configuration-source presentation: where the config file lives, what JSON
//! sources it resolves to, and the persisted UI preferences.

use agena_application::Application;
use agena_application::dto::{ConfigJsonSources, TuiPreferencesResource};
use anyhow::{Context, Result};
use std::path::PathBuf;

/// Absolute path to the resolved configuration file.
pub(crate) fn config_path(application: &Application) -> PathBuf {
    application
        .config_path()
        .expect("Application configuration projection must provide its config path")
}

/// The ordered JSON configuration sources that apply for this workspace.
pub(crate) fn config_json_sources(application: &Application) -> Result<ConfigJsonSources> {
    application
        .config_json_sources()
        .context("failed to read Application configuration-source projection")
}

/// Persisted UI preferences (theme, graphics mode, …).
pub(crate) fn ui_configuration(application: &Application) -> TuiPreferencesResource {
    application
        .tui_preferences()
        .expect("Application configuration projection must provide UI preferences")
}
