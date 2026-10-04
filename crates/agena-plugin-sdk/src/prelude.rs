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
    HostSecretSetRequest, HostSession, HostStorageDeleteRequest, HostStorageGetRequest,
    HostStorageGetResponse, HostStorageListRequest, HostStorageListResponse, HostStorageRecord,
    HostStorageScope, HostStorageSetRequest, HostStorageVisibility, HostThemeListResponse,
    HostThemePalette, HostThemeRegisterRequest, HostThemeRemoveRequest, HostThemeRemoveResponse,
    HostToolMutationResponse, HostToolRegisterRequest, HostToolRemoveRequest,
    HostToolUpdateRequest, LogLevel, NoopHostClient, PathPermissionQuery, PermissionQuery,
    PluginNotifyAction, PluginNotifyActionTarget, PluginNotifyRequest, ToolRegistryChangeKind,
    ToolRegistryChangedEvent,
};
pub use crate::macro_support::{schema_example_texts, schema_usage_text, service_method_for};
pub use crate::manifest::{
    CommandDefinition, CommandDiagnostic, CommandHostEffect, CommandInvokeInput, CommandResult,
    CommandStatus, CommandTarget, ContributionKind, EmptyPluginSettings, HookSubscription,
    MAX_JSON_ESCAPE_BYTES, MAX_JSON_ESCAPE_DEPTH, PathInputKind, PathKind, PluginDisplayContent,
    PluginDisplayContribution, PluginManifest, PluginServiceDeclarations, PluginServiceExport,
    PluginServiceImport, PluginServiceInvokeInput, PluginServiceInvokeOutput, PluginServiceMethod,
    PluginSkillDefinition, PluginSurfaceContributions, PluginTerminalColor,
    PluginTerminalContributions, PluginTerminalThemeColors, PluginTerminalThemePalette,
    SettingsConstraints, SettingsContract, SettingsNode, SettingsNodeKind, SettingsOption,
    SettingsVariant, ToolContract, ToolDefinition, ToolDocs, ToolInput, ToolRuntimePolicy,
    ToolStreamingMode, ToolTag, TransportKind, normalize_tool_tag_name,
};
pub use crate::plugin::{InitContext, InitOutcome, Plugin, PluginSettings, ToolStreamSink};
pub use crate::service_client::{
    PluginServiceClient, PluginServiceEndpoint, PluginServiceEndpointClient,
    PluginServiceInvokeExt, PluginServiceResponse, encode_service_output,
};
pub use crate::settings_contract::{bounded_json_schema, decorate_settings_contract};
pub use agena_macros::{PluginSettingsStore, ToolInput, agena_plugin};

#[cfg(feature = "cdylib")]
pub use crate::cdylib_abi::{AgenaPluginCdylib, AgenaPluginCdylib_Ref};
