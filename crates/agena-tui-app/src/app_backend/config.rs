//! Configuration-source presentation: where the config file lives, what JSON
//! sources it resolves to, and the persisted UI preferences.
//!
//! The config read model is assembled by the server and cached on the
//! [`crate::TuiBackend`] at connect time and before settings rebuilds; these
//! helpers are synchronous because settings presentation runs in the TUI event
//! loop.

use agena_tui_transcript::TranscriptDetailDefaults;

use crate::{TuiColorSchemeResource, TuiGraphicsModeResource, TuiPreferencesResource};
use agena_application::dto::ConfigJsonSources;
use anyhow::{Context, Result};
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;

/// The ordered JSON configuration sources that apply for this workspace.
pub(crate) fn config_json_sources(application: &crate::TuiBackend) -> Result<ConfigJsonSources> {
    application
        .config_sources()
        .context("configuration sources have not been loaded from the server yet")
}

/// Persisted UI preferences (theme, graphics mode, …), projected from the
/// resolved configuration document's `ui` section.
pub(crate) fn ui_configuration(application: &crate::TuiBackend) -> TuiPreferencesResource {
    let mut preferences = application
        .config_sources()
        .map(|sources| tui_preferences_from_effective(&sources.effective))
        .unwrap_or_default();
    apply_environment(&mut preferences, |key| std::env::var(key).ok());
    preferences
}

fn apply_environment(
    preferences: &mut TuiPreferencesResource,
    get: impl Fn(&str) -> Option<String>,
) {
    if let Some(value) = get("AGENA_TUI_THEME").filter(|value| !value.trim().is_empty()) {
        preferences.theme = Some(value);
    }
    if let Some(value) = get("AGENA_TUI_COLOR_SCHEME") {
        match value.trim() {
            "auto" => preferences.color_scheme = TuiColorSchemeResource::Auto,
            "dark" => preferences.color_scheme = TuiColorSchemeResource::Dark,
            "light" => preferences.color_scheme = TuiColorSchemeResource::Light,
            _ => tracing::warn!("invalid AGENA_TUI_COLOR_SCHEME; expected auto, dark or light"),
        }
    }
    if let Some(value) = get("AGENA_TUI_GRAPHICS") {
        match value.trim() {
            "auto" => preferences.graphics = TuiGraphicsModeResource::Auto,
            "native" => preferences.graphics = TuiGraphicsModeResource::Native,
            "unicode" => preferences.graphics = TuiGraphicsModeResource::Unicode,
            _ => tracing::warn!("invalid AGENA_TUI_GRAPHICS; expected auto, native or unicode"),
        }
    }
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
    let transcript = ui.get("transcript").unwrap_or(&JsonValue::Null);
    let activity = transcript.get("activity").unwrap_or(&JsonValue::Null);
    let activity_default_expanded = activity
        .get("default_expanded")
        .and_then(JsonValue::as_bool)
        .unwrap_or_default();
    let mut kind_defaults = BTreeMap::new();
    if let Some(kinds) = activity.get("kinds").and_then(JsonValue::as_object) {
        for (id, value) in kinds {
            if let Some(expanded) = value.as_bool() {
                kind_defaults.insert(id.clone(), expanded);
            }
        }
    }
    if let Some(categories) = transcript
        .get("tools")
        .and_then(|tools| tools.get("categories"))
        .and_then(JsonValue::as_object)
    {
        for (category, value) in categories {
            if let Some(expanded) = value.as_bool() {
                kind_defaults.insert(format!("tool-category:{category}"), expanded);
            }
        }
    }
    if let Some(overrides) = transcript
        .get("tools")
        .and_then(|tools| tools.get("overrides"))
        .and_then(JsonValue::as_object)
    {
        for (tool, value) in overrides {
            if let Some(expanded) = value.as_bool() {
                kind_defaults.insert(format!("tool:{tool}"), expanded);
            }
        }
    }
    TuiPreferencesResource {
        locale,
        theme,
        color_scheme,
        graphics,
        transcript_detail_defaults: TranscriptDetailDefaults {
            activity_default_expanded,
            kind_defaults,
        },
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::tui_preferences_from_effective;

    #[test]
    fn shared_transcript_preferences_drive_tui_activity_and_tool_defaults() {
        let preferences = tui_preferences_from_effective(&json!({
            "ui": {
                "tui": {
                    "transcript": {
                        "activity_default_expanded": false,
                        "activity_kinds": { "reasoning": true }
                    }
                },
                "transcript": {
                    "activity": {
                        "default_expanded": true,
                        "kinds": { "reasoning": false, "example.trace": true }
                    },
                    "tools": {
                        "categories": { "bash": false, "read": true },
                        "overrides": { "fs.read": false, "agena.shell.run": true }
                    }
                }
            }
        }));

        assert!(
            preferences
                .transcript_detail_defaults
                .activity_default_expanded
        );
        assert_eq!(
            preferences
                .transcript_detail_defaults
                .kind_defaults
                .get("reasoning"),
            Some(&false)
        );
        assert_eq!(
            preferences
                .transcript_detail_defaults
                .kind_defaults
                .get("example.trace"),
            Some(&true)
        );
        assert_eq!(
            preferences
                .transcript_detail_defaults
                .kind_defaults
                .get("tool-category:bash"),
            Some(&false)
        );
        assert_eq!(
            preferences
                .transcript_detail_defaults
                .kind_defaults
                .get("tool-category:read"),
            Some(&true)
        );
        assert_eq!(
            preferences
                .transcript_detail_defaults
                .kind_defaults
                .get("tool:fs.read"),
            Some(&false)
        );
        assert_eq!(
            preferences
                .transcript_detail_defaults
                .kind_defaults
                .get("tool:agena.shell.run"),
            Some(&true)
        );
    }
}
