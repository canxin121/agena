//! Session model: state, persistence, history, and execution services.

pub(crate) mod cost;
mod doom_loop;
mod manager;
pub use agena_runtime_session_core::model;
mod processor;
mod prompt_window;
mod store;
mod transcript;

pub(crate) use agena_runtime::ContextGovernor;
pub use agena_runtime_session_core::model::Session;
pub(crate) use agena_runtime_session_core::model::{
    PromptCompactionMessage, SessionRuntimeState, SubtaskRuntimeState,
};
pub use manager::{
    SessionManager, SessionSubtaskOutput, SessionSubtaskOutputChunk, SessionSubtaskRequest,
    SessionSubtaskResponse,
};
pub use processor::SessionProcessor;

pub(crate) type ExecutionControl = agena_runtime::ExecutionControl<crate::part::PartContent>;
pub(crate) type ExecutionRegistry = agena_runtime::ExecutionRegistry<crate::part::PartContent>;
pub(crate) use agena_runtime::ExecutionControlError;
