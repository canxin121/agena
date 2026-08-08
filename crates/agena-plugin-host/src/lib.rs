//! # agena-plugin-host
//!
//! Host-side runtime for Agena plugins.
//!
//! Loads plugins from configuration, verifies and transports their artifacts
//! (in-process, cdylib, stdio, HTTP), dispatches hooks, multiplexes
//! plugin→host callbacks, enforces quotas, and records logs and status.
//!
//! ## Key items
//!
//! - [`HostError`] / [`TransportError`] — host and transport failures.
//! - [`PluginToolRegistry`] — registry of tools published by loaded plugins.
//! - [`sdk`] — re-export of the [`agena_plugin_sdk`] types used by the host.
//! - [`HostThemePalette`] — theme palette provided to plugins by the host.
//!
//! The `host`, `loader`, `dispatcher`, `transport`, `quota`, `logs`, and
//! `status` modules hold the concrete implementations.

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
    ConfiguredPlugin, HttpAuth, PluginHostConfig, PluginPackage, PluginPolicyConfig,
    PluginSignature, PluginsConfig, RestartPolicy, TimeoutsConfig, ToolDescriptionOverride,
    ToolPresentationConfig, UiPresentationConfig, UiPresentationOverride, UiTextDisplayMode,
};
pub use error::{HostError, TransportError};
pub use host::{
    AgentStopDispatch, AgentStopHookRun, HookRunRecord, HookRunStatus, HostDisplayContribution,
    HostNotification, LoadedPlugin,
    PluginCommandCatalogItem, PluginHost, PluginHostBuildConfig, PluginInspect,
    PluginStudioControlCatalogItem, PluginStudioUiCatalog, PluginStudioViewCatalogItem,
    PluginTuiUiCatalog, PluginUiCatalog, PluginUiToolInvokeResponse, PluginUiToolInvokeStatus,
    StaticPluginRegistration, ToolInvokeStream,
};
#[cfg(feature = "signing")]
pub use loader::{verify_sha256, verify_signature, verify_signature_bytes};
pub use logs::{PluginLogRecord, PluginLogStore};
pub use registry::PluginToolRegistry;
pub use sdk::host_api::HostThemePalette;
pub use sdk::{
    AgentStopInput, AgentStopPatch, AuthInput, AuthOutput, ChatDirection, ChatHeadersInput,
    ChatHeadersPatch, ChatMessage, ChatMessageInput, ChatMessagePatch, ChatMessagesTransformInput,
    ChatMessagesTransformPatch, ChatParamsInput, ChatParamsPatch, ChatSystemTransformInput,
    ChatSystemTransformPatch, CommandAfterInput, CommandAfterPatch, CommandBeforeInput,
    CommandBeforeOutcome, CommandBeforePatch, CommandBeforeResponse, ConfigInput, ConfigPatch,
    ContributionKind, EventEnvelope, EventFilter, HookSubscription, NotificationInput,
    PluginCommandDefinition, PluginCommandInvokeInput, PluginCommandOutput, PluginDisplayContent,
    PluginDisplayContribution, PluginError, PluginKey, PluginKeyParseError, PluginManifest,
    PluginNotifyAction, PluginNotifyActionTarget, PluginNotifyRequest, PluginStudioControl,
    PluginStudioControlOption, PluginStudioUiContributions, PluginStudioView,
    PluginTuiUiContributions, PluginUiAction, PluginUiContributions, PluginUiThemePalette,
    PostRunInput, PreRunInput, ProviderDescriptor, ProviderKind, ProviderListInput,
    ProviderListPatch, SessionEndInput, SessionEndReason, SessionStartInput, SessionStartPatch,
    SessionStartSource, ShellEnvInput, ShellEnvPatch, ToolAfterInput, ToolAfterPatch,
    ToolBeforeInput, ToolBeforePatch, ToolDefinition, ToolDefinitionInput, ToolDefinitionPatch,
    ToolDescriptionMode, ToolFailureInput, ToolInvokeInput, ToolInvokeOutput, ToolKey,
    ToolKeyParseError, ToolPermissionNetworksInput, ToolPermissionPathsInput,
    UserPromptSubmitInput, UserPromptSubmitPatch,
};
