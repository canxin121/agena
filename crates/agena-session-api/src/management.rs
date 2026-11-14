//! Typed control-plane operations shared by HTTP and plugin clients.
//!
//! Each request names an application operation. Transport authentication and
//! browser redirect handling stay at the transport boundary.
pub mod marketplace;
pub mod provider;
pub mod settings;

use agena_domain::mcp_connections::*;
use agena_provider::draft::ProviderConfigDraft;
use agena_skills::source::{
    SkillCreateRequest, SkillEnabledRequest, SkillInstallRequest, SkillResourceWriteRequest,
    SkillUpdateRequest, SkillValidationRequest,
};
use marketplace::*;
use provider::{AuthLoginKindResource, ProviderAdapterModelsResource, ProviderModelResource};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use settings::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "domain", content = "request", rename_all = "snake_case")]
pub enum ManagementQuery {
    Settings(SettingsQuery),
    Providers(ProviderQuery),
    Auth(AuthQuery),
    ModelCatalog(ModelCatalogQuery),
    Mcp(McpQuery),
    Skills(SkillsQuery),
    Plugins(PluginQuery),
    Marketplace(MarketplaceQuery),
    RuntimeTasks,
    RuntimeMetrics,
    RuntimeDiagnostics,
    Usage(UsageQuery),
    OperatorTools,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "domain", content = "request", rename_all = "snake_case")]
