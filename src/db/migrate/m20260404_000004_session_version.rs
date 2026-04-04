use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const SESSIONS_TABLE: &str = "agena_sessions";
const VERSION_COLUMN: &str = "version";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if manager.has_column(SESSIONS_TABLE, VERSION_COLUMN).await? {
            return Ok(());
        }

        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new(SESSIONS_TABLE))
                    .add_column(
                        ColumnDef::new(Alias::new(VERSION_COLUMN))
                            .big_integer()
                            .not_null()
                            .default(1),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if !manager.has_column(SESSIONS_TABLE, VERSION_COLUMN).await? {
            return Ok(());
        }

        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new(SESSIONS_TABLE))
                    .drop_column(Alias::new(VERSION_COLUMN))
                    .to_owned(),
            )
            .await
    }
}
