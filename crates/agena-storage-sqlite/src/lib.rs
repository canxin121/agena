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
//! - [`initialize_schema`] — create the schema only for an empty database and
//!   otherwise require an exact match with this build.
//!
//! The `schema_invariants` module installs database triggers that keep
//! invariants enforced at the storage layer.

mod engine;
mod model_catalog_repository;
mod permission_rule_repository;
mod schema;
mod schema_invariants;
mod transaction;
mod workspace_repository;

pub use engine::SqliteEngine;
pub use model_catalog_repository::SeaModelCatalogRepository;
pub use permission_rule_repository::{
    SeaPermissionRuleRepository, SeaPermissionRuleTransactionWriter,
};
pub use schema::initialize_schema;
pub use schema_invariants::install_invariant_triggers;
pub use transaction::{
    acquire_write_lock, is_sqlite_busy, run_transaction_app_effects, run_transaction_effects,
};
pub use workspace_repository::SeaWorkspaceRepository;

#[cfg(test)]
mod concurrency_tests;
#[cfg(test)]
mod engine_tests;
