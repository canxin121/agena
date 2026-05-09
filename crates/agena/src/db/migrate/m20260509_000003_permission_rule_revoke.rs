use sea_orm::sea_query::{ColumnDef, Index};
use sea_orm_migration::prelude::*;

use crate::db::entities;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for column in [
            ColumnDef::new(entities::permission_rule::Column::RevokedAtMs)
                .big_integer()
                .to_owned(),
            ColumnDef::new(entities::permission_rule::Column::RevokedReason)
                .string()
                .to_owned(),
            ColumnDef::new(entities::permission_rule::Column::RevokedBy)
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
            .create_index(
                Index::create()
                    .name("idx_agena_permission_rule_active_updated")
                    .table(entities::permission_rule::Entity)
                    .col(entities::permission_rule::Column::RevokedAtMs)
                    .col(entities::permission_rule::Column::UpdatedAtMs)
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("idx_agena_permission_rule_active_updated")
                    .table(entities::permission_rule::Entity)
                    .to_owned(),
            )
            .await?;

        for column in [
            entities::permission_rule::Column::RevokedBy,
            entities::permission_rule::Column::RevokedReason,
            entities::permission_rule::Column::RevokedAtMs,
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
