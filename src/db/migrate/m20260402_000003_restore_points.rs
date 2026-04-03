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

        let create_restore_points = schema
            .create_table_from_entity(entities::session_restore_point::Entity)
            .if_not_exists()
            .to_owned();
        let create_checkpoint_blobs = schema
            .create_table_from_entity(entities::checkpoint_blob::Entity)
            .if_not_exists()
            .to_owned();

        manager
            .get_connection()
            .execute(backend.build(&create_restore_points))
            .await?;
        manager
            .get_connection()
            .execute(backend.build(&create_checkpoint_blobs))
            .await?;

        for index in [
            Index::create()
                .name("idx_agena_restore_points_session_created")
                .table(entities::session_restore_point::Entity)
                .col(entities::session_restore_point::Column::SessionId)
                .col(entities::session_restore_point::Column::Id)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("idx_agena_restore_points_session_seq")
                .table(entities::session_restore_point::Entity)
                .col(entities::session_restore_point::Column::SessionId)
                .col(entities::session_restore_point::Column::UptoSeq)
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
                    .table(entities::checkpoint_blob::Entity)
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .if_exists()
                    .table(entities::session_restore_point::Entity)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}
