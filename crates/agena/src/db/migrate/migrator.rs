use sea_orm_migration::prelude::*;

use super::event_migration::EventsMigration;
use super::m20260427_000001_initial;
use super::m20260509_000002_permission_rule_scope;
use super::m20260509_000003_permission_rule_revoke;
use super::m20260509_000004_permission_rule_global_scope;
use super::m20260515_000005_activity_projection;

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
        ]
    }
}
