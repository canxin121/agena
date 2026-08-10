//! The v2 parts-first store: pure types, the engine contract, the sealed
//! `SessionStore` facade, and the in-memory backend. See
//! `docs/database-design-v2.md` sections 14-15.
//!
//! The facade composes either [`PersistenceEngine`] backend — [`InMemoryEngine`]
//! here or the SQLite engine in `agena-storage-sqlite` — and is the only public
//! chat-data entry point; no layer outside this module touches the database.

mod engine;
mod error;
mod facade;
mod in_memory;
mod jsonl;
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
pub use state::{
    InFlightRun, LEASE_STALENESS_MS, PendingInteraction, StateInputs, apply_part_transition,
    derive_session_state, lease_is_fresh, presentation,
};
pub use types::*;
