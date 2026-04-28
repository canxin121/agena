use sea_orm::Schema;
use sea_orm::sea_query::Index;
use sea_orm_migration::prelude::*;

use crate::entity;

/// Default migrator: creates the unified events table only.
///
/// Legacy table cleanup lives in the separate
/// [`DropLegacyEventTablesMigration`] so callers can stage the cutover.
pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(Migration)]
    }
}

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Creates the `agena_events` table and its indexes.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let schema = Schema::new(backend);

        let create = schema
            .create_table_from_entity(entity::Entity)
            .if_not_exists()
            .to_owned();
        manager
            .get_connection()
            .execute(backend.build(&create))
            .await?;

        for index in indexes() {
            manager
                .get_connection()
                .execute(backend.build(&index))
                .await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().if_exists().table(entity::Entity).to_owned())
            .await?;
        Ok(())
    }
}

/// Separate migration that drops the legacy event tables. Run only when the
/// reader/writer cutover has been completed.
#[derive(DeriveMigrationName)]
pub struct DropLegacyEventTablesMigration;

#[async_trait::async_trait]
impl MigrationTrait for DropLegacyEventTablesMigration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for legacy in [LegacyTable::SessionEvents, LegacyTable::SessionHistoryEvents] {
            manager
                .drop_table(Table::drop().if_exists().table(legacy).to_owned())
                .await?;
        }
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "downgrade is not supported: legacy event tables cannot be reconstructed".into(),
        ))
    }
}

fn indexes() -> Vec<sea_orm::sea_query::IndexCreateStatement> {
    vec![
        Index::create()
            .name("idx_agena_events_seq_global")
            .table(entity::Entity)
            .col(entity::Column::SeqGlobal)
            .unique()
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("idx_agena_events_session_seq")
            .table(entity::Entity)
            .col(entity::Column::SessionId)
            .col(entity::Column::SeqSession)
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("idx_agena_events_workspace_seq")
            .table(entity::Entity)
            .col(entity::Column::WorkspaceId)
            .col(entity::Column::SeqGlobal)
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("idx_agena_events_kind_seq")
            .table(entity::Entity)
            .col(entity::Column::KindTag)
            .col(entity::Column::SeqGlobal)
            .if_not_exists()
            .to_owned(),
    ]
}

#[derive(DeriveIden)]
enum LegacyTable {
    #[sea_orm(iden = "agena_session_events")]
    SessionEvents,
    #[sea_orm(iden = "agena_session_history_events")]
    SessionHistoryEvents,
}
