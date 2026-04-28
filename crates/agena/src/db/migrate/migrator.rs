use sea_orm_migration::prelude::*;

use super::m20260427_000001_initial;
use super::event_migration::EventsMigration;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260427_000001_initial::Migration),
            Box::new(EventsMigration),
        ]
    }
}
