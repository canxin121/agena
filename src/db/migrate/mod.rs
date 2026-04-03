mod m20260322_000001_initial;
mod m20260330_000002_runtime;
mod m20260404_000003_drop_checkpoints;
mod migrator;

use sea_orm::{DatabaseConnection, DbErr};
use sea_orm_migration::MigratorTrait;

pub use migrator::Migrator;

pub async fn up(db: &DatabaseConnection) -> Result<(), DbErr> {
    Migrator::up(db, None).await
}
