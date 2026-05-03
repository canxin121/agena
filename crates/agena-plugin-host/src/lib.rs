//! Host-side runtime for agena plugins. Loads them from config, dispatches
//! hooks, multiplexes plugin → host callbacks.

pub mod config;
pub mod dispatcher;
pub mod error;
pub mod host;
pub mod loader;
pub mod manifest_io;
pub mod registry;
pub mod status;
pub mod transport;

pub use agena_plugin_sdk as sdk;
pub use config::{
    HttpAuth, PluginEntry, PluginSignature, PluginsConfig, RestartPolicy, TimeoutsConfig,
};
pub use error::{HostError, TransportError};
pub use host::{
    LoadedPlugin, PluginEntryHandle, PluginEntryResolution, PluginHost, PluginHostBuilder,
    ToolInvokeStream,
};
#[cfg(feature = "signing")]
pub use loader::{verify_sha256, verify_signature, verify_signature_bytes};
pub use registry::PluginEntryRegistry;
pub use sdk::{
    AgentStopInput, AgentStopPatch, AuthInput, AuthOutput, ChatDirection, ChatHeadersInput,
    ChatHeadersPatch, ChatMessage, ChatMessageInput, ChatMessagePatch, ChatMessagesTransformInput,
    ChatMessagesTransformPatch, ChatParamsInput, ChatParamsPatch, ChatSystemTransformInput,
    ChatSystemTransformPatch, CommandAfterInput, CommandAfterPatch, CommandBeforeInput,
    CommandBeforeOutcome, CommandBeforePatch, CommandBeforeResponse, ConfigInput, ConfigPatch,
    EntryBehavior, EntryDefinitionInput, EntryDefinitionPatch, EntrySource, EventEnvelope,
    EventFilter, HookSubscription, PermissionAskDecision, PermissionAskInput, PermissionDecision,
    PluginEntryDecl, PluginError, PluginManifest, PostTurnInput, PreTurnInput, ProviderDescriptor,
    ProviderKind, ProviderListInput, ProviderListPatch, SessionCompactedInput,
    SessionCompactingInput, SessionCompactingPatch, SessionEndInput, SessionEndReason,
    SessionStartInput, SessionStartPatch, SessionStartSource, ShellEnvInput, ShellEnvPatch,
    ToolAfterInput, ToolAfterPatch, ToolBeforeInput, ToolBeforePatch, ToolFailureInput,
    ToolInvokeInput, ToolInvokeOutput, ToolPermissionPathsInput, UserPromptSubmitInput,
    UserPromptSubmitPatch,
};
