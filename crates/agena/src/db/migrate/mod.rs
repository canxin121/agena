use sea_orm::sea_query::{
    Condition, ConditionalStatement, Expr, Index, IndexCreateStatement, TableCreateStatement,
};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbErr, Schema, TransactionTrait};

use crate::db::{entities, event_entity};

/// Bootstraps the current database schema directly.
///
/// Agena creates the current schema from scratch.
pub async fn up(db: &DatabaseConnection) -> Result<(), DbErr> {
    let backend = db.get_database_backend();
    let schema = Schema::new(backend);
    let txn = db.begin().await?;

    for create in table_definitions(&schema) {
        txn.execute(backend.build(&create)).await?;
    }
    for index in index_definitions() {
        txn.execute(backend.build(&index)).await?;
    }

    txn.commit().await
}

fn table_definitions(schema: &Schema) -> Vec<TableCreateStatement> {
    vec![
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
            .create_table_from_entity(event_entity::Entity)
            .if_not_exists()
            .to_owned(),
        schema
            .create_table_from_entity(entities::activity_message::Entity)
            .if_not_exists()
            .to_owned(),
        schema
            .create_table_from_entity(entities::activity_part::Entity)
            .if_not_exists()
            .to_owned(),
        schema
            .create_table_from_entity(entities::activity_projection_state::Entity)
            .if_not_exists()
            .to_owned(),
        schema
            .create_table_from_entity(entities::model_catalog_entry::Entity)
            .if_not_exists()
            .to_owned(),
        schema
            .create_table_from_entity(entities::model_catalog_state::Entity)
            .if_not_exists()
            .to_owned(),
    ]
}

