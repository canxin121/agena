pub mod crud;
pub mod entities;
pub mod tx;

mod migrate;

pub use migrate::up as migrate_up;

use sea_orm::{DatabaseConnection, DbErr};

pub async fn init_schema(db: &DatabaseConnection) -> Result<(), DbErr> {
    migrate::up(db).await
}
