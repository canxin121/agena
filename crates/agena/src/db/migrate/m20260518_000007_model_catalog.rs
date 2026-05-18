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
                .create_table_from_entity(entities::model_catalog_entry::Entity)
                .if_not_exists()
                .to_owned(),
            schema
                .create_table_from_entity(entities::model_catalog_state::Entity)
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
                .name("uq_agena_model_catalog_kind_model")
                .table(entities::model_catalog_entry::Entity)
                .col(entities::model_catalog_entry::Column::Kind)
                .col(entities::model_catalog_entry::Column::ModelId)
                .unique()
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("idx_agena_model_catalog_model_id")
                .table(entities::model_catalog_entry::Entity)
                .col(entities::model_catalog_entry::Column::ModelId)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("idx_agena_model_catalog_kind")
                .table(entities::model_catalog_entry::Entity)
                .col(entities::model_catalog_entry::Column::Kind)
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
                .table(entities::model_catalog_state::Entity)
                .to_owned(),
            Table::drop()
                .if_exists()
                .table(entities::model_catalog_entry::Entity)
                .to_owned(),
        ] {
            manager.drop_table(table).await?;
        }
        Ok(())
    }
}
