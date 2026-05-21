use sea_orm_migration::prelude::*;

use super::event_migration::EventsMigration;
use super::m20260427_000001_initial;
use super::m20260509_000002_permission_rule_scope;
use super::m20260509_000003_permission_rule_revoke;
use super::m20260509_000004_permission_rule_global_scope;
use super::m20260515_000005_activity_projection;
use super::m20260515_000005_session_goal;
use super::m20260516_000006_session_goal_accounting;
use super::m20260518_000007_model_catalog;
use super::m20260520_000008_activity_projection_state;
use super::m20260520_000009_remove_legacy_history_rewrites;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260427_000001_initial::Migration),
            Box::new(m20260509_000002_permission_rule_scope::Migration),
            Box::new(m20260509_000003_permission_rule_revoke::Migration),
            Box::new(m20260509_000004_permission_rule_global_scope::Migration),
            Box::new(EventsMigration),
            Box::new(m20260515_000005_activity_projection::Migration),
            Box::new(m20260515_000005_session_goal::Migration),
            Box::new(m20260516_000006_session_goal_accounting::Migration),
            Box::new(m20260518_000007_model_catalog::Migration),
            Box::new(m20260520_000008_activity_projection_state::Migration),
            Box::new(m20260520_000009_remove_legacy_history_rewrites::Migration),
        ]
    }
}