pub enum ManagementCommand {
    Settings(SettingsCommand),
    Providers(ProviderCommand),
    Auth(AuthCommand),
    ModelCatalogRefresh,
    Mcp(McpCommand),
    Skills(SkillsCommand),
    Plugins(PluginCommand),
    Marketplace(MarketplaceCommand),
    ReloadRuntime,
    CancelRuntimeTask {
        task_id: String,
    },
    InvokeOperatorTool {
        workspace_id: i64,
        tool: String,
        #[serde(default)]
        input: Option<Value>,
    },
    PermissionRulesBatch(PermissionRuleBatchRequest),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum SettingsQuery {
    Get(ConfigSettingsGetInput),
    GetLayer {
        layer: ConfigSettingsLayer,
        input: ConfigSettingsGetInput,
    },
    Sources {
        paths: Vec<String>,
    },
    List(ConfigSettingsListInput),
    Resolved,
    Validate(ConfigSettingsValidateInput),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum SettingsCommand {
    Set(ConfigSettingsSetInput),
    SetLayer {
        layer: ConfigSettingsLayer,
        input: ConfigSettingsSetInput,
    },
    Patch(ConfigSettingsPatchInput),
    Delete(ConfigSettingsDeleteInput),
    DeleteLayer {
        layer: ConfigSettingsLayer,
        input: ConfigSettingsDeleteInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum ProviderQuery {
    ConfiguredModels {
        provider_id: String,
    },
    ConfiguredAdapterModels {
        provider_id: String,
    },
    AwsProfiles,
    Draft {
        #[serde(default)]
        provider_id: Option<String>,
    },
    DraftModels {
        draft: ProviderConfigDraft,
        #[serde(default)]
        adapter_ids: Vec<String>,
    },
    ModelDraft {
        draft: ProviderConfigDraft,
        adapter_id: String,
        model_id: String,
        #[serde(default)]
        provider_model: Option<ProviderModelResource>,
    },
    CatalogTemplate {
        catalog_model_id: String,
        model_value: Value,
        #[serde(default)]
        clear: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum ProviderCommand {
    RefreshClientVersions,
    SaveDraft {
        draft: ProviderConfigDraft,
        #[serde(default)]
        adapter_models: Vec<ProviderAdapterModelsResource>,
        #[serde(default)]
        selected_adapter_ids: Vec<String>,
        #[serde(default)]
        selected_model_keys: BTreeSet<String>,
        #[serde(default)]
        model_config_values: BTreeMap<String, Value>,
    },
    SaveAdapter {
        draft: ProviderConfigDraft,
        adapter: ProviderAdapterModelsResource,
    },
    SaveModel {
        draft: ProviderConfigDraft,
        adapter_id: String,
        model_id: String,
        value: Value,
    },
    DeleteProvider {
        provider_id: String,
    },
    DeleteAdapter {
        draft: ProviderConfigDraft,
        adapter_id: String,
    },
    DeleteModel {
        draft: ProviderConfigDraft,
        adapter_id: String,
        model_id: String,
    },
    StartDraftAuth {
        draft: ProviderConfigDraft,
    },
    ContinueDraftAuth {
        draft: ProviderConfigDraft,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum AuthQuery {
    List,
    Get { provider_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum AuthCommand {
    SetApiKey {
        provider_id: String,
        api_key: String,
    },
    Remove {
        provider_id: String,
    },
    Refresh {
        provider_id: String,
    },
    StartBrowser {
        provider_id: String,
        kind: AuthLoginKindResource,
        redirect_uri: String,
    },
    FinishBrowser {
        provider_id: String,
        kind: AuthLoginKindResource,
        code: String,
        pkce_verifier: String,
        redirect_uri: String,
    },
    StartDevice {
        provider_id: String,
        kind: AuthLoginKindResource,
        #[serde(default)]
        enterprise_domain: Option<String>,
    },
    PollDevice {
        provider_id: String,
        kind: AuthLoginKindResource,
        device_code: String,
        #[serde(default)]
        user_code: Option<String>,
        #[serde(default)]
        enterprise_domain: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum ModelCatalogQuery {
    List {
        #[serde(default)]
        q: String,
        #[serde(default)]
        origin: Option<String>,
        #[serde(default)]
        offset: usize,
        #[serde(default)]
        limit: Option<usize>,
    },
    Lookup {
        model_ids: Vec<String>,
    },
    Match {
        model_ids: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum McpQuery {
    List,
    Get {
        id: String,
    },
    PreviewImport(McpImportPreviewRequest),
    Capabilities {
        id: String,
        kind: String,
        #[serde(default)]
        cursor: Option<String>,
    },
    ReadResource {
        id: String,
        request: McpResourceReadRequest,
    },
    GetPrompt {
        id: String,
        request: McpPromptGetRequest,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum McpCommand {
    Create(McpConnectionCreateRequest),
    Update {
        id: String,
        request: McpConnectionUpdateRequest,
    },
    Remove {
        id: String,
        expected_revision: String,
    },
    SetEnabled {
        id: String,
        request: McpConnectionEnabledRequest,
    },
    SetBridgeEnabled(McpBridgeEnabledRequest),
    Import(McpImportRequest),
    Reconnect {
        id: String,
    },
    Test {
        id: String,
    },
    SetCredential {
        id: String,
        token: String,
    },
    DeleteCredential {
        id: String,
    },
    StartAuth {
        id: String,
        request: McpOAuthStartRequest,
    },
    CompleteAuth {
        id: String,
        request: McpOAuthCompleteRequest,
    },
    CancelAuth {
        id: String,
        flow_id: String,
    },
    Logout {
        id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum SkillsQuery {
    List {
        #[serde(default)]
        workspace_id: Option<i64>,
    },
    Get {
        #[serde(default)]
        workspace_id: Option<i64>,
        id: String,
    },
    Resource {
        #[serde(default)]
        workspace_id: Option<i64>,
        id: String,
        path: String,
    },
    Validate {
        #[serde(default)]
        workspace_id: Option<i64>,
        request: SkillValidationRequest,
    },
    PreviewInstall {
        #[serde(default)]
        workspace_id: Option<i64>,
        request: SkillInstallRequest,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum SkillsCommand {
    Create {
        #[serde(default)]
        workspace_id: Option<i64>,
        request: SkillCreateRequest,
    },
    Update {
        #[serde(default)]
        workspace_id: Option<i64>,
        id: String,
        request: SkillUpdateRequest,
    },
    Delete {
        #[serde(default)]
        workspace_id: Option<i64>,
        id: String,
        expected_revision: String,
    },
    SetEnabled {
        #[serde(default)]
        workspace_id: Option<i64>,
        id: String,
        request: SkillEnabledRequest,
    },
    WriteResource {
        #[serde(default)]
        workspace_id: Option<i64>,
        id: String,
        request: SkillResourceWriteRequest,
    },
    DeleteResource {
        #[serde(default)]
        workspace_id: Option<i64>,
        id: String,
        path: String,
        expected_revision: String,
    },
    Install {
        #[serde(default)]
        workspace_id: Option<i64>,
        request: SkillInstallRequest,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum PluginQuery {
    List,
    Get {
        plugin_id: String,
    },
    Settings {
        plugin_id: String,
    },
    Architecture,
    Surface,
    ToolChanges {
        #[serde(default)]
        after_generation: Option<u64>,
        #[serde(default)]
        limit: Option<usize>,
    },
    Logs {
        plugin_id: String,
        #[serde(default)]
        after_seq: Option<u64>,
        #[serde(default)]
        limit: Option<usize>,
    },
    WebSettings {
        plugin_id: String,
        page_id: String,
        generation: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum PluginCommand {
    SetSettings {
        plugin_id: String,
        settings: Value,
    },
    InvokeTool {
        plugin_id: String,
        tool_id: String,
        input: Value,
        #[serde(default)]
        session_id: Option<i64>,
    },
    InvokeCommand {
        plugin_id: String,
        command_id: String,
        input: Value,
        #[serde(default)]
        session_id: Option<i64>,
        #[serde(default)]
        slash: Option<String>,
        #[serde(default)]
        raw: String,
    },
    SetWebSettings {
        plugin_id: String,
        page_id: String,
        generation: String,
        settings: Value,
    },
    InvokeWebCommand {
        plugin_id: String,
        page_id: String,
        generation: String,
        command_id: String,
        input: Value,
        #[serde(default)]
        session_id: Option<i64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum MarketplaceQuery {
    Search(MarketplaceSearchRequest),
    Installed,
    Outdated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum MarketplaceCommand {
    Sync(MarketplaceRegistryRequest),
    Install(MarketplaceInstallRequest),
    Uninstall(MarketplaceUninstallRequestBody),
    Upgrade(MarketplaceUpgradeRequest),
}

/// Usage filters preserve the HTTP date/CSV grammar in the shared service.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageQuery {
    #[serde(default)]
    pub period: Option<agena_domain::UsagePeriod>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub include_subagents: Option<bool>,
    #[serde(default)]
    pub timezone_offset_minutes: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRuleBatchRequest {
    pub rule_ids: Vec<i64>,
    pub scope: String,
    #[serde(default)]
    pub session_id: Option<i64>,
    pub operation: PermissionRuleBatchOperation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum PermissionRuleBatchOperation {
    Delete,
    SetMode {
        mode: crate::resource::PermissionMode,
    },
}

impl ManagementQuery {
    pub fn session_id(&self) -> Option<i64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn management_settings_keeps_flat_edit_options_and_false_values() {
        let request: ManagementCommand = serde_json::from_value(json!({
            "domain": "settings", "request": {
                "method": "set_layer", "params": {
                    "layer": "workspace",
                    "input": {"path": "session.max_turns", "value": 7,
                        "expected_revision": "r1", "dry_run": true,
                        "validate": false, "reload": false}
                }
            }
        }))
        .unwrap();
        let ManagementCommand::Settings(SettingsCommand::SetLayer { layer, input }) = &request
        else {
            panic!("settings command")
        };
        assert_eq!(*layer, ConfigSettingsLayer::Workspace);
        assert_eq!(input.options.expected_revision.as_deref(), Some("r1"));
        assert!(input.options.dry_run);
        assert!(!input.options.validate);
        assert!(!input.options.reload);
        let encoded = serde_json::to_value(request).unwrap();
        assert_eq!(encoded["request"]["params"]["input"]["reload"], false);
        assert!(
            encoded["request"]["params"]["input"]
                .get("options")
                .is_none()
        );
    }

    #[test]
    fn management_rejects_unknown_operations_and_wrong_typed_identifiers() {
        for input in [
            json!({"domain":"mcp","request":{"method":"unknown"}}),
            json!({"domain":"skills","request":{"method":"get","params":{"id":"test","workspace_id":"wrong"}}}),
            json!({"domain":"plugins","request":{"method":"logs","params":{"plugin_id":"test","after_seq":-1}}}),
        ] {
            assert!(serde_json::from_value::<ManagementQuery>(input).is_err());
        }
        let command: ManagementCommand = serde_json::from_value(json!({
            "domain":"permission_rules_batch", "request": {
                "rule_ids":[1,2], "scope":"global",
                "operation":{"action":"set_mode","mode":"ask"}
            }
        }))
        .unwrap();
        assert!(matches!(
            command,
            ManagementCommand::PermissionRulesBatch(PermissionRuleBatchRequest {
                operation: PermissionRuleBatchOperation::SetMode {
                    mode: crate::resource::PermissionMode::Ask
                },
                ..
            })
        ));
    }
}

impl ManagementCommand {
    pub fn session_id(&self) -> Option<i64> {
        match self {
            Self::Plugins(
                PluginCommand::InvokeTool { session_id, .. }
                | PluginCommand::InvokeCommand { session_id, .. }
                | PluginCommand::InvokeWebCommand { session_id, .. },
            ) => *session_id,
            Self::PermissionRulesBatch(request) => request.session_id,
            _ => None,
        }
    }
}
