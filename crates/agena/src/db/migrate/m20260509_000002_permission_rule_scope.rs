use sea_orm::sea_query::{ColumnDef, Index};
use sea_orm_migration::prelude::*;

use crate::db::entities;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for column in [
            ColumnDef::new(entities::permission_rule::Column::Scope)
                .string()
                .not_null()
                .default("workspace")
                .to_owned(),
            ColumnDef::new(entities::permission_rule::Column::SessionId)
                .big_integer()
                .to_owned(),
            ColumnDef::new(entities::permission_rule::Column::WorkspaceId)
                .big_integer()
                .to_owned(),
            ColumnDef::new(entities::permission_rule::Column::Source)
                .string()
                .not_null()
                .default("legacy")
                .to_owned(),
            ColumnDef::new(entities::permission_rule::Column::Reason)
                .string()
                .to_owned(),
            ColumnDef::new(entities::permission_rule::Column::Operator)
                .string()
                .to_owned(),
        ] {
            manager
                .alter_table(
                    Table::alter()
                        .table(entities::permission_rule::Entity)
                        .add_column(column)
                        .to_owned(),
                )
                .await?;
        }

        manager
            .drop_index(
                Index::drop()
                    .name("uq_agena_permission_rule_action_key")
                    .table(entities::permission_rule::Entity)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("uq_agena_permission_rule_scope_subject")
                    .table(entities::permission_rule::Entity)
                    .col(entities::permission_rule::Column::ActionKey)
                    .col(entities::permission_rule::Column::Scope)
                    .col(entities::permission_rule::Column::SessionId)
                    .col(entities::permission_rule::Column::WorkspaceId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("uq_agena_permission_rule_scope_subject")
                    .table(entities::permission_rule::Entity)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("uq_agena_permission_rule_action_key")
                    .table(entities::permission_rule::Entity)
                    .col(entities::permission_rule::Column::ActionKey)
                    .unique()
                    .to_owned(),
            )
            .await?;

        for column in [
            entities::permission_rule::Column::Operator,
            entities::permission_rule::Column::Reason,
            entities::permission_rule::Column::Source,
            entities::permission_rule::Column::WorkspaceId,
            entities::permission_rule::Column::SessionId,
            entities::permission_rule::Column::Scope,
        ] {
            manager
                .alter_table(
                    Table::alter()
                        .table(entities::permission_rule::Entity)
                        .drop_column(column)
                        .to_owned(),
                )
                .await?;
        }

        Ok(())
    }
}
