pub mod crud;
pub mod entities;
pub mod event_entity;
pub mod sea_event_store;
pub mod tx;

mod schema;

pub use sea_event_store::SeaEventStore;

use sea_orm::{DatabaseConnection, DbErr};

pub async fn init_schema(db: &DatabaseConnection) -> Result<(), DbErr> {
    schema::create(db).await
}
