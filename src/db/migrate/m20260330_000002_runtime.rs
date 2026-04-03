use sea_orm::Schema;
use sea_orm::sea_query::Index;
use sea_orm_migration::prelude::*;

use crate::db::entities;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let schema = Schema::new(backend);

        let create_session_events = schema
            .create_table_from_entity(entities::session_event::Entity)
            .if_not_exists()
            .to_owned();
        let create_permission_rules = schema
            .create_table_from_entity(entities::permission_rule::Entity)
            .if_not_exists()
            .to_owned();

        manager
            .get_connection()
            .execute(backend.build(&create_session_events))
            .await?;
        manager
            .get_connection()
            .execute(backend.build(&create_permission_rules))
            .await?;

        for index in [
            Index::create()
                .name("idx_agena_session_events_session_seq")
                .table(entities::session_event::Entity)
                .col(entities::session_event::Column::SessionId)
                .col(entities::session_event::Column::Seq)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("idx_agena_session_events_created")
                .table(entities::session_event::Entity)
                .col(entities::session_event::Column::SessionId)
                .col(entities::session_event::Column::CreatedAtMs)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("uq_agena_permission_rule_action_key")
                .table(entities::permission_rule::Entity)
                .col(entities::permission_rule::Column::ActionKey)
                .unique()
                .if_not_exists()
                .to_owned(),
        ] {
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
                    .table(entities::permission_rule::Entity)
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .if_exists()
                    .table(entities::session_event::Entity)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}
