use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const SESSIONS_TABLE: &str = "agena_sessions";
const MESSAGES_TABLE: &str = "agena_messages";
const RUNTIME_STATE_COLUMN: &str = "runtime_state_json";
const TAGS_COLUMN: &str = "tags_json";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if !manager
            .has_column(SESSIONS_TABLE, RUNTIME_STATE_COLUMN)
            .await?
        {
            manager
                .alter_table(
                    Table::alter()
                        .table(Alias::new(SESSIONS_TABLE))
                        .add_column(
                            ColumnDef::new(Alias::new(RUNTIME_STATE_COLUMN))
                                .json_binary()
                                .null(),
                        )
                        .to_owned(),
                )
                .await?;
        }

        if !manager.has_column(MESSAGES_TABLE, TAGS_COLUMN).await? {
            manager
                .alter_table(
                    Table::alter()
                        .table(Alias::new(MESSAGES_TABLE))
                        .add_column(ColumnDef::new(Alias::new(TAGS_COLUMN)).json_binary().null())
                        .to_owned(),
                )
                .await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if manager.has_column(MESSAGES_TABLE, TAGS_COLUMN).await? {
            manager
                .alter_table(
                    Table::alter()
                        .table(Alias::new(MESSAGES_TABLE))
                        .drop_column(Alias::new(TAGS_COLUMN))
                        .to_owned(),
                )
                .await?;
        }

        if manager
            .has_column(SESSIONS_TABLE, RUNTIME_STATE_COLUMN)
            .await?
        {
            manager
                .alter_table(
                    Table::alter()
                        .table(Alias::new(SESSIONS_TABLE))
                        .drop_column(Alias::new(RUNTIME_STATE_COLUMN))
                        .to_owned(),
                )
                .await?;
        }

        Ok(())
    }
}
