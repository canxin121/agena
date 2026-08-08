//! Provider adapter contract: client construction, auth, completion, and streaming.

pub mod auth;
pub mod credential;

mod catalog_decoration;
mod cataloged_models;
pub mod chat_wire;
pub mod core;
mod multi_adapter;
mod registry;
mod tool_mode;
pub mod utils;
pub mod wire_message;

pub use crate::provider_sse as sse;
pub use agena_provider::CompletionResponse;
pub use agena_provider::{
    self as copilot_models, self as prompt_cache, self as protocol_ids, self as tool_stream,
};
pub use utils::{
    ProviderRequestHeaderHook, install_request_header_hook, with_request_cancellation,
};

pub use agena_domain::{Model, ModelId, ModelSpeedMode, ModelThinkingMode};
pub use catalog_decoration::catalog_decoration_source;
pub use cataloged_models::CatalogedModelsProvider;
pub use core::ModelRuntime;
pub use credential::{ManagedCredential, parse_sap_ai_core_service_key, should_retry_credential};
pub use multi_adapter::MultiAdapterProvider;
pub use multi_adapter::ProviderModelRoute;
pub use registry::ProviderRegistry;
pub use wire_message::{
    WirePart as ProjectedSessionPart, project_completion_input, project_operation_output,
    project_operation_output as project_session_tool_result_output, project_persisted,
    project_persisted as project_session_parts, project_persisted_text_lossy,
    project_persisted_text_lossy as project_session_text_lossy,
};
