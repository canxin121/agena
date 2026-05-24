//! Convenience re-exports for plugin authors.
//!
//! ```ignore
//! use agena_plugin_sdk::prelude::*;
//! ```

pub use async_trait::async_trait;
pub use serde::{Deserialize, Serialize};
pub use serde_json::{Value, json};
pub use std::sync::Arc;

pub use crate::error::{PluginError, PluginErrorCode, Result};
pub use crate::hooks::*;
pub use crate::host_api::{
    EventSubscription, HostAgentDescriptor, HostAgentGetRequest, HostAgentGetResponse,
    HostAgentListResponse, HostAgentRegisterRequest, HostAgentRemoveRequest,
    HostAgentRemoveResponse, HostAgentRestoreRequest, HostAgentRestoreResponse,
    HostAgentSwitchRequest, HostAgentSwitchResponse, HostClient, HostEntryDescriptor,
    HostEntryListResponse, HostEntryMutationResponse, HostEntryRegisterRequest,
    HostEntryRemoveRequest, HostEntryUpdateRequest, HostGetSessionRequest, HostGetSessionResponse,
    HostHookEntry, HostHookListResponse, HostLspDiagnostic, HostLspListDiagnosticsRequest,
    HostLspListDiagnosticsResponse, HostLspListServersResponse, HostLspServer,
    HostMcpAddServerRequest, HostMcpListServersResponse, HostMcpRemoveServerRequest,
    HostMcpRemoveServerResponse, HostMcpServerSpec, HostNetworkPermissionCheckRequest,
    HostPathPermissionCheckRequest, HostPermissionCheckResponse, HostPlanEntry, HostPlanGetRequest,
    HostPlanGetResponse, HostPlanListResponse, HostPluginStatus, HostPluginStatusGetRequest,
    HostPluginStatusGetResponse, HostPluginStatusListResponse, HostRenameSessionRequest,
    HostRenameSessionResponse, HostSchedulerCreateRequest, HostSchedulerCreateResponse,
    HostSchedulerDeleteRequest, HostSchedulerDeleteResponse, HostSchedulerJob,
    HostSchedulerListResponse, HostSecretDeleteRequest, HostSecretGetRequest,
    HostSecretGetResponse, HostSecretListResponse, HostSecretSetRequest, HostSession,
    HostStatuslineContributeRequest, HostStatuslineListResponse, HostStatuslineRemoveRequest,
    HostStatuslineRemoveResponse, HostStatuslineSegment, HostStorageDeleteRequest,
    HostStorageEntry, HostStorageGetRequest, HostStorageGetResponse, HostStorageListRequest,
    HostStorageListResponse, HostStorageScope, HostStorageSetRequest, HostStorageVisibility,
    HostThemeListResponse, HostThemePalette, HostThemeRegisterRequest, HostThemeRemoveRequest,
    HostThemeRemoveResponse, HostWorktreeEntry, HostWorktreeListResponse, LogLevel, NoopHostClient,
};
pub use crate::manifest::{
    HookSubscription, HostCapability, InputNetworkSpec, InputPathSpec, NetworkAccessSpec,
    PathAccessSpec, PathKind, PluginManifest, PluginManifestBuilder, PluginStudioCommand,
    PluginStudioControl, PluginStudioControlOption, PluginStudioUiContributions, PluginStudioView,
    PluginToolDecl, PluginTuiContentBlock, PluginTuiStatuslineSegment, PluginTuiUiContributions,
    PluginUiAction, PluginUiContributions, PluginUiThemePalette, ToolDescriptionMode,
    ToolStreamingMode, ToolTag, TransportKind, normalize_tool_tag_name,
};
pub use crate::plugin::{InitContext, InitOutcome, Plugin, ToolStreamSink};

#[cfg(feature = "cdylib")]
pub use crate::cdylib_abi::{AgenaPluginCdylib, AgenaPluginCdylib_Ref};
