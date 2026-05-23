//! Host-side runtime for agena plugins. Loads them from config, dispatches
//! hooks, multiplexes plugin → host callbacks.

pub mod config;
pub mod dispatcher;
pub mod error;
pub mod host;
pub mod loader;
pub mod logs;
pub mod manifest_io;
pub mod quota;
pub mod registry;
pub mod status;
pub mod transport;

pub use agena_plugin_sdk as sdk;
pub use config::{
    HttpAuth, PluginEntry, PluginSignature, PluginsConfig, RestartPolicy, TimeoutsConfig,
    ToolPresentationConfig,
};
pub use error::{HostError, TransportError};
pub use host::{
    LoadedPlugin, PluginHost, PluginHostBuilder, PluginInspect, PluginStudioCommandCatalogItem,
    PluginStudioControlCatalogItem, PluginStudioUiCatalog, PluginStudioViewCatalogItem,
    PluginTuiContentBlockCatalogItem, PluginTuiUiCatalog, PluginUiCatalog,
    PluginUiToolInvokeResponse, ToolInvokeStream,
};
#[cfg(feature = "signing")]
pub use loader::{verify_sha256, verify_signature, verify_signature_bytes};
pub use logs::{PluginLogEntry, PluginLogStore};
pub use registry::PluginEntryRegistry;
pub use sdk::host_api::{
    HostNetworkPermissionCheckRequest, HostPathPermissionCheckRequest, HostPermissionCheckResponse,
    HostStatuslineSegment, HostThemePalette,
};
pub use sdk::{
    AgentStopInput, AgentStopPatch, AuthInput, AuthOutput, ChatDirection, ChatHeadersInput,
    ChatHeadersPatch, ChatMessage, ChatMessageInput, ChatMessagePatch, ChatMessagesTransformInput,
    ChatMessagesTransformPatch, ChatParamsInput, ChatParamsPatch, ChatSystemTransformInput,
    ChatSystemTransformPatch, CommandAfterInput, CommandAfterPatch, CommandBeforeInput,
    CommandBeforeOutcome, CommandBeforePatch, CommandBeforeResponse, ConfigInput, ConfigPatch,
    EventEnvelope, EventFilter, HookSubscription, NotificationInput, PermissionAskDecision,
    PermissionAskInput, PermissionDecision, PluginError, PluginManifest, PluginStudioCommand,
    PluginStudioControl, PluginStudioControlOption, PluginStudioUiContributions, PluginStudioView,
    PluginToolDecl, PluginTuiContentBlock, PluginTuiStatuslineSegment, PluginTuiUiContributions,
    PluginUiAction, PluginUiContributions, PluginUiThemePalette, PostRunInput, PreRunInput,
    ProviderDescriptor, ProviderKind, ProviderListInput, ProviderListPatch, SessionEndInput,
    SessionEndReason, SessionStartInput, SessionStartPatch, SessionStartSource, ShellEnvInput,
    ShellEnvPatch, ToolAfterInput, ToolAfterPatch, ToolBeforeInput, ToolBeforePatch,
    ToolDefinitionInput, ToolDefinitionPatch, ToolDescriptionMode, ToolFailureInput,
    ToolInvokeInput, ToolInvokeOutput, ToolPermissionNetworksInput, ToolPermissionPathsInput,
    UserPromptSubmitInput, UserPromptSubmitPatch,
};
