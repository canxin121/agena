//! Convenience re-exports for plugin authors.
//!
//! ```ignore
//! use agena_plugin_sdk::prelude::*;
//! ```

pub use async_trait::async_trait;
pub use schemars::JsonSchema;
pub use serde::{Deserialize, Serialize};
pub use serde_json::{Value, json};
pub use std::sync::Arc;

pub use crate::error::{PluginError, PluginErrorKind, Result};
pub use crate::hooks::*;
pub use crate::host_api::{
    EventSubscription, HostClient, HostDisplayContributeRequest, HostDisplayRemoveRequest,
    HostDisplayRemoveResponse, HostGetSessionRequest, HostGetSessionResponse, HostHookDescriptor,
    HostHookListResponse, HostHookRegistration, HostImageExecuteRequest, HostImageExecuteResponse,
    HostImageInput, HostImageOperation, HostLspDiagnostic, HostLspListDiagnosticsRequest,
    HostLspListDiagnosticsResponse, HostLspListServersResponse, HostLspServer,
    HostMcpAddServerRequest, HostMcpListServersResponse, HostMcpRemoveServerRequest,
    HostMcpRemoveServerResponse, HostMcpServerSpec, HostPluginStatus, HostPluginStatusGetRequest,
    HostPluginStatusGetResponse, HostPluginStatusListResponse, HostRegisteredToolDescriptor,
    HostRegisteredToolListResponse, HostRenameSessionRequest, HostRenameSessionResponse,
    HostSchedulerCreateRequest, HostSchedulerCreateResponse, HostSchedulerDeleteRequest,
    HostSchedulerDeleteResponse, HostSchedulerJob, HostSchedulerListResponse,
    HostSecretDeleteRequest, HostSecretGetRequest, HostSecretGetResponse, HostSecretListResponse,
    HostSecretSetRequest, HostSession, HostSnapshotListResponse, HostSnapshotSummary,
    HostStorageDeleteRequest, HostStorageGetRequest, HostStorageGetResponse,
    HostStorageListRequest, HostStorageListResponse, HostStorageRecord, HostStorageScope,
    HostStorageSetRequest, HostStorageVisibility, HostThemeListResponse, HostThemePalette,
    HostThemeRegisterRequest, HostThemeRemoveRequest, HostThemeRemoveResponse,
    HostToolMutationResponse, HostToolRegisterRequest, HostToolRemoveRequest,
    HostToolUpdateRequest, LogLevel, NoopHostClient, PluginNotifyAction, PluginNotifyActionTarget,
    PluginNotifyRequest, ToolRegistryChangeKind, ToolRegistryChangedEvent,
};
pub use crate::macro_support::{schema_example_texts, schema_usage_text};
pub use crate::manifest::{
    ContributionKind, HookSubscription, InputNetworkSpec, InputPathSpec, NetworkAccessSpec,
    PLUGIN_WORKBENCH_TAB_IDS, PathAccessSpec, PathKind, PluginCommandDefinition,
    PluginDisplayContent, PluginDisplayContribution, PluginManifest, PluginSkillDefinition,
    PluginStudioCommand, PluginStudioControl, PluginStudioControlOption,
    PluginStudioUiContributions, PluginStudioView, PluginTuiColor, PluginTuiThemeColors,
    PluginTuiUiContributions, PluginUiAction, PluginUiContributions, PluginUiThemePalette,
    ToolDefinition, ToolDescriptionMode, ToolDisplayPreset, ToolInput, ToolResultPolicy,
    ToolResultRenderKind, ToolStreamingMode, ToolTag, TransportKind, normalize_tool_tag_name,
    plugin_workbench_tab_id_is_supported,
};
pub use crate::plugin::{InitContext, InitOutcome, Plugin, PluginConfig, ToolStreamSink};
pub use agena_macros::{PluginConfigStore, ToolInput, agena_plugin};

#[cfg(feature = "cdylib")]
pub use crate::cdylib_abi::{AgenaPluginCdylib, AgenaPluginCdylib_Ref};
