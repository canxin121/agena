use std::{
    collections::HashSet,
    collections::hash_map::DefaultHasher,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, OnceLock},
};

use crate::short_link::shorten_url_for_display;
use agena::event::{EventFilter, Scope, bus::SubscriptionItem};
use agena::permission::PermissionScope;
use agena::{
    agents::AgentDescriptor,
    config::{
        ConfigSettingsDeleteInput, ConfigSettingsEditOptions, ConfigSettingsEditResponse,
        ConfigSettingsGetInput, ConfigSettingsPatchInput, ConfigSettingsPathInput,
        ConfigSettingsSetInput, OpenAiResponsesBackendConfig, ProcessEnvironment,
        ProviderAdapterDefinition, ProviderAdapterOverlay, ProviderApiSubtype, ProviderAuthConfig,
        ProviderAuthMode, ProviderAuthOverlay, ProviderCapabilityFamilyConfig,
        ProviderModelOverlay, ProviderOverlay, ProviderSecretSourceConfig,
        ProviderSecretSourceOverlay, ProviderToolRoute, ProviderToolsConfig, delete_file_setting,
        draft_bedrock_sigv4_provider_adapter_models_target,
        draft_cline_api_provider_adapter_models_target,
        draft_credential_provider_adapter_models_target,
        draft_gitlab_provider_adapter_models_target, draft_none_provider_adapter_models_target,
        draft_provider_adapter_models_target, list_provider_adapter_models_with_config,
        parse_settings_path, patch_file_settings, provider_model_overlay_from_catalog_definition,
        read_file_setting, saved_provider_adapter_models_target, set_file_setting,
    },
    event::{DomainEvent, EventKind},
    memory::MemoryStore,
    message::{
        AttachmentItem, AttachmentKind, AttachmentSource, EnterSnapshotToolInput,
        ExitSnapshotToolInput, PartContent, ToolInvocation, UserInputReply,
    },
    model::{ModelCapabilities, ModelId, ModelMetadata, ModelRef, ProviderId},
    model_catalog::{
        CatalogModelDefinition, ModelCatalogProviderRecord, catalog_definition_from_model,
        decorate_provider_models,
    },
    permission::PermissionReplyKind,
    provider::ProviderModel,
    provider::auth::{
        AuthData, CredentialIssuer, exchange_gitlab_oauth_code, exchange_openai_oauth_code,
        parse_oauth_callback_url, poll_copilot_device_code, poll_openai_headless_device_code,
        start_copilot_device_code, start_gitlab_oauth, start_openai_browser_oauth,
        start_openai_headless_device_code,
    },
    runtime::AgenaRuntime,
    tool,
};
use agena_api::{
    commands::{
        Command as ApiCommand, CommandResult, CompactSessionParams, ContinueRunParams,
        CreateSessionParams, ReplacePermissionRuleParams, ReplyPermissionParams,
        ReplyUserInputParams, RewindSessionParams, SubmitMessageParams, UpdateSessionParams,
        UpdateSessionSelectionParams, UpsertPermissionRuleParams,
    },
    pagination::PaginatedResponse,
    queries::{
        GetSessionParams, ListMessagesParams, ListPermissionRulesParams, ListSessionsParams, Query,
        QueryResult,
    },
    resource::{
        MessageResource, PartLoadMode, PermissionReply, PermissionRuleResource,
        ProviderAdapterModelsResource, ProviderAdapterModelsResponse,
        ProviderAdapterSummaryResource, ProviderDefaultsResource, ProviderSummaryResource,
        ProviderToolBindingResource, ProviderToolsSummaryResource, RunOptions,
        SessionExecutionResource, SessionResource, WorkspaceResource,
    },
};
use agena_api_server::{
    dispatch,
    local_api::{
        CatalogModelResource, ModelCatalogListResponse,
        ModelCatalogResponse as LocalModelCatalogResponse, ModelCatalogSourceKind, normalize_limit,
    },
    state::AppState,
};
use anyhow::{Result, anyhow};
use base64::engine::general_purpose::STANDARD;
use ignore::WalkBuilder;
use mime_guess::MimeGuess;
use sea_orm::DatabaseConnection;
use serde_json::{Map as JsonMap, Value as JsonValue};
use tokio::sync::mpsc;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct SnapshotCommandOutput {
    #[serde(default)]
    pub action: Option<String>,
    pub path: String,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

mod backend_auth;
mod backend_catalog;
mod backend_config;
mod backend_drafts;
mod backend_events;
mod backend_helpers;
mod backend_plugins;
mod backend_provider;
mod backend_session;
mod backend_types;
mod backend_util;
mod backend_workspace;

use self::backend_auth::*;
use self::backend_catalog::*;
use self::backend_config::*;
pub(crate) use self::backend_drafts::*;
use self::backend_events::*;
use self::backend_helpers::*;
pub(crate) use self::backend_helpers::{
    provider_tools_config_for_preset, provider_tools_preset_from_config,
    provider_tools_suggested_preset_for_draft,
};
pub(crate) use self::backend_types::*;
use self::backend_util::*;

const MAX_ATTACHMENT_BYTES: usize = 50 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ConfigJsonSources {
    pub config_path: PathBuf,
    pub config_found: bool,
    pub project_config_path: PathBuf,
    pub project_config_found: bool,
    pub applied_layers: Vec<String>,
    pub file: JsonValue,
    pub project_file: JsonValue,
    pub effective: JsonValue,
}

/// Push notification emitted by the unified bus for the active session.
/// Indicates whether the change requires reloading messages.
#[derive(Debug, Clone)]
pub struct LiveEvent {
    /// Concrete event payload when the subscriber kept up with the bus.
    /// `None` means the receiver lagged and the UI should force-refresh
    /// from persisted state instead of trying to apply an incremental patch.
    pub event: Option<DomainEvent>,
    /// True for events that materially change session state — the UI should
    /// trigger a `refresh_session` after handling.
    pub triggers_refresh: bool,
    /// True when the UI should ignore incremental assumptions and force a
    /// replay from persisted state (for example after bus lag).
    pub force_refresh: bool,
}

#[derive(Clone)]
pub struct Backend {
    runtime: AgenaRuntime,
    app_state: AppState,
    workspace_root: PathBuf,
    file_index: Arc<OnceLock<Vec<PathBuf>>>,
}
