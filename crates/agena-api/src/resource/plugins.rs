//! Public plugin inspection and configuration responses.

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Plugin status with the compact documentation needed by plugin lists.
/// Status fields stay flat for existing clients; full manifests remain in inspect.
pub struct PluginListItem {
    #[serde(flatten)]
    pub status: agena_plugin_sdk::inspection::PluginStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub translations: std::collections::BTreeMap<String, PluginSummaryTranslation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginSummaryTranslation {
    pub summary: String,
}

impl PluginListItem {
    pub fn new(
        status: agena_plugin_sdk::inspection::PluginStatus,
        manifest: Option<agena_plugin_sdk::PluginManifest>,
    ) -> Self {
        let mut item = Self {
            status,
            summary: None,
            translations: std::collections::BTreeMap::new(),
        };
        if let Some(manifest) = manifest {
            item.summary = manifest
                .summary
                .map(|summary| summary.trim().to_owned())
                .filter(|summary| !summary.is_empty());
            item.translations = manifest
                .translations
                .into_iter()
                .filter_map(|(locale, translation)| {
                    let summary = translation.summary?.trim().to_owned();
                    (!summary.is_empty()).then_some((locale, PluginSummaryTranslation { summary }))
                })
                .collect();
        }
        item
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Inspection details of a loaded plugin.
pub struct PluginInspectResponse {
    pub plugin: agena_plugin_sdk::inspection::PluginInspect,
}

fn empty_object() -> serde_json::Value {
    serde_json::json!({})
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Neutral plugin surface with the tool registry generation it reflects.
pub struct PluginSurfaceCatalogResponse {
    pub catalog: agena_plugin_sdk::inspection::PluginSurfaceCatalog,
    /// Registered tools available to permission-policy editors.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permission_tools: Vec<PermissionToolCatalogResource>,
    /// Recent plugin notification intents, newest last.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notifications: Vec<agena_plugin_sdk::inspection::HostNotification>,
    /// Server clock at the notification snapshot, for remote-client baselines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notifications_at_ms: Option<i64>,
    /// Built-in and plugin-contributed transcript activity kinds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub activity_kinds: Vec<agena_domain::ActivityKind>,
    pub tool_registry_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_registry_last_event: Option<agena_plugin_sdk::host_api::ToolRegistryChangedEvent>,
}

use super::PermissionToolCatalogResource;

#[derive(Debug, Clone, Deserialize)]
/// Context of a user-driven plugin command/tool request.
pub struct CommandRequestContext {
    #[serde(default = "empty_object")]
    pub input: serde_json::Value,
    #[serde(default)]
    pub session_id: Option<i64>,
    /// Slash spelling and raw argument text are presentation context exposed
    /// to command hooks. They never participate in authorization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slash: Option<String>,
    #[serde(default)]
    pub raw: String,
}

#[derive(Debug, Clone, Deserialize)]
/// Request to invoke a registered plugin tool explicitly.
pub struct PluginToolInvokeRequest {
    pub tool: String,
    #[serde(default)]
    pub plugin_id: Option<String>,
    #[serde(flatten)]
    pub context: CommandRequestContext,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Complete settings state rendered by every plugin workbench client.
pub struct PluginSettingsResponse {
    pub plugin_id: String,
    pub contract: agena_plugin_sdk::SettingsContract,
    pub defaults: serde_json::Value,
    pub configured: serde_json::Value,
    pub effective: serde_json::Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<PluginSettingsDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// One safe, field-addressed settings diagnostic.
pub struct PluginSettingsDiagnostic {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
/// Replace the complete plugin-owned configuration value.
pub struct PluginSettingsUpdateRequest {
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Persisted settings and Runtime reload outcome.
pub struct PluginSettingsUpdateResponse {
    pub settings: PluginSettingsResponse,
    pub reload_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Log records of a plugin.
pub struct PluginLogListResponse {
    pub plugin_id: String,
    pub logs: Vec<agena_plugin_sdk::inspection::PluginLogRecord>,
}

#[derive(Debug, Clone, Deserialize, Default)]
/// Query for listing plugin log records.
pub struct PluginLogListQuery {
    #[serde(default)]
    pub after_seq: Option<u64>,
    #[serde(default)]
    pub limit: Option<usize>,
}
pub use agena_plugin_sdk::inspection::HostNotification;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginListResponse {
    pub items: Vec<PluginListItem>,
}

pub use agena_plugin_sdk::inspection::{PluginToolInvokeResponse, PluginToolInvokeStatus};

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Server-owned command result, including its routing identity.
pub struct PluginCommandInvokeResponse {
    pub plugin_id: String,
    pub command_id: String,
    pub result: agena_plugin_sdk::CommandResult,
}

#[cfg(test)]
mod client_response_contract_tests {
    use super::*;
    use agena_plugin_sdk::inspection::PluginStatus;
    use serde_json::json;

    fn status() -> PluginStatus {
        PluginStatus::initial(
            &agena_plugin_sdk::PluginKey::new("fixture", "plugin").unwrap(),
            "static",
        )
    }

    #[test]
    fn plugin_list_roundtrips_when_empty_translations_are_omitted() {
        let response = PluginListResponse {
            items: vec![PluginListItem::new(status(), None)],
        };
        let wire = serde_json::to_value(response).unwrap();
        assert!(wire["items"][0].get("translations").is_none());
        assert!(wire["items"][0].get("status").is_none());
        assert_eq!(wire["items"][0]["plugin_id"], "fixture.plugin");
        let decoded: PluginListResponse = serde_json::from_value(wire.clone()).unwrap();
        assert!(decoded.items[0].translations.is_empty());
        assert_eq!(serde_json::to_value(decoded).unwrap(), wire);
    }

    #[test]
    fn plugin_inspection_preserves_flat_service_declarations() {
        let wire = json!({"plugin": {
            "status": status(),
            "services": {"imports": [{
                "id": "fixture.service", "api_version": 1, "optional": true,
                "provider": "fixture.provider", "resolved_provider": "fixture.provider", "state": "ready"
            }]}
        }});
        let decoded: PluginInspectResponse = serde_json::from_value(wire.clone()).unwrap();
        let import = &decoded.plugin.services.as_ref().unwrap().imports[0];
        assert_eq!(import.declaration.id, "fixture.service");
        assert!(import.declaration.optional);
        assert!(import.methods.is_empty());
        assert_eq!(serde_json::to_value(decoded).unwrap(), wire);
    }

    #[test]
    fn plugin_surface_distinguishes_absent_notification_clock_from_zero() {
        let mut wire = json!({"catalog": {}, "tool_registry_generation": 4});
        let decoded: PluginSurfaceCatalogResponse = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(decoded.notifications_at_ms, None);
        assert!(decoded.notifications.is_empty());
        assert!(
            serde_json::to_value(decoded)
                .unwrap()
                .get("notifications_at_ms")
                .is_none()
        );
        wire["notifications_at_ms"] = json!(0);
        let decoded: PluginSurfaceCatalogResponse = serde_json::from_value(wire).unwrap();
        assert_eq!(decoded.notifications_at_ms, Some(0));
        assert_eq!(
            serde_json::to_value(decoded).unwrap()["notifications_at_ms"],
            0
        );
    }

    #[test]
    fn workspace_file_tree_keeps_omitted_leaf_children_and_optional_size() {
        let wire = json!({"workspace_id": 3, "root": "/workspace", "path": "", "entries": [{
            "name": "目录", "path": "目录", "kind": "directory", "children": [
                {"name": "leaf.txt", "path": "目录/leaf.txt", "kind": "file", "size": 2},
                {"name": "link", "path": "目录/link", "kind": "symlink"}
            ]
        }]});
        let decoded: crate::resource::WorkspaceFileTreeResource =
            serde_json::from_value(wire.clone()).unwrap();
        assert!(decoded.entries[0].children[0].children.is_empty());
        assert_eq!(decoded.entries[0].children[1].size, None);
        assert_eq!(serde_json::to_value(decoded).unwrap(), wire);
    }

    #[test]
    fn settings_edit_missing_revisions_remain_compatible_and_preserve_null_values() {
        let wire = json!({
            "config_path": "/workspace/agena.toml", "config_found": true,
            "operation": "set", "dry_run": false, "changed": true, "created": false,
            "deleted": false, "validated": true, "reload_requested": false, "reload_required": false,
            "previous": null, "current": {"value": true}
        });
        let decoded: crate::resource::ConfigSettingsEditResponse =
            serde_json::from_value(wire.clone()).unwrap();
        assert!(decoded.before_revision.is_empty());
        assert!(decoded.after_revision.is_empty());
        assert!(decoded.path.is_none());
        assert!(decoded.reload.is_none());
        let encoded = serde_json::to_value(decoded).unwrap();
        assert!(encoded.get("path").is_none());
        assert!(encoded.get("reload").is_none());
        assert_eq!(encoded["previous"], wire["previous"]);
        assert_eq!(encoded["current"], wire["current"]);
    }
}
