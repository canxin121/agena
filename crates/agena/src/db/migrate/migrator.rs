use sea_orm_migration::prelude::*;

use super::event_migration::EventsMigration;
use super::m20260427_000001_initial;
use super::m20260509_000002_permission_rule_scope;
use super::m20260509_000003_permission_rule_revoke;
use super::m20260509_000004_permission_rule_global_scope;
use super::m20260515_000005_activity_projection;
use super::m20260518_000007_model_catalog;
use super::m20260520_000008_activity_projection_state;
use super::m20260520_000009_remove_legacy_history_rewrites;
use super::m20260523_000012_remove_activity_message_finish;
use super::m20260523_000013_add_activity_message_provider_state;

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
            Box::new(m20260518_000007_model_catalog::Migration),
            Box::new(m20260520_000008_activity_projection_state::Migration),
            Box::new(m20260520_000009_remove_legacy_history_rewrites::Migration),
            Box::new(m20260523_000012_remove_activity_message_finish::Migration),
            Box::new(m20260523_000013_add_activity_message_provider_state::Migration),
        ]
    }
}
