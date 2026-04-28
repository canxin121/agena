//! sea_orm-backed [`agena_event::EventStore`] implementation.
//!
//! This crate is **database-backend-agnostic**: it pulls in `sea-orm` and
//! `sea-orm-migration` with `default-features = false`, leaving the choice of
//! `sqlx-sqlite` / `sqlx-postgres` / `sqlx-mysql` and the async runtime to
//! the binary that consumes it.

pub mod entity;
pub mod migration;
pub mod store;

pub use entity::{Column as EventColumn, Entity as EventEntity, Model as EventModel};
pub use migration::{
    DropLegacyEventTablesMigration, Migration as UnifiedEventsMigration, Migrator,
};
pub use store::SeaEventStore;
