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

        for create in [
            schema
                .create_table_from_entity(entities::activity_message::Entity)
                .if_not_exists()
                .to_owned(),
            schema
                .create_table_from_entity(entities::activity_part::Entity)
                .if_not_exists()
                .to_owned(),
        ] {
            manager
                .get_connection()
                .execute(backend.build(&create))
                .await?;
        }

        for index in [
            Index::create()
                .name("idx_agena_activity_messages_session_created")
                .table(entities::activity_message::Entity)
                .col(entities::activity_message::Column::SessionId)
                .col(entities::activity_message::Column::CreatedAtMs)
                .col(entities::activity_message::Column::MessageId)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("idx_agena_activity_messages_session_compacted")
                .table(entities::activity_message::Entity)
                .col(entities::activity_message::Column::SessionId)
                .col(entities::activity_message::Column::IsCompacted)
                .col(entities::activity_message::Column::CreatedAtMs)
                .col(entities::activity_message::Column::MessageId)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("idx_agena_activity_parts_message_index")
                .table(entities::activity_part::Entity)
                .col(entities::activity_part::Column::MessageId)
                .col(entities::activity_part::Column::PartIndex)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("idx_agena_activity_parts_session_message")
                .table(entities::activity_part::Entity)
                .col(entities::activity_part::Column::SessionId)
                .col(entities::activity_part::Column::MessageId)
                .col(entities::activity_part::Column::PartId)
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
        for table in [
            Table::drop()
                .if_exists()
                .table(entities::activity_part::Entity)
                .to_owned(),
            Table::drop()
                .if_exists()
                .table(entities::activity_message::Entity)
                .to_owned(),
        ] {
            manager.drop_table(table).await?;
        }
        Ok(())
    }
}
