//! Runtime orchestration over session-kernel capabilities. These projections
//! contain one recorded provider input and never a mutable full Session.

pub(crate) mod input;
pub(crate) mod media;
mod processor;
mod replay;

pub use input::{KernelPrompt, KernelPromptHydrator};
pub use processor::{KernelRoundOutcome, KernelRoundRequest, KernelRoundRun, KernelToolInvocation};
pub use replay::{KernelProviderState, reference_replay_text};

#[cfg(test)]
mod tests;
