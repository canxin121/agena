//! Typed execution lifecycle state machine.
//!
//! This value is intentionally independent of session storage and execution
//! orchestration. The session layer owns when transitions are requested; the
//! domain layer owns which transitions are valid.

use crate::{ExecutionId, ExecutionOutcome, ExecutionPhase};

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
    fn invalid_phase_transitions_are_rejected() {
        let mut execution = ExecutionLifecycle::start(ExecutionId::new());
        assert_eq!(
            execution.transition(ExecutionPhase::StreamingModel),
            Err(ExecutionTransitionError::InvalidPhase {
                from: ExecutionPhase::Starting,
                to: ExecutionPhase::StreamingModel,
            })
        );
    }
}
