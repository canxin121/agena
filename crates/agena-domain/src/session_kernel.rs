//! Facts and independent version domains of the session kernel.
//!
//! These values describe committed facts. They confer no write authority;
//! attempt capabilities are issued by storage after checking current ownership.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("kernel identity/version must be positive and fit a SQLite integer")]
pub struct InvalidKernelNumber;

macro_rules! positive_number {
    ($($name:ident),+ $(,)?) => {$ (
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "i64", into = "i64")]
        pub struct $name(i64);

        impl $name {
            pub const fn get(self) -> i64 { self.0 }
        }
        impl TryFrom<i64> for $name {
            type Error = InvalidKernelNumber;
            fn try_from(value: i64) -> Result<Self, Self::Error> {
                if value > 0 { Ok(Self(value)) } else { Err(InvalidKernelNumber) }
            }
        }
        impl From<$name> for i64 {
            fn from(value: $name) -> Self { value.0 }
        }
    )+};
}

positive_number!(
    SessionId,
    EntryId,
    SegmentId,
    RoundId,
    ToolId,
    InteractionId
);
positive_number!(
    AttemptGeneration,
    EntryRevision,
    MetadataVersion,
    MembershipGeneration,
    CommitSequence
);

macro_rules! version_number {
    ($($name:ident),+ $(,)?) => {$ (
        impl $name {
            pub const INITIAL: Self = Self(1);
            pub fn next(self) -> Result<Self, InvalidKernelNumber> {
                self.0.checked_add(1).map(Self).ok_or(InvalidKernelNumber)
            }
        }
    )+};
}
version_number!(
    AttemptGeneration,
    EntryRevision,
    MetadataVersion,
    MembershipGeneration,
    CommitSequence
);

/// One actual transaction sequence plus its command ordinal. Idempotent
/// replays retain this position even after a new server epoch starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CommitPosition {
    pub sequence: CommitSequence,
    pub ordinal: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionPhase {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
}

impl ExecutionPhase {
    pub const fn is_terminal(self) -> bool {
        !matches!(self, Self::Queued | Self::Running)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionEffect {
    Changed,
    Unchanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TransitionError {
    #[error("attempt generation has been superseded")]
    StaleAttempt,
    #[error("execution is not in a state which permits this operation")]
    Conflict,
    #[error(transparent)]
    Exhausted(#[from] InvalidKernelNumber),
}

/// The current generation and the execution's one authoritative decision.
/// Historical attempt outcomes are separately immutable records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionState {
    generation: AttemptGeneration,
    phase: ExecutionPhase,
}

impl Default for ExecutionState {
    fn default() -> Self {
        Self {
            generation: AttemptGeneration::INITIAL,
            phase: ExecutionPhase::Queued,
        }
    }
}

impl ExecutionState {
    pub fn from_stored(generation: AttemptGeneration, phase: ExecutionPhase) -> Self {
        Self { generation, phase }
    }
    pub fn generation(self) -> AttemptGeneration {
        self.generation
    }
    pub fn phase(self) -> ExecutionPhase {
        self.phase
    }

    pub fn check_running(self, generation: AttemptGeneration) -> Result<(), TransitionError> {
        if generation != self.generation {
            return Err(TransitionError::StaleAttempt);
        }
        if self.phase != ExecutionPhase::Running {
            return Err(TransitionError::Conflict);
        }
        Ok(())
    }

    pub fn claim(&mut self) -> Result<(), TransitionError> {
        if self.phase != ExecutionPhase::Queued {
            return Err(TransitionError::Conflict);
        }
        self.phase = ExecutionPhase::Running;
        Ok(())
    }

    /// Same terminal decision is idempotent within the same generation.
    /// A different decision, including a late success after cancel, conflicts.
    pub fn finish(
        &mut self,
        generation: AttemptGeneration,
        phase: ExecutionPhase,
    ) -> Result<TransitionEffect, TransitionError> {
        if generation != self.generation {
            return Err(TransitionError::StaleAttempt);
        }
        if !phase.is_terminal() {
            return Err(TransitionError::Conflict);
        }
        if self.phase == phase {
            return Ok(TransitionEffect::Unchanged);
        }
        if self.phase.is_terminal() {
            return Err(TransitionError::Conflict);
        }
        if self.phase == ExecutionPhase::Queued
            && !matches!(
                phase,
                ExecutionPhase::Cancelled | ExecutionPhase::Interrupted
            )
        {
            return Err(TransitionError::Conflict);
        }
        self.phase = phase;
        Ok(TransitionEffect::Changed)
    }

    pub fn retry(&mut self) -> Result<AttemptGeneration, TransitionError> {
        if !self.phase.is_terminal() {
            return Err(TransitionError::Conflict);
        }
        let generation = self.generation.next()?;
        self.generation = generation;
        self.phase = ExecutionPhase::Queued;
        Ok(generation)
    }
}

/// Independent capture and process outcomes allow a successful process exit
/// to coexist with interrupted log capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureState {
    Open,
    Complete,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPhase {
    Preparing,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
}

/// The provider may report work it performs itself. That lifecycle is a
/// transcript fact, and never a request for the local tool executor to run it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionKind {
    Runtime,
    ProviderHosted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRoundPhase {
    Running,
    Succeeded,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum InteractionDecision {
    Permission {
        reply: crate::PermissionReply,
        allowed: bool,
    },
    Input(String),
    Cancel,
}
