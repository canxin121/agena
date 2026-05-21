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
        let create = schema
            .create_table_from_entity(entities::activity_projection_state::Entity)
            .if_not_exists()
            .to_owned();
        manager
            .get_connection()
            .execute(backend.build(&create))
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .if_exists()
                    .table(entities::activity_projection_state::Entity)
                    .to_owned(),
            )
            .await
    }
}
