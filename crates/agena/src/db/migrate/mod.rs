pub mod event_migration;
mod m20260427_000001_initial;
mod m20260509_000002_permission_rule_scope;
mod m20260509_000003_permission_rule_revoke;
mod m20260509_000004_permission_rule_global_scope;
mod m20260515_000005_activity_projection;
mod m20260515_000005_session_goal;
mod m20260518_000007_model_catalog;
mod m20260520_000008_activity_projection_state;
mod m20260520_000009_remove_legacy_history_rewrites;
mod m20260521_000011_session_usage_cleanup;
mod migrator;

use sea_orm::{DatabaseConnection, DbErr};
use sea_orm_migration::MigratorTrait;

pub use migrator::Migrator;

pub async fn up(db: &DatabaseConnection) -> Result<(), DbErr> {
    Migrator::up(db, None).await
}