fn index_definitions() -> Vec<IndexCreateStatement> {
    vec![
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
            .name("idx_agena_session_root_id")
            .table(entities::session::Entity)
            .col(entities::session::Column::RootId)
            .col(entities::session::Column::Depth)
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
            .name("uq_agena_permission_rule_global_subject")
            .table(entities::permission_rule::Entity)
            .col(entities::permission_rule::Column::ActionKey)
            .col(entities::permission_rule::Column::Scope)
            .unique()
            .cond_where(
                Condition::all()
                    .add(Expr::col(entities::permission_rule::Column::SessionId).is_null())
                    .add(Expr::col(entities::permission_rule::Column::WorkspaceId).is_null()),
            )
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("uq_agena_permission_rule_workspace_subject")
            .table(entities::permission_rule::Entity)
            .col(entities::permission_rule::Column::ActionKey)
            .col(entities::permission_rule::Column::Scope)
            .col(entities::permission_rule::Column::WorkspaceId)
            .unique()
            .cond_where(
                Condition::all()
                    .add(Expr::col(entities::permission_rule::Column::SessionId).is_null())
                    .add(Expr::col(entities::permission_rule::Column::WorkspaceId).is_not_null()),
            )
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("uq_agena_permission_rule_session_subject")
            .table(entities::permission_rule::Entity)
            .col(entities::permission_rule::Column::ActionKey)
            .col(entities::permission_rule::Column::Scope)
            .col(entities::permission_rule::Column::SessionId)
            .unique()
            .cond_where(
                Condition::all()
                    .add(Expr::col(entities::permission_rule::Column::SessionId).is_not_null())
                    .add(Expr::col(entities::permission_rule::Column::WorkspaceId).is_null()),
            )
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("idx_agena_permission_rule_active_updated")
            .table(entities::permission_rule::Entity)
            .col(entities::permission_rule::Column::RevokedAtMs)
            .col(entities::permission_rule::Column::UpdatedAtMs)
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("idx_agena_events_seq_global")
            .table(event_entity::Entity)
            .col(event_entity::Column::SeqGlobal)
            .unique()
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("idx_agena_events_session_seq")
            .table(event_entity::Entity)
            .col(event_entity::Column::SessionId)
            .col(event_entity::Column::SeqSession)
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("idx_agena_events_workspace_seq")
            .table(event_entity::Entity)
            .col(event_entity::Column::WorkspaceId)
            .col(event_entity::Column::SeqGlobal)
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("idx_agena_events_kind_seq")
            .table(event_entity::Entity)
            .col(event_entity::Column::KindTag)
            .col(event_entity::Column::SeqGlobal)
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("idx_agena_activity_messages_session_created")
            .table(entities::activity_message::Entity)
            .col(entities::activity_message::Column::SessionId)
            .col(entities::activity_message::Column::CreatedAtMs)
            .col(entities::activity_message::Column::MessageId)
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("idx_agena_activity_messages_session_hidden")
            .table(entities::activity_message::Entity)
            .col(entities::activity_message::Column::SessionId)
            .col(entities::activity_message::Column::IsHidden)
            .col(entities::activity_message::Column::CreatedAtMs)
            .col(entities::activity_message::Column::MessageId)
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("idx_agena_activity_parts_message_index")
            .table(entities::activity_part::Entity)
            .col(entities::activity_part::Column::MessageId)
            .col(entities::activity_part::Column::PartIndex)
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("idx_agena_activity_parts_session_message")
            .table(entities::activity_part::Entity)
            .col(entities::activity_part::Column::SessionId)
            .col(entities::activity_part::Column::MessageId)
            .col(entities::activity_part::Column::PartId)
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("uq_agena_model_catalog_kind_model")
            .table(entities::model_catalog_entry::Entity)
            .col(entities::model_catalog_entry::Column::Kind)
            .col(entities::model_catalog_entry::Column::ModelId)
            .unique()
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("idx_agena_model_catalog_model_id")
            .table(entities::model_catalog_entry::Entity)
            .col(entities::model_catalog_entry::Column::ModelId)
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("idx_agena_model_catalog_kind")
            .table(entities::model_catalog_entry::Entity)
            .col(entities::model_catalog_entry::Column::Kind)
            .if_not_exists()
            .to_owned(),
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use sea_orm::{ActiveModelTrait, ActiveValue::Set, Database, DatabaseBackend, Statement};

    #[tokio::test]
    async fn init_schema_creates_current_schema_without_migration_history() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("create sqlite memory db");

        up(&db).await.expect("initialize schema");
        up(&db).await.expect("initialize schema is idempotent");

        let tables = sqlite_object_names(&db, "table")
            .await
            .expect("list sqlite tables");
        let expected_tables = BTreeSet::from([
            "agena_activity_messages".to_string(),
            "agena_activity_parts".to_string(),
            "agena_activity_projection_states".to_string(),
            "agena_events".to_string(),
            "agena_model_catalog_entries".to_string(),
            "agena_model_catalog_state".to_string(),
            "agena_permission_rules".to_string(),
            "agena_sessions".to_string(),
            "agena_workspaces".to_string(),
        ]);
        for table in expected_tables {
            assert!(tables.contains(&table), "missing table {table}");
        }
        assert!(
            !tables.contains("seaql_migrations"),
            "migration tracking table should not exist"
        );

        let activity_message_columns = sqlite_table_columns(&db, "agena_activity_messages")
            .await
            .expect("inspect activity message columns");
        assert!(activity_message_columns.contains("provider_state"));
        assert!(activity_message_columns.contains("is_hidden"));
        assert!(!activity_message_columns.contains("finish"));
        assert!(!activity_message_columns.contains("is_compacted"));

        let indexes = sqlite_object_names(&db, "index")
            .await
            .expect("list sqlite indexes");
        for index in [
            "idx_agena_events_seq_global",
            "uq_agena_permission_rule_global_subject",
            "uq_agena_permission_rule_workspace_subject",
            "uq_agena_permission_rule_session_subject",
        ] {
            assert!(indexes.contains(index), "missing index {index}");
        }
    }

    #[tokio::test]
    async fn permission_rule_unique_indexes_handle_nullable_subjects() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("create sqlite memory db");
        up(&db).await.expect("initialize schema");

        insert_permission_rule(&db, "shell", "global", None, None)
            .await
            .expect("insert global rule");
        assert!(
            insert_permission_rule(&db, "shell", "global", None, None)
                .await
                .is_err(),
            "duplicate global rule should be rejected"
        );

        insert_permission_rule(&db, "shell", "workspace", None, Some(7))
            .await
            .expect("insert workspace rule");
        assert!(
            insert_permission_rule(&db, "shell", "workspace", None, Some(7))
                .await
                .is_err(),
            "duplicate workspace rule should be rejected"
        );
        insert_permission_rule(&db, "shell", "workspace", None, Some(8))
            .await
            .expect("different workspace should be allowed");

        insert_permission_rule(&db, "shell", "session", Some(99), None)
            .await
            .expect("insert session rule");
        assert!(
            insert_permission_rule(&db, "shell", "session", Some(99), None)
                .await
                .is_err(),
            "duplicate session rule should be rejected"
        );
        insert_permission_rule(&db, "shell", "session", Some(100), None)
            .await
            .expect("different session should be allowed");
    }

    async fn sqlite_object_names(
        db: &DatabaseConnection,
        object_type: &str,
    ) -> Result<BTreeSet<String>, DbErr> {
        let stmt = Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT name FROM sqlite_master WHERE type = ?1 AND name NOT LIKE 'sqlite_%'",
            [object_type.into()],
        );
        let rows = db.query_all(stmt).await?;
        rows.into_iter()
            .map(|row| row.try_get("", "name"))
            .collect()
    }

    async fn sqlite_table_columns(
        db: &DatabaseConnection,
        table_name: &str,
    ) -> Result<BTreeSet<String>, DbErr> {
        let stmt = Statement::from_string(
            DatabaseBackend::Sqlite,
            format!("PRAGMA table_info({table_name})"),
        );
        let rows = db.query_all(stmt).await?;
        rows.into_iter()
            .map(|row| row.try_get("", "name"))
            .collect()
    }

    async fn insert_permission_rule(
        db: &DatabaseConnection,
        action_key: &str,
        scope: &str,
        session_id: Option<i64>,
        workspace_id: Option<i64>,
    ) -> Result<entities::permission_rule::Model, DbErr> {
        entities::permission_rule::ActiveModel {
            action_key: Set(action_key.to_owned()),
            mode: Set("allow".to_owned()),
            scope: Set(scope.to_owned()),
            session_id: Set(session_id),
            workspace_id: Set(workspace_id),
            source: Set("test".to_owned()),
            reason: Set(None),
            operator: Set(None),
            revoked_at_ms: Set(None),
            revoked_reason: Set(None),
            revoked_by: Set(None),
            created_at_ms: Set(1),
            updated_at_ms: Set(1),
            ..Default::default()
        }
        .insert(db)
        .await
    }
}
