pub mod crud;
pub mod entities;
pub mod tx;
pub mod event_entity;
pub mod sea_event_store;
pub mod history_event_store;

mod migrate;

pub use migrate::up as migrate_up;
pub use sea_event_store::SeaEventStore;
pub use history_event_store::HistoryEventStore;

use sea_orm::{DatabaseConnection, DbErr};

pub async fn init_schema(db: &DatabaseConnection) -> Result<(), DbErr> {
    migrate::up(db).await
}
