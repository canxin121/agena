//! Convenience re-exports for plugin authors.
//!
//! ```ignore
//! use agena_plugin_sdk::prelude::*;
//! ```

pub use crate::cancellation::{CancellationToken, current_cancellation_token};
pub use async_trait::async_trait;
pub use schemars::JsonSchema;
pub use serde::{Deserialize, Serialize};
pub use serde_json::{Value, json};
pub use std::sync::Arc;

pub use crate::error::{PluginError, PluginErrorKind, Result};
pub use crate::hooks::*;
pub use crate::host_api::{
    EventSubscription, HostClient, HostClientExt, HostDisplayContributeRequest,
    HostDisplayRemoveRequest, HostDisplayRemoveResponse, HostGetSessionRequest,
    HostGetSessionResponse, HostHookDescriptor, HostHookListResponse, HostHookRegistration,
    HostImageExecuteRequest, HostImageExecuteResponse, HostImageInput, HostImageOperation,
    HostLspDiagnostic, HostLspListServersResponse, HostLspServer, HostMcpServerSpec,
    HostPluginStatus, HostPluginStatusGetRequest, HostPluginStatusGetResponse,
    HostPluginStatusListResponse, HostRegisteredToolDescriptor, HostRegisteredToolListResponse,
    HostRenameSessionRequest, HostRenameSessionResponse, HostSecretDeleteRequest,
    HostSecretGetRequest, HostSecretGetResponse, HostSecretListResponse, HostSecretSetRequest,
    HostSession, HostSessionStreamKind, HostSessionStreamRequest, HostStorageDeleteRequest,
    HostStorageGetRequest, HostStorageGetResponse, HostStorageListRequest, HostStorageListResponse,
    HostStorageRecord, HostStorageScope, HostStorageSetRequest, HostStorageVisibility,
    HostThemeListResponse, HostThemePalette, HostThemeRegisterRequest, HostThemeRemoveRequest,
    HostThemeRemoveResponse, HostToolMutationResponse, HostToolRegisterRequest,
    HostToolRemoveRequest, HostToolUpdateRequest, LogLevel, NoopHostClient, PathPermissionQuery,
    PermissionQuery, PluginNotifyAction, PluginNotifyActionTarget, PluginNotifyRequest,
    ToolRegistryChangeKind, ToolRegistryChangedEvent,
};
pub use crate::macro_support::{schema_example_texts, schema_usage_text, service_method_for};
pub use crate::manifest::{
    CommandDefinition, CommandDiagnostic, CommandHostEffect, CommandInvokeInput, CommandResult,
    CommandStatus, CommandTarget, ContributionKind, EmptyPluginSettings, HookHandlerDefinition,
    HookHandlerFilters, HookHandlerTranslation, HookSubscription, MAX_JSON_ESCAPE_BYTES,
    MAX_JSON_ESCAPE_DEPTH, PathInputKind, PathKind, PluginDisplayContent,
    PluginDisplayContribution, PluginManifest, PluginManifestTranslation,
    PluginServiceDeclarations, PluginServiceExport, PluginServiceImport, PluginServiceInvokeInput,
    PluginServiceInvokeOutput, PluginServiceMethod, PluginSkillDefinition,
    PluginSurfaceContributions, PluginTerminalColor, PluginTerminalContributions,
    PluginTerminalThemeColors, PluginTerminalThemePalette, SettingsConstraints, SettingsContract,
    SettingsNode, SettingsNodeKind, SettingsOption, SettingsVariant, ToolContract, ToolDefinition,
    ToolDocs, ToolDocsTranslation, ToolInput, ToolRuntimePolicy, ToolStreamingMode, ToolTag,
    TransportKind, normalize_tool_tag_name,
};
pub use crate::plugin::{InitContext, InitOutcome, Plugin, PluginSettings, ToolStreamSink};
pub use crate::service_client::{
    PluginServiceClient, PluginServiceEndpoint, PluginServiceEndpointClient,
    PluginServiceInvokeExt, PluginServiceResponse, encode_service_output,
};
pub use crate::settings_contract::{bounded_json_schema, decorate_settings_contract};
pub use agena_macros::{ToolInput, agena_plugin};

#[cfg(feature = "cdylib")]
pub use crate::cdylib_abi::{AgenaPluginCdylib, AgenaPluginCdylib_Ref};
