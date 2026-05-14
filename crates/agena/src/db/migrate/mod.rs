pub mod event_migration;
mod m20260427_000001_initial;
mod m20260509_000002_permission_rule_scope;
mod m20260509_000003_permission_rule_revoke;
mod m20260509_000004_permission_rule_global_scope;
mod m20260515_000005_session_goal;
mod migrator;

use sea_orm::{DatabaseConnection, DbErr};
use sea_orm_migration::MigratorTrait;

pub use migrator::Migrator;

pub async fn up(db: &DatabaseConnection) -> Result<(), DbErr> {
    Migrator::up(db, None).await
}
