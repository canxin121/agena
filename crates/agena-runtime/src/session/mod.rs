mod cache;
pub(crate) mod cost;
mod doom_loop;
pub(crate) mod history;
mod manager;
mod model;
mod processor;
mod prompt_window;
mod store;

pub(crate) use agena_runtime::ContextGovernor;
pub(crate) use manager::{SessionManager, SessionSubtaskRequest};
pub(crate) use model::{
    PromptCompactionContent, PromptCompactionMessage, PromptWindowRuntime, Session,
    SessionExecutionContext, SessionRuntimeState, SubtaskRuntimeState,
};
pub(crate) use processor::SessionProcessor;

pub(crate) type ExecutionControl = agena_runtime::ExecutionControl<crate::message::PartContent>;
pub(crate) type ExecutionRegistry = agena_runtime::ExecutionRegistry<crate::message::PartContent>;
pub(crate) use agena_runtime::ExecutionControlError;

pub(crate) use history::ProjectedMessageHeader;
