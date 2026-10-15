//! Parts, immutable snapshots, commands and the sealed [`SessionStore`] API.
//!
//! Business code uses this API for reads and writes. Concrete engines, shared
//! caches and the notification bus stay private. Backend implementors integrate
//! through [`crate::backend`]; ready-made factories return only the sealed API.

mod commands;
pub(crate) mod engine;
mod error;
pub(crate) mod facade;
pub(crate) mod in_memory;
mod jsonl;
pub(crate) mod part_update;
mod state;
mod types;

pub use commands::{PartCommand, PartCommit, PartSnapshot, PartUpdate};
pub(crate) use engine::PersistenceEngine;
pub use engine::{MaintenanceOutcome, SessionChange};
pub use error::StoreError;
pub use facade::{GlobalSubscription, SessionObserver, SessionStore, Subscription};
pub(crate) use in_memory::InMemoryEngine;
pub use jsonl::{ExportRecord, ParsedBundle, parse, prepare_resource_import, serialize};
pub(crate) use part_update::{prepare_part_update, prepare_run_completion, validate_run_content};
pub use state::{
    InFlightRun, PendingInteraction, StateInputs, apply_part_transition, derive_session_state,
    presentation,
};
pub use types::*;

/// Create a fully assembled store with the same commit, cache and notification
/// behavior as durable backends. Consumers never select individual memory layers.
pub fn memory_session_store(max_cached_sessions: usize) -> std::sync::Arc<dyn SessionStore> {
    crate::backend::session_store(
        InMemoryEngine::default(),
        max_cached_sessions,
        crate::content::ContentHub::in_memory(),
    )
}
