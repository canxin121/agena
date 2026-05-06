use sea_orm::Schema;
use sea_orm::sea_query::Index;
use sea_orm_migration::prelude::*;

use crate::db::entities;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Single fresh schema for the agena database.
///
/// Tables created:
/// - `agena_workspaces`
/// - `agena_sessions`
/// - `agena_permission_rules`
/// - `agena_session_snapshots`
///
/// The unified event log (`agena_events`) is created by the migrator from
/// `agena-event-store-sea`, which runs immediately after this migration.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let schema = Schema::new(backend);

        for create in [
            schema
                .create_table_from_entity(entities::workspace::Entity)
                .if_not_exists()
                .to_owned(),
            schema
                .create_table_from_entity(entities::session::Entity)
                .if_not_exists()
                .to_owned(),
            schema
                .create_table_from_entity(entities::permission_rule::Entity)
                .if_not_exists()
                .to_owned(),
            schema
                .create_table_from_entity(entities::session_snapshot::Entity)
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
                .name("uq_agena_workspace_path")
                .table(entities::workspace::Entity)
                .col(entities::workspace::Column::Path)
                .unique()
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("idx_agena_session_parent_id")
                .table(entities::session::Entity)
                .col(entities::session::Column::ParentId)
                .col(entities::session::Column::Id)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("idx_agena_session_workspace_id_updated")
                .table(entities::session::Entity)
                .col(entities::session::Column::WorkspaceId)
                .col(entities::session::Column::UpdatedAtMs)
                .col(entities::session::Column::Id)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("uq_agena_permission_rule_action_key")
                .table(entities::permission_rule::Entity)
                .col(entities::permission_rule::Column::ActionKey)
                .unique()
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
        for entity in [
            Table::drop()
                .if_exists()
                .table(entities::session_snapshot::Entity)
                .to_owned(),
            Table::drop()
                .if_exists()
                .table(entities::permission_rule::Entity)
                .to_owned(),
            Table::drop()
                .if_exists()
                .table(entities::session::Entity)
                .to_owned(),
            Table::drop()
                .if_exists()
                .table(entities::workspace::Entity)
                .to_owned(),
        ] {
            manager.drop_table(entity).await?;
        }
        Ok(())
    }
}
