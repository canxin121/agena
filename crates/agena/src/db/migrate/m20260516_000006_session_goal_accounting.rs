use sea_orm::ConnectionTrait;
use sea_orm::EntityName;
use sea_orm::Statement;
use sea_orm::sea_query::ColumnDef;
use sea_orm_migration::prelude::*;

use crate::db::entities;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        add_column_if_missing(
            manager,
            "tokens_used",
            ColumnDef::new(entities::session_goal::Column::TokensUsed)
                .big_integer()
                .not_null()
                .default(0)
                .to_owned(),
        )
        .await?;
        add_column_if_missing(
            manager,
            "time_used_seconds",
            ColumnDef::new(entities::session_goal::Column::TimeUsedSeconds)
                .big_integer()
                .not_null()
                .default(0)
                .to_owned(),
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for column in [
            entities::session_goal::Column::TimeUsedSeconds,
            entities::session_goal::Column::TokensUsed,
        ] {
            let _ = manager
                .alter_table(
                    Table::alter()
                        .table(entities::session_goal::Entity)
                        .drop_column(column)
                        .to_owned(),
                )
                .await;
        }
        Ok(())
    }
}

async fn add_column_if_missing(
    manager: &SchemaManager<'_>,
    expected_name: &str,
    column: ColumnDef,
) -> Result<(), DbErr> {
    if has_column(manager, expected_name).await? {
        return Ok(());
    }
    manager
        .alter_table(
            Table::alter()
                .table(entities::session_goal::Entity)
                .add_column(column)
                .to_owned(),
        )
        .await
}

async fn has_column(manager: &SchemaManager<'_>, expected_name: &str) -> Result<bool, DbErr> {
    let backend = manager.get_database_backend();
    let table_name = entities::session_goal::Entity.table_name().to_string();
    let statement = Statement::from_string(backend, format!("PRAGMA table_info('{table_name}')"));
    let rows = manager.get_connection().query_all(statement).await?;
    for row in rows {
        let name: String = row.try_get("", "name")?;
        if name == expected_name {
            return Ok(true);
        }
    }
    Ok(false)
}
