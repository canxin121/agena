//! The parts-first store: pure types, the engine contract, the sealed
//! `SessionStore` facade, and the in-memory backend.
//!
//! The facade composes either [`PersistenceEngine`] backend — [`InMemoryEngine`]
//! here or the SQLite engine in `agena-storage-sqlite` — and is the only public
//! chat-data entry point; no layer outside this module touches the database.

mod engine;
mod error;
mod facade;
mod in_memory;
mod jsonl;
mod part_update;
mod state;
mod types;

pub use engine::{MaintenanceOutcome, PersistenceEngine, SessionChange};
pub use error::StoreError;
pub use facade::{
    GlobalSubscription, MemoryLayer, NotificationBus, SessionFacade, SessionObserver, SessionStore,
    Subscription,
};
pub use in_memory::{InMemoryEngine, InMemoryEngineConfig};
pub use jsonl::{ExportRecord, ParsedBundle, parse, serialize};
pub use part_update::{prepare_part_update, prepare_run_completion, validate_run_content};
pub use state::{
    InFlightRun, LEASE_STALENESS_MS, PendingInteraction, StateInputs, apply_part_transition,
    derive_session_state, lease_is_fresh, presentation,
};
pub use types::*;
