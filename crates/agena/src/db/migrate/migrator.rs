use sea_orm_migration::prelude::*;

use super::m20260322_000001_initial;
use super::m20260330_000002_runtime;
use super::m20260404_000003_drop_checkpoints;
use super::m20260404_000004_session_version;
use super::m20260405_000005_prompt_runtime;
use super::m20260425_000006_append_only_history;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260322_000001_initial::Migration),
            Box::new(m20260330_000002_runtime::Migration),
            Box::new(m20260404_000003_drop_checkpoints::Migration),
            Box::new(m20260404_000004_session_version::Migration),
            Box::new(m20260405_000005_prompt_runtime::Migration),
            Box::new(m20260425_000006_append_only_history::Migration),
        ]
    }
}
