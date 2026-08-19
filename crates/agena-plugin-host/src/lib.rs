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

pub mod activation;
pub mod config;
pub mod dispatcher;
pub mod effect_scope;
pub mod error;
pub mod event_pipeline;
pub mod host;
pub mod loader;
pub mod logs;
pub mod manifest_io;
pub mod profiles;
pub mod quota;
pub mod registry;
pub mod scoped_registry;
pub mod services;
pub mod status;
pub mod transport;

pub use activation::{
    PluginActivationBlock, PluginActivationPlan, PluginReloadAction, PluginReloadDecision,
    PluginReloadPlan, PluginReloadReason, plan_plugin_activation, plan_plugin_reload,
    plugin_activation_epochs,
};
pub use agena_plugin_sdk as sdk;
pub use config::{
    ConfiguredPlugin, HttpAuth, PluginActivationConfig, PluginHostConfig, PluginPackage,
    PluginPolicyConfig, PluginSignature, PluginsConfig, RestartPolicy, TimeoutsConfig,
};
pub use effect_scope::{
    PluginEffectDescriptor, PluginEffectDisposeReport, PluginEffectHandle, PluginEffectScope,
    PluginEffectScopeInspect, PluginEffectScopeState, PluginEffectState,
};
pub use error::{HostError, TransportError};
pub use event_pipeline::{
    PluginAroundNext, PluginAroundPipeline, PluginBailPipeline, PluginBailReport,
    PluginEventDefinition, PluginEventMode, PluginGuardDecision, PluginGuardErrorPolicy,
    PluginGuardPipeline, PluginGuardReport, PluginObservePipeline, PluginObserveReport,
    PluginPipelineError, PluginPipelineFailure, PluginPipelineFailurePolicy,
    PluginPipelineHandlerDescriptor, PluginPipelineRegistration, PluginTransformBailControl,
    PluginTransformBailOutcome, PluginTransformBailPipeline, PluginTransformBailReport,
    PluginTransformPipeline, PluginTransformReport,
};
pub use host::{
    AgentStopDispatch, AgentStopHookRun, HookRunRecord, HookRunStatus, HostDisplayContribution,
    HostNotification, LoadedPlugin, PluginActivationDiagnostic, PluginActivationInspect,
    PluginArchitectureCatalog, PluginArchitectureEffect, PluginArchitectureNode,
    PluginArchitecturePipeline, PluginDependencyEdge, PluginDependencyKind, PluginHost,
    PluginHostBuildConfig, PluginInspect, PluginOperationCatalogItem, PluginServiceImportInspect,
    PluginServiceInspect, PluginSurfaceCatalog, PluginTerminalSurfaceCatalog,
    PluginToolInvokeResponse, PluginToolInvokeStatus, StaticPluginRegistration, ToolInvokeStream,
};
#[cfg(feature = "signing")]
pub use loader::{verify_sha256, verify_signature, verify_signature_bytes};
pub use logs::{PluginLogRecord, PluginLogStore};

pub use profiles::{
    PluginProfile, PluginProfileAction, PluginProfileChange, PluginProfileEntry,
    PluginProfileResolution, PluginProfileResolutionMeta, apply_json_merge_patch,
    resolve_plugin_profiles, validate_profiles,
};
pub use registry::PluginToolRegistry;
pub use scoped_registry::{
    PluginScopeKey, ScopedRegistry, ScopedRegistryEntryDescriptor, ScopedRegistryError,
    ScopedRegistryLayer, ScopedRegistryRegistration, ScopedRegistryValue,
};
pub use sdk::host_api::HostThemePalette;
pub use sdk::{
    AgentCancelInput, AgentStopInput, AgentStopPatch, AuthInput, AuthOutput, ChatDirection,
    ChatHeadersInput, ChatHeadersPatch, ChatMessage, ChatMessageInput, ChatMessagePatch,
    ChatMessagesTransformInput, ChatMessagesTransformPatch, ChatParamsInput, ChatParamsPatch,
    ChatSystemTransformInput, ChatSystemTransformPatch, CommandAfterInput, CommandAfterPatch,
    CommandBeforeInput, CommandBeforeOutcome, CommandBeforePatch, CommandBeforeResponse,
    ConfigInput, ConfigPatch, ContributionKind, EventEnvelope, EventFilter, HookSubscription,
    NotificationInput, PluginDisplayContent, PluginDisplayContribution, PluginError, PluginKey,
    PluginKeyParseError, PluginManifest, PluginNotifyAction, PluginNotifyActionTarget,
    PluginNotifyRequest, PluginOperationDefinition, PluginOperationInvokeInput,
    PluginOperationResult, PluginServiceDeclarations, PluginServiceExport, PluginServiceImport,
    PluginServiceInvokeInput, PluginServiceInvokeOutput, PluginServiceMethod, PostRunInput,
    PreRunInput, ProviderDescriptor, ProviderKind, ProviderListInput, ProviderListPatch,
    SessionEndInput, SessionEndReason, SessionStartInput, SessionStartPatch, SessionStartSource,
    ShellEnvInput, ShellEnvPatch, ToolAfterInput, ToolAfterPatch, ToolBeforeInput, ToolBeforePatch,
    ToolDefinition, ToolDefinitionInput, ToolDefinitionPatch, ToolFailureInput, ToolInvokeInput,
    ToolInvokeOutput, ToolKey, ToolKeyParseError, ToolPermissionNetworksInput,
    ToolPermissionPathsInput, UserPromptSubmitInput, UserPromptSubmitPatch,
};
pub use services::{
    PluginServiceBinding, PluginServiceBindingKey, PluginServiceResolutionBlock,
    PluginServiceResolutionPlan, resolve_plugin_services,
};
