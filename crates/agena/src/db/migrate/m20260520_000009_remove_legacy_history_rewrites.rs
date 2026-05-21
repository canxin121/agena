use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel,
    QueryFilter, TransactionTrait,
};
use sea_orm_migration::prelude::*;
use serde_json::json;

use crate::db::event_entity;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        rename_activity_projection_hidden_column(manager).await?;

        let txn = manager.get_connection().begin().await?;
        rewrite_legacy_events(&txn).await?;
        txn.commit().await?;

        manager
            .drop_table(
                Table::drop()
                    .if_exists()
                    .table(Alias::new("agena_session_snapshots"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        restore_legacy_activity_projection_column(manager).await?;

        manager
            .create_table(
                Table::create()
                    .if_not_exists()
                    .table(Alias::new("agena_session_snapshots"))
                    .col(
                        ColumnDef::new(Alias::new("session_id"))
                            .big_integer()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("last_seq"))
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("view_json"))
                            .json_binary()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("updated_at_ms"))
                            .big_integer()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await
    }
}

async fn rename_activity_projection_hidden_column(
    manager: &SchemaManager<'_>,
) -> Result<(), DbErr> {
    if !manager.has_table("agena_activity_messages").await? {
        return Ok(());
    }

    manager
        .drop_index(
            Index::drop()
                .if_exists()
                .name("idx_agena_activity_messages_session_compacted")
                .table(Alias::new("agena_activity_messages"))
                .to_owned(),
        )
        .await?;

    if manager
        .has_column("agena_activity_messages", "is_compacted")
        .await?
        && !manager
            .has_column("agena_activity_messages", "is_hidden")
            .await?
    {
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("agena_activity_messages"))
                    .rename_column(Alias::new("is_compacted"), Alias::new("is_hidden"))
                    .to_owned(),
            )
            .await?;
    }

    if !manager
        .has_column("agena_activity_messages", "is_hidden")
        .await?
    {
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("agena_activity_messages"))
                    .add_column(
                        ColumnDef::new(Alias::new("is_hidden"))
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .to_owned(),
            )
            .await?;
    }

    manager
        .create_index(
            Index::create()
                .name("idx_agena_activity_messages_session_hidden")
                .table(Alias::new("agena_activity_messages"))
                .col(Alias::new("session_id"))
                .col(Alias::new("is_hidden"))
                .col(Alias::new("created_at_ms"))
                .col(Alias::new("message_id"))
                .if_not_exists()
                .to_owned(),
        )
        .await
}

async fn restore_legacy_activity_projection_column(
    manager: &SchemaManager<'_>,
) -> Result<(), DbErr> {
    if !manager.has_table("agena_activity_messages").await? {
        return Ok(());
    }

    manager
        .drop_index(
            Index::drop()
                .if_exists()
                .name("idx_agena_activity_messages_session_hidden")
                .table(Alias::new("agena_activity_messages"))
                .to_owned(),
        )
        .await?;

    if manager
        .has_column("agena_activity_messages", "is_hidden")
        .await?
        && !manager
            .has_column("agena_activity_messages", "is_compacted")
            .await?
    {
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("agena_activity_messages"))
                    .rename_column(Alias::new("is_hidden"), Alias::new("is_compacted"))
                    .to_owned(),
            )
            .await?;
    }

    manager
        .create_index(
            Index::create()
                .name("idx_agena_activity_messages_session_compacted")
                .table(Alias::new("agena_activity_messages"))
                .col(Alias::new("session_id"))
                .col(Alias::new("is_compacted"))
                .col(Alias::new("created_at_ms"))
                .col(Alias::new("message_id"))
                .if_not_exists()
                .to_owned(),
        )
        .await
}

async fn rewrite_legacy_events<C>(db: &C) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    let events = event_entity::Entity::find()
        .filter(event_entity::Column::KindTag.is_in([
            "message_revised",
            "tool_call_completed",
            "system_notice_appended",
        ]))
        .all(db)
        .await?;

    for event in events {
        match event.kind_tag.as_str() {
            "message_revised" => {
                event_entity::Entity::delete_by_id(event.id)
                    .exec(db)
                    .await?;
            }
            "system_notice_appended" if is_removed_prompt_summary_notice(&event.payload) => {
                event_entity::Entity::delete_by_id(event.id)
                    .exec(db)
                    .await?;
            }
            "tool_call_completed" => {
                if let Some(replacement) = legacy_pruned_tool_output(&event.payload) {
                    let mut payload = event.payload.clone();
                    if let Some(output) = payload.pointer_mut("/payload/output") {
                        *output = json!({
                            "kind": "text",
                            "text": replacement,
                        });
                    }
                    let mut active = event.into_active_model();
                    active.payload = ActiveValue::Set(payload);
                    active.update(db).await?;
                }
            }
            _ => {}
        }
    }

    Ok(())
}

fn is_removed_prompt_summary_notice(payload: &serde_json::Value) -> bool {
    payload
        .pointer("/payload/kind")
        .and_then(serde_json::Value::as_str)
        == Some("compaction_summary")
}

fn legacy_pruned_tool_output(payload: &serde_json::Value) -> Option<String> {
    if payload
        .pointer("/payload/output/kind")
        .and_then(serde_json::Value::as_str)
        != Some("pruned")
    {
        return None;
    }

    Some(
        payload
            .pointer("/payload/output/replacement")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
    )
}
