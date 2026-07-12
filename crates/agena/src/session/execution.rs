//! Typed execution lifecycle for a session.
//!
//! Workflow readiness, process liveness, and message construction are
//! deliberately different state machines. This module owns process liveness:
//! exactly one `ExecutionStarted` must be followed by exactly one
//! `ExecutionFinished`.

use serde::{Deserialize, Serialize};

use super::ExecutionId;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionSource {
    #[default]
    User,
    Continue,
    Compaction,
    PermissionReply,
    UserInputReply,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionPhase {
    Starting,
    PreparingModel,
    StreamingModel,
    ExecutingTools,
    Cancelling,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionFailureKind {
    Provider,
    Internal,
    ProcessRestart,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExecutionOutcome {
    Completed,
    Cancelled,
    Failed {
        failure_kind: ExecutionFailureKind,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionLifecycle {
    Active {
        execution_id: ExecutionId,
        phase: ExecutionPhase,
    },
    Terminal {
        execution_id: ExecutionId,
        outcome: ExecutionOutcome,
    },
}

impl ExecutionLifecycle {
    pub fn start(execution_id: ExecutionId) -> Self {
        Self::Active {
            execution_id,
            phase: ExecutionPhase::Starting,
        }
    }

    pub fn execution_id(&self) -> ExecutionId {
        match self {
            Self::Active { execution_id, .. } | Self::Terminal { execution_id, .. } => {
                *execution_id
            }
        }
    }

    pub fn transition(&mut self, phase: ExecutionPhase) -> Result<(), ExecutionTransitionError> {
        let Self::Active {
            execution_id,
            phase: current,
        } = self
        else {
            return Err(ExecutionTransitionError::AlreadyTerminal);
        };
        if !phase_transition_allowed(*current, phase) {
            return Err(ExecutionTransitionError::InvalidPhase {
                from: *current,
                to: phase,
            });
        }
        *self = Self::Active {
            execution_id: *execution_id,
            phase,
        };
        Ok(())
    }

    pub fn finish(&mut self, outcome: ExecutionOutcome) -> Result<(), ExecutionTransitionError> {
        let Self::Active { execution_id, .. } = self else {
            return Err(ExecutionTransitionError::AlreadyTerminal);
        };
        *self = Self::Terminal {
            execution_id: *execution_id,
            outcome,
        };
        Ok(())
    }
}

fn phase_transition_allowed(from: ExecutionPhase, to: ExecutionPhase) -> bool {
    from == to
        || matches!(
            (from, to),
            (ExecutionPhase::Starting, ExecutionPhase::PreparingModel)
                | (ExecutionPhase::Starting, ExecutionPhase::ExecutingTools)
                | (ExecutionPhase::Starting, ExecutionPhase::Cancelling)
                | (
                    ExecutionPhase::PreparingModel,
                    ExecutionPhase::StreamingModel
                )
                | (ExecutionPhase::PreparingModel, ExecutionPhase::Cancelling)
                | (
                    ExecutionPhase::StreamingModel,
                    ExecutionPhase::ExecutingTools
                )
                | (
                    ExecutionPhase::StreamingModel,
                    ExecutionPhase::PreparingModel
                )
                | (ExecutionPhase::StreamingModel, ExecutionPhase::Cancelling)
                | (
                    ExecutionPhase::ExecutingTools,
                    ExecutionPhase::PreparingModel
                )
                | (ExecutionPhase::ExecutingTools, ExecutionPhase::Cancelling)
        )
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ExecutionTransitionError {
    #[error("execution is already terminal")]
    AlreadyTerminal,
    #[error("invalid execution phase transition: {from:?} -> {to:?}")]
    InvalidPhase {
        from: ExecutionPhase,
        to: ExecutionPhase,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_execution_cannot_be_reopened_or_finished_twice() {
        let mut execution = ExecutionLifecycle::start(ExecutionId::new());
        execution
            .finish(ExecutionOutcome::Cancelled)
            .expect("first terminal transition");
        assert_eq!(
            execution.transition(ExecutionPhase::Starting),
            Err(ExecutionTransitionError::AlreadyTerminal)
        );
        assert_eq!(
            execution.finish(ExecutionOutcome::Completed),
            Err(ExecutionTransitionError::AlreadyTerminal)
        );
    }

    #[test]
    fn cancelling_is_reachable_from_every_active_work_phase() {
        for phase in [
            ExecutionPhase::Starting,
            ExecutionPhase::PreparingModel,
            ExecutionPhase::StreamingModel,
            ExecutionPhase::ExecutingTools,
        ] {
            assert!(phase_transition_allowed(phase, ExecutionPhase::Cancelling));
        }
    }

    #[test]
    fn tool_first_and_multi_round_execution_paths_are_valid() {
        let mut tool_first = ExecutionLifecycle::start(ExecutionId::new());
        tool_first
            .transition(ExecutionPhase::ExecutingTools)
            .expect("resume pending tools");
        tool_first
            .transition(ExecutionPhase::PreparingModel)
            .expect("prepare after tools");
        tool_first
            .transition(ExecutionPhase::StreamingModel)
            .expect("stream model");
        tool_first
            .transition(ExecutionPhase::ExecutingTools)
            .expect("execute emitted tools");
        tool_first
            .transition(ExecutionPhase::PreparingModel)
            .expect("prepare next model round");
        tool_first
            .transition(ExecutionPhase::StreamingModel)
            .expect("stream next model round");
        tool_first
            .finish(ExecutionOutcome::Completed)
            .expect("finish execution");
    }
}
