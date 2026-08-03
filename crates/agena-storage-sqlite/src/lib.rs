//! SQLite/SeaORM implementations of storage contracts.
//!
//! This crate owns concrete database adapters. `agena-storage` remains the
//! backend-neutral contract crate and must not gain SeaORM types.

mod event_store;
mod model_catalog_repository;
mod model_message_repository;
mod permission_rule_repository;
mod projection_lookup_repository;
mod schema;
mod schema_invariants;
mod schema_lifecycle;
mod sequence_allocator;
mod session_stats_repository;
mod session_summary_repository;
mod stored_values;
mod transaction;
mod usage_repository;
mod workspace_repository;

pub use event_store::SeaEventStore;
pub use model_catalog_repository::SeaModelCatalogRepository;
pub use model_message_repository::{
    PersistedCompletionUsage, SeaModelMessageRepository, SeaModelMessageTransactionWriter,
};
pub use permission_rule_repository::{
    SeaPermissionRuleRepository, SeaPermissionRuleTransactionWriter,
};
pub use projection_lookup_repository::SeaProjectionLookupRepository;
pub use schema::initialize_schema;
pub use schema_invariants::install_invariant_triggers;
pub use schema_lifecycle::CURRENT_SCHEMA_VERSION;
pub use sequence_allocator::SqliteSequenceAllocator;
pub use session_stats_repository::SeaSessionStatsRepository;
pub use session_summary_repository::SeaSessionSummaryRepository;
pub use stored_values::{StoredExecutionStatus, StoredPartKind, StoredRole};
pub use transaction::{run_transaction_app_effects, run_transaction_effects};
pub use usage_repository::SeaUsageRepository;
pub use workspace_repository::SeaWorkspaceRepository;

#[cfg(test)]
mod concurrency_tests;
