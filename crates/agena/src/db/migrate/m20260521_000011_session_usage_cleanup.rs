use sea_orm::ConnectionTrait;
use sea_orm::Statement;
use sea_orm::sea_query::ColumnDef;
use sea_orm_migration::prelude::*;

use sea_query::Alias;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_column_if_exists(manager, "agena_session_goals", "token_budget").await?;
        drop_column_if_exists(manager, "agena_session_goals", "tokens_used").await?;
        drop_column_if_exists(manager, "agena_session_goals", "time_used_seconds").await?;

        if manager.has_table("agena_session_budgets").await? {
            manager
                .drop_table(
                    Table::drop()
                        .if_exists()
                        .table(Alias::new("agena_session_budgets"))
                        .to_owned(),
                )
                .await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        add_column_if_missing(
            manager,
            "agena_session_goals",
            "tokens_used",
            ColumnDef::new(Alias::new("tokens_used"))
                .big_integer()
                .not_null()
                .default(0)
                .to_owned(),
        )
        .await?;
        add_column_if_missing(
            manager,
            "agena_session_goals",
            "time_used_seconds",
            ColumnDef::new(Alias::new("time_used_seconds"))
                .big_integer()
                .not_null()
                .default(0)
                .to_owned(),
        )
        .await?;
        add_column_if_missing(
            manager,
            "agena_session_goals",
            "token_budget",
            ColumnDef::new(Alias::new("token_budget"))
                .big_integer()
                .null()
                .to_owned(),
        )
        .await?;
        Ok(())
    }
}

async fn drop_column_if_exists(
    manager: &SchemaManager<'_>,
    table_name: &str,
    column_name: &str,
) -> Result<(), DbErr> {
    if !manager.has_table(table_name).await?
        || !has_column(manager, table_name, column_name).await?
    {
        return Ok(());
    }
    manager
        .alter_table(
            Table::alter()
                .table(Alias::new(table_name))
                .drop_column(Alias::new(column_name))
                .to_owned(),
        )
        .await
}

async fn add_column_if_missing(
    manager: &SchemaManager<'_>,
    table_name: &str,
    column_name: &str,
    column: ColumnDef,
) -> Result<(), DbErr> {
    if !manager.has_table(table_name).await? || has_column(manager, table_name, column_name).await?
    {
        return Ok(());
    }
    manager
        .alter_table(
            Table::alter()
                .table(Alias::new(table_name))
                .add_column(column)
                .to_owned(),
        )
        .await
}

async fn has_column(
    manager: &SchemaManager<'_>,
    table_name: &str,
    expected_name: &str,
) -> Result<bool, DbErr> {
    let backend = manager.get_database_backend();
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
