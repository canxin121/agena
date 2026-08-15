//! Configuration-source presentation: where the config file lives, what JSON
//! sources it resolves to, and the persisted UI preferences.
//!
//! The config read model is assembled by the center and cached on the
//! [`crate::TuiBackend`] at connect time and before settings rebuilds; these
//! helpers are synchronous because settings presentation runs in the TUI event
//! loop.

use agena_application::dto::{
    ConfigJsonSources, TuiColorSchemeResource, TuiGraphicsModeResource, TuiPreferencesResource,
};
use anyhow::{Context, Result};
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Absolute path to the resolved global configuration file. In remote mode the
/// path comes from the center's settings read model; when that is not yet
/// loaded, a workspace-anchored placeholder is returned.
pub(crate) fn config_path(application: &crate::TuiBackend) -> PathBuf {
    application
        .config_sources()
        .map(|sources| sources.config_path)
        .unwrap_or_else(|| {
            application
                .workspace_root()
                .join(".agena-remote-config-unavailable")
        })
}

/// The ordered JSON configuration sources that apply for this workspace.
pub(crate) fn config_json_sources(application: &crate::TuiBackend) -> Result<ConfigJsonSources> {
    application
        .config_sources()
        .context("configuration sources have not been loaded from the center yet")
}

/// Persisted UI preferences (theme, graphics mode, …), projected from the
/// resolved configuration document's `ui` section.
pub(crate) fn ui_configuration(application: &crate::TuiBackend) -> TuiPreferencesResource {
    application
        .config_sources()
        .map(|sources| tui_preferences_from_effective(&sources.effective))
        .unwrap_or_default()
}

fn tui_preferences_from_effective(effective: &JsonValue) -> TuiPreferencesResource {
    let ui = effective.get("ui").unwrap_or(&JsonValue::Null);
    let locale = ui
        .get("locale")
        .and_then(JsonValue::as_str)
        .map(str::to_owned);
    let tui = ui.get("tui").unwrap_or(&JsonValue::Null);
    let theme = tui
        .get("theme")
        .and_then(JsonValue::as_str)
        .map(str::to_owned);
    let color_scheme = match tui.get("color_scheme").and_then(JsonValue::as_str) {
        Some("dark") => TuiColorSchemeResource::Dark,
        Some("light") => TuiColorSchemeResource::Light,
        _ => TuiColorSchemeResource::Auto,
    };
    let graphics = match tui.get("graphics").and_then(JsonValue::as_str) {
        Some("native") => TuiGraphicsModeResource::Native,
        Some("unicode") => TuiGraphicsModeResource::Unicode,
        _ => TuiGraphicsModeResource::Auto,
    };
    let transcript = tui.get("transcript").unwrap_or(&JsonValue::Null);
    let transcript_activity_default_expanded = transcript
        .get("activity_default_expanded")
        .and_then(JsonValue::as_bool)
        .unwrap_or_default();
    let mut transcript_activity_kinds = BTreeMap::new();
    if let Some(kinds) = transcript
        .get("activity_kinds")
        .and_then(JsonValue::as_object)
    {
        for (id, value) in kinds {
            if let Some(expanded) = value.as_bool() {
                transcript_activity_kinds.insert(id.clone(), expanded);
            }
        }
    }
    TuiPreferencesResource {
        locale,
        theme,
        color_scheme,
        graphics,
        transcript_activity_default_expanded,
        transcript_activity_kinds,
    }
}
