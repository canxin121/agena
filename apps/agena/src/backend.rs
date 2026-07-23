use std::{
    collections::HashSet,
    collections::hash_map::DefaultHasher,
    env, fs,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use agena_api::{
    commands::{
        Command as ApiCommand, CommandResult, CompactSessionParams, ContinueRunParams,
        CreateSessionParams, ReplacePermissionRuleParams, ReplyPermissionParams,
        ReplyUserInputParams, RewindSessionParams, SubmitMessageParams, UpdateSessionParams,
        UpdateSessionSelectionParams, UpsertPermissionRuleParams,
    },
    pagination::PaginatedResponse,
    queries::{GetSessionParams, ListMessagesParams, ListSessionsParams, Query, QueryResult},
    resource::{
        MessageAttachment, MessageAttachmentKind, MessageAttachmentSource, MessageResource,
        PartLoadMode, PermissionReply, PermissionRuleResource, ProviderAdapterModelsResource,
        ProviderAdapterModelsResponse, ProviderAdapterSummaryResource, ProviderDefaultsResource,
        ProviderModelResource, ProviderSummaryResource, RunOptions, SessionExecutionResource,
        SessionResource, WorkspaceResource,
    },
};
use agena_application::dto::{CatalogModelResource, ConfigJsonSources, ModelCatalogListResponse};
use agena_application::{Application, dispatch};
use agena_domain::Model as ProviderModel;
use agena_domain::PermissionScope;
use agena_domain::ToolInvocation;
use agena_domain::{EventFilter, EventScope as Scope};
use agena_domain::{ModelRef, ProviderId};
use agena_domain::{PermissionReplyKind, UserInputReply};
use agena_plugin_sdk::{AttachmentItem, AttachmentKind, AttachmentSource};
use agena_provider::{
    AuthData, CatalogModelDefinition, CredentialIssuer, OpenAiResponsesBackendConfig,
    ProviderAdapterOverlay, ProviderCapabilityFamilyConfig, ProviderOverlay,
    provider_model_overlay_from_catalog_definition,
};
use agena_provider::{ProviderNativeToolRoute, ProviderNativeToolsConfig};
use agena_runtime::{
    ConfigSettingsEditResponse, ConfigSettingsPathInput, parse_oauth_callback_url,
};
use anyhow::{Result, anyhow};
use base64::engine::general_purpose::STANDARD;
use ignore::WalkBuilder;
use mime_guess::MimeGuess;
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
    provider_native_tools_config_for_preset, provider_native_tools_preset_from_config,
};
pub(crate) use self::backend_session::message_attachment_to_wire;
pub(crate) use self::backend_types::*;
use self::backend_util::*;

const MAX_ATTACHMENT_BYTES: usize = 50 * 1024 * 1024;

/// Push notification emitted by the unified bus for the active session.
/// Indicates whether the change requires reloading messages.
#[derive(Debug, Clone)]
pub struct LiveEvent {
    /// Concrete event payload when the subscriber kept up with the bus.
    /// `None` means the receiver lagged and the UI should force-refresh
    /// from persisted state instead of trying to apply an incremental patch.
    pub event: Option<agena_runtime::RuntimePresentationEvent>,
    /// True for events that materially change session state — the UI should
    /// trigger a `refresh_session` after handling.
    pub triggers_refresh: bool,
    /// True when the UI should ignore incremental assumptions and force a
    /// replay from persisted state (for example after bus lag).
    pub force_refresh: bool,
}

#[derive(Clone)]
pub struct Backend {
    application: Application,
    workspace_root: PathBuf,
    file_index: Arc<OnceLock<Vec<PathBuf>>>,
}
