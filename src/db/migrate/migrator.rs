use sea_orm_migration::prelude::*;

use super::m20260322_000001_initial;
use super::m20260330_000002_runtime;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260322_000001_initial::Migration),
            Box::new(m20260330_000002_runtime::Migration),
        ]
    }
}
