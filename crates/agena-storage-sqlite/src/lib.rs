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
//! - [`initialize_schema`] — create an empty database, validate existing durable
//!   objects exactly, and install missing derived performance indexes.
//!
//! The `schema_invariants` module installs database triggers that keep
//! invariants enforced at the storage layer.

mod content_backend;
mod database_content;
mod engine;
mod model_catalog_repository;
mod permission_rule_repository;
mod schema;
mod schema_invariants;
mod transaction;
mod workspace_repository;

pub use content_backend::{FileContentBackend, FileContentConfig};
pub use database_content::DatabaseContentBackend;
#[cfg(test)]
use engine::SqliteEngine;
pub use model_catalog_repository::SeaModelCatalogRepository;
pub use permission_rule_repository::{
    SeaPermissionRuleRepository, SeaPermissionRuleTransactionWriter,
};
pub use schema::initialize_schema;
pub use schema_invariants::install_invariant_triggers;
pub use transaction::{
    acquire_write_lock, acquire_write_permit, is_sqlite_busy, is_write_admission_busy,
    run_transaction_app_effects, run_transaction_effects,
};
pub use workspace_repository::SeaWorkspaceRepository;

/// Assemble the only public Part storage boundary. Runtime code never holds a
/// raw SQLite engine or decides how its cache is maintained.
pub fn session_store(
    database: std::sync::Arc<sea_orm::DatabaseConnection>,
    max_cached_sessions: usize,
) -> std::sync::Arc<dyn agena_storage::store::SessionStore> {
    let contents = agena_storage::content::ContentHub::new(
        std::sync::Arc::new(DatabaseContentBackend::new(database.clone())),
        Default::default(),
    );
    session_store_with_contents(database, max_cached_sessions, contents)
}

/// Infrastructure composition may inject a content backend (for example a
/// measured stream sink) while preserving the same sealed Part facade.
pub fn session_store_with_contents(
    database: std::sync::Arc<sea_orm::DatabaseConnection>,
    max_cached_sessions: usize,
    contents: agena_storage::content::ContentHub,
) -> std::sync::Arc<dyn agena_storage::store::SessionStore> {
    agena_storage::backend::session_store(
        engine::SqliteEngine::new(database),
        max_cached_sessions,
        contents,
    )
}

#[cfg(test)]
mod concurrency_tests;
#[cfg(test)]
mod engine_tests;
