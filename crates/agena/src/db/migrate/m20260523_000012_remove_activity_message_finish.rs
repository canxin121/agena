use sea_orm::sea_query::Table;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if manager
            .has_column("agena_activity_messages", "finish")
            .await?
        {
            manager
                .alter_table(
                    Table::alter()
                        .table(Alias::new("agena_activity_messages"))
                        .drop_column(Alias::new("finish"))
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if !manager
            .has_column("agena_activity_messages", "finish")
            .await?
        {
            manager
                .alter_table(
                    Table::alter()
                        .table(Alias::new("agena_activity_messages"))
                        .add_column(ColumnDef::new(Alias::new("finish")).string().null())
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}
