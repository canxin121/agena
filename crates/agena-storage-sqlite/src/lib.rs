//! # agena-storage-sqlite
//!
//! SQLite/SeaORM implementations of storage contracts.
//!
//! This crate owns the concrete database adapter. [`agena_storage`] remains the
//! backend-neutral contract crate and must not gain SeaORM types.
//!
//! ## Key items
//!
//! - [`SeaModelCatalogRepository`], [`SeaWorkspaceRepository`],
//!   [`SeaPermissionRuleRepository`] — SQLite-backed infrastructure repositories.
//! - [`initialize_schema`] — create an empty instance database or validate its
//!   complete schema exactly, including scheduler and application state.
//! - [`ServerStateRepository`] — instance-scoped JSON state on the same pool.
//!
//! The session-kernel schema enforces ownership, immutable content, and fact
//! invariants with foreign keys, checks, and transaction-scoped commands.

mod connection;
mod kernel;
mod model_catalog_repository;
mod permission_rule_repository;
mod schema;
mod server_state_repository;
mod transaction;
mod workspace_repository;

pub use connection::{KernelDatabase, connect_owned_database, is_memory_database};
pub use kernel::{
    KernelDiagnostics, KernelMetricsSnapshot, ReadCacheSnapshot, SqliteKernel, open_kernel_runtime,
    open_session_kernel,
};
pub use model_catalog_repository::SeaModelCatalogRepository;
pub use permission_rule_repository::{
    SeaPermissionRuleRepository, SeaPermissionRuleTransactionWriter,
};
pub use schema::initialize_schema;
pub use server_state_repository::ServerStateRepository;
pub use transaction::{
    acquire_write_lock, acquire_write_permit, is_sqlite_busy, is_write_admission_busy,
    run_transaction_app_effects, run_transaction_effects,
};
pub use workspace_repository::SeaWorkspaceRepository;

#[cfg(test)]
mod concurrency_tests;
#[cfg(test)]
mod instance_tests;
