use sea_orm::sea_query::Index;
use sea_orm::Schema;
use sea_orm_migration::prelude::*;

use crate::db::entities;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let schema = Schema::new(backend);

        let create_workspaces = schema
            .create_table_from_entity(entities::workspace::Entity)
            .if_not_exists()
            .to_owned();
        let create_sessions = schema
            .create_table_from_entity(entities::session::Entity)
            .if_not_exists()
            .to_owned();
        let create_messages = schema
            .create_table_from_entity(entities::message::Entity)
            .if_not_exists()
            .to_owned();
        let create_message_parts = schema
            .create_table_from_entity(entities::message_part::Entity)
            .if_not_exists()
            .to_owned();
        let create_message_part_details = schema
            .create_table_from_entity(entities::message_part_detail::Entity)
            .if_not_exists()
            .to_owned();

        manager.get_connection().execute(backend.build(&create_workspaces)).await?;
        manager.get_connection().execute(backend.build(&create_sessions)).await?;
        manager.get_connection().execute(backend.build(&create_messages)).await?;
        manager.get_connection().execute(backend.build(&create_message_parts)).await?;
        manager
            .get_connection()
            .execute(backend.build(&create_message_part_details))
            .await?;

        let indexes = [
            Index::create()
                .name("uq_agena_workspace_path")
                .table(entities::workspace::Entity)
                .col(entities::workspace::Column::Path)
                .unique()
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("uq_agena_message_part_msg_index")
                .table(entities::message_part::Entity)
                .col(entities::message_part::Column::MessageId)
                .col(entities::message_part::Column::PartIndex)
                .unique()
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("idx_agena_session_parent_id")
                .table(entities::session::Entity)
                .col(entities::session::Column::ParentId)
                .col(entities::session::Column::Id)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("idx_agena_session_workspace_id_updated")
                .table(entities::session::Entity)
                .col(entities::session::Column::WorkspaceId)
                .col(entities::session::Column::UpdatedAtMs)
                .col(entities::session::Column::Id)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("idx_agena_message_session_created")
                .table(entities::message::Entity)
                .col(entities::message::Column::SessionId)
                .col(entities::message::Column::CreatedAtMs)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("idx_agena_message_parent_id")
                .table(entities::message::Entity)
                .col(entities::message::Column::ParentMessageId)
                .col(entities::message::Column::Id)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("idx_agena_message_part_name")
                .table(entities::message_part::Entity)
                .col(entities::message_part::Column::MessageId)
                .col(entities::message_part::Column::Name)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("idx_agena_message_part_summary")
                .table(entities::message_part::Entity)
                .col(entities::message_part::Column::MessageId)
                .col(entities::message_part::Column::SummaryText)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("idx_agena_message_part_call")
                .table(entities::message_part::Entity)
                .col(entities::message_part::Column::CallId)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("idx_agena_message_part_operation")
                .table(entities::message_part::Entity)
                .col(entities::message_part::Column::OperationId)
                .if_not_exists()
                .to_owned(),
        ];

        for index in indexes {
            manager.get_connection().execute(backend.build(&index)).await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .if_exists()
                    .table(entities::message_part_detail::Entity)
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .if_exists()
                    .table(entities::message_part::Entity)
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .if_exists()
                    .table(entities::message::Entity)
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .if_exists()
                    .table(entities::session::Entity)
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .if_exists()
                    .table(entities::workspace::Entity)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}
