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

        manager
            .get_connection()
            .execute(
                backend.build(
                    &schema
                        .create_table_from_entity(entities::session_goal::Entity)
                        .if_not_exists()
                        .to_owned(),
                ),
            )
            .await?;

        for index in [
            Index::create()
                .name("uq_agena_session_goal_session_id")
                .table(entities::session_goal::Entity)
                .col(entities::session_goal::Column::SessionId)
                .unique()
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("idx_agena_session_goal_status_updated")
                .table(entities::session_goal::Entity)
                .col(entities::session_goal::Column::Status)
                .col(entities::session_goal::Column::UpdatedAtMs)
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
        for index in [
            "idx_agena_session_goal_status_updated",
            "uq_agena_session_goal_session_id",
        ] {
            manager
                .drop_index(
                    Index::drop()
                        .name(index)
                        .table(entities::session_goal::Entity)
                        .to_owned(),
                )
                .await?;
        }

        manager
            .drop_table(
                Table::drop()
                    .if_exists()
                    .table(entities::session_goal::Entity)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}
