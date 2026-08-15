//! Configuration-source presentation: where the config file lives, what JSON
//! sources it resolves to, and the persisted UI preferences.

use agena_application::dto::{ConfigJsonSources, TuiPreferencesResource};
use anyhow::{Context, Result};
use std::path::PathBuf;

/// Absolute path to the resolved configuration file.
pub(crate) fn config_path(application: &crate::TuiBackend) -> PathBuf {
    application
        .embedded_application()
        .ok()
        .and_then(|application| application.config_path().ok())
        .unwrap_or_else(|| {
            application
                .workspace_root()
                .join(".agena-remote-config-unavailable")
        })
}

/// The ordered JSON configuration sources that apply for this workspace.
pub(crate) fn config_json_sources(application: &crate::TuiBackend) -> Result<ConfigJsonSources> {
    application
        .embedded_application()?
        .config_json_sources()
        .context("failed to read Application configuration-source projection")
}

/// Persisted UI preferences (theme, graphics mode, …).
pub(crate) fn ui_configuration(application: &crate::TuiBackend) -> TuiPreferencesResource {
    application
        .embedded_application()
        .ok()
        .and_then(|application| application.tui_preferences().ok())
        .unwrap_or_default()
}
