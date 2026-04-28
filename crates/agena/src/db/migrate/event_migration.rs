use sea_orm::Schema;
use sea_orm::sea_query::Index;
use sea_orm_migration::prelude::*;

use crate::db::event_entity;

/// Creates the `agena_events` table and its indexes.
#[derive(DeriveMigrationName)]
pub struct EventsMigration;

#[async_trait::async_trait]
impl MigrationTrait for EventsMigration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let schema = Schema::new(backend);

        let create = schema
            .create_table_from_entity(event_entity::Entity)
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
            .drop_table(
                Table::drop()
                    .if_exists()
                    .table(event_entity::Entity)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}

fn indexes() -> Vec<sea_orm::sea_query::IndexCreateStatement> {
    vec![
        Index::create()
            .name("idx_agena_events_seq_global")
            .table(event_entity::Entity)
            .col(event_entity::Column::SeqGlobal)
            .unique()
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("idx_agena_events_session_seq")
            .table(event_entity::Entity)
            .col(event_entity::Column::SessionId)
            .col(event_entity::Column::SeqSession)
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("idx_agena_events_workspace_seq")
            .table(event_entity::Entity)
            .col(event_entity::Column::WorkspaceId)
            .col(event_entity::Column::SeqGlobal)
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("idx_agena_events_kind_seq")
            .table(event_entity::Entity)
            .col(event_entity::Column::KindTag)
            .col(event_entity::Column::SeqGlobal)
            .if_not_exists()
            .to_owned(),
    ]
}
