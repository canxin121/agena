use sea_orm::sea_query::{
    Condition, ConditionalStatement, Expr, Index, IndexCreateStatement, TableCreateStatement,
};
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, DbErr, Schema, Statement,
    TransactionTrait,
};

use crate::db::{entities, event_entity};

/// Create the one current development schema.
///
/// There are deliberately no schema versions or migrations. If a development
/// database no longer matches these definitions, delete it and start again.
pub async fn create(db: &DatabaseConnection) -> Result<(), DbErr> {
    let backend = db.get_database_backend();
    if backend != DatabaseBackend::Sqlite {
        return Err(DbErr::Custom("Agena currently requires SQLite".to_owned()));
    }

    ensure_sqlite_foreign_keys(db).await?;

    let schema = Schema::new(backend);
    let txn = db.begin().await?;

    for create in table_definitions(&schema) {
        txn.execute(backend.build(&create)).await?;
    }
    for index in index_definitions() {
        txn.execute(backend.build(&index)).await?;
    }
    install_invariant_triggers(&txn).await?;

    txn.commit().await
}

async fn ensure_sqlite_foreign_keys(db: &DatabaseConnection) -> Result<(), DbErr> {
    let row = db
        .query_one(Statement::from_string(
            DatabaseBackend::Sqlite,
            "PRAGMA foreign_keys".to_owned(),
        ))
        .await?
        .ok_or_else(|| DbErr::Custom("SQLite did not return PRAGMA foreign_keys".to_owned()))?;
    let enabled: i64 = row.try_get("", "foreign_keys")?;
    if enabled == 1 {
        Ok(())
    } else {
        Err(DbErr::Custom(
            "SQLite foreign-key enforcement must be enabled for Agena".to_owned(),
        ))
    }
}

#[cfg(test)]
async fn sqlite_table_exists(db: &DatabaseConnection, table: &str) -> Result<bool, DbErr> {
    let row = db
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT 1 AS present FROM sqlite_master WHERE type = 'table' AND name = ? LIMIT 1",
            [table.into()],
        ))
        .await?;
    Ok(row.is_some())
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
            .create_table_from_entity(entities::session_lineage::Entity)
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
            .name("idx_agena_session_lineage_kind")
            .table(entities::session_lineage::Entity)
            .col(entities::session_lineage::Column::RelationKind)
            .col(entities::session_lineage::Column::SessionId)
            .if_not_exists()
            .to_owned(),
        Index::create()
            .name("idx_agena_session_lineage_task")
            .table(entities::session_lineage::Entity)
            .col(entities::session_lineage::Column::TaskId)
            .col(entities::session_lineage::Column::SessionId)
            .cond_where(
                Condition::all()
                    .add(Expr::col(entities::session_lineage::Column::TaskId).is_not_null()),
            )
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
            .name("uq_agena_events_session_seq")
            .table(event_entity::Entity)
            .col(event_entity::Column::SessionId)
            .col(event_entity::Column::SeqSession)
            .unique()
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
            .name("idx_agena_activity_messages_session_turn")
            .table(entities::activity_message::Entity)
            .col(entities::activity_message::Column::SessionId)
            .col(entities::activity_message::Column::TurnId)
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
            .name("uq_agena_activity_parts_operation_identity")
            .table(entities::activity_part::Entity)
            .col(entities::activity_part::Column::MessageId)
            .col(entities::activity_part::Column::Kind)
            .col(entities::activity_part::Column::OperationId)
            .unique()
            .cond_where(
                Condition::all()
                    .add(Expr::col(entities::activity_part::Column::OperationId).is_not_null()),
            )
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

async fn install_invariant_triggers<C>(db: &C) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    let backend = db.get_database_backend();
    for sql in [
        "CREATE TRIGGER IF NOT EXISTS agena_sessions_hierarchy_insert_valid \
         BEFORE INSERT ON agena_sessions \
         WHEN (NEW.parent_id IS NULL AND (NEW.depth != 0 OR NEW.root_id != 0)) \
           OR (NEW.parent_id IS NOT NULL AND NOT EXISTS ( \
             SELECT 1 FROM agena_sessions parent \
             WHERE parent.id = NEW.parent_id \
               AND parent.workspace_id = NEW.workspace_id \
               AND parent.lifecycle_state = 'ready' \
               AND NEW.depth = parent.depth + 1 \
               AND NEW.root_id = parent.root_id \
           )) \
         BEGIN SELECT RAISE(ABORT, 'invalid session hierarchy'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_sessions_hierarchy_immutable \
         BEFORE UPDATE OF parent_id, root_id, depth ON agena_sessions \
         WHEN (OLD.parent_id IS NOT NEW.parent_id OR OLD.root_id != NEW.root_id OR OLD.depth != NEW.depth) \
           AND NOT (OLD.parent_id IS NULL AND NEW.parent_id IS NULL AND OLD.root_id = 0 AND NEW.root_id = NEW.id AND OLD.depth = 0 AND NEW.depth = 0) \
         BEGIN SELECT RAISE(ABORT, 'session hierarchy is immutable'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_sessions_root_finalize \
         AFTER INSERT ON agena_sessions \
         WHEN NEW.parent_id IS NULL AND NEW.root_id = 0 \
         BEGIN UPDATE agena_sessions SET root_id = NEW.id WHERE id = NEW.id; END",
        "CREATE TRIGGER IF NOT EXISTS agena_sessions_lifecycle_insert_valid \
         BEFORE INSERT ON agena_sessions \
         WHEN NEW.lifecycle_state NOT IN ('creating', 'ready') \
           OR NEW.creation_error IS NOT NULL \
         BEGIN SELECT RAISE(ABORT, 'invalid session lifecycle state'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_sessions_lifecycle_update_valid \
         BEFORE UPDATE OF lifecycle_state ON agena_sessions \
         WHEN NEW.lifecycle_state NOT IN ('creating', 'ready', 'failed') \
         BEGIN SELECT RAISE(ABORT, 'invalid session lifecycle state'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_sessions_lifecycle_transition_valid \
         BEFORE UPDATE OF lifecycle_state ON agena_sessions \
         WHEN OLD.lifecycle_state != NEW.lifecycle_state \
           AND NOT (OLD.lifecycle_state = 'creating' AND NEW.lifecycle_state IN ('ready', 'failed')) \
         BEGIN SELECT RAISE(ABORT, 'invalid session lifecycle transition'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_sessions_creation_error_shape \
         BEFORE UPDATE OF lifecycle_state, creation_error ON agena_sessions \
         WHEN (NEW.lifecycle_state = 'failed' AND (NEW.creation_error IS NULL OR length(trim(NEW.creation_error)) = 0)) \
           OR (NEW.lifecycle_state != 'failed' AND NEW.creation_error IS NOT NULL) \
         BEGIN SELECT RAISE(ABORT, 'creation error must describe only failed sessions'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_session_lineage_shape_insert_valid \
         BEFORE INSERT ON agena_session_lineage \
         WHEN NEW.relation_kind NOT IN ('child', 'fork', 'rewind', 'subagent') \
           OR NOT EXISTS (SELECT 1 FROM agena_sessions s WHERE s.id = NEW.session_id AND s.parent_id IS NOT NULL) \
           OR (NEW.relation_kind = 'subagent' AND (NEW.task_id IS NULL OR NEW.subtask_status IS NULL)) \
           OR (NEW.relation_kind = 'subagent' AND (length(trim(NEW.task_id)) = 0 OR NEW.subtask_status NOT IN ('created', 'running', 'completed', 'failed', 'cancelled', 'timed_out', 'interrupted'))) \
           OR (NEW.relation_kind = 'subagent' AND ( \
             (NEW.subtask_status = 'created' AND (NEW.subtask_started_at_ms IS NOT NULL OR NEW.subtask_finished_at_ms IS NOT NULL OR NEW.subtask_error IS NOT NULL)) \
             OR (NEW.subtask_status = 'running' AND (NEW.subtask_started_at_ms IS NULL OR NEW.subtask_finished_at_ms IS NOT NULL OR NEW.subtask_error IS NOT NULL)) \
             OR (NEW.subtask_status = 'completed' AND (NEW.subtask_started_at_ms IS NULL OR NEW.subtask_finished_at_ms IS NULL OR NEW.subtask_error IS NOT NULL)) \
             OR (NEW.subtask_status IN ('failed', 'cancelled', 'timed_out', 'interrupted') AND (NEW.subtask_started_at_ms IS NULL OR NEW.subtask_finished_at_ms IS NULL OR NEW.subtask_error IS NULL OR length(trim(NEW.subtask_error)) = 0)) \
             OR (NEW.subtask_started_at_ms IS NOT NULL AND NEW.subtask_started_at_ms < 0) \
             OR (NEW.subtask_finished_at_ms IS NOT NULL AND NEW.subtask_finished_at_ms < NEW.subtask_started_at_ms) \
           )) \
           OR (NEW.relation_kind != 'subagent' AND (NEW.task_id IS NOT NULL OR NEW.subtask_status IS NOT NULL OR NEW.subtask_started_at_ms IS NOT NULL OR NEW.subtask_finished_at_ms IS NOT NULL OR NEW.subtask_error IS NOT NULL)) \
           OR (NEW.relation_kind IN ('fork', 'rewind') AND NEW.source_cutoff_seq_global IS NULL) \
           OR NEW.source_cutoff_seq_global < 0 \
           OR NEW.source_message_id <= 0 \
           OR (NEW.relation_kind NOT IN ('fork', 'rewind') AND (NEW.source_cutoff_seq_global IS NOT NULL OR NEW.source_message_id IS NOT NULL)) \
           OR (NEW.relation_kind = 'rewind' AND NEW.source_message_id IS NULL) \
         BEGIN SELECT RAISE(ABORT, 'invalid session lineage'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_session_lineage_provenance_immutable \
         BEFORE UPDATE OF relation_kind, source_cutoff_seq_global, source_message_id, task_id ON agena_session_lineage \
         WHEN OLD.relation_kind IS NOT NEW.relation_kind \
           OR OLD.source_cutoff_seq_global IS NOT NEW.source_cutoff_seq_global \
           OR OLD.source_message_id IS NOT NEW.source_message_id \
           OR OLD.task_id IS NOT NEW.task_id \
         BEGIN SELECT RAISE(ABORT, 'session lineage provenance is immutable'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_session_lineage_subtask_status_valid \
         BEFORE UPDATE OF subtask_status, subtask_started_at_ms, subtask_finished_at_ms, subtask_error ON agena_session_lineage \
         WHEN NEW.relation_kind != 'subagent' \
           OR NEW.subtask_status NOT IN ('created', 'running', 'completed', 'failed', 'cancelled', 'timed_out', 'interrupted') \
           OR (NEW.subtask_status = 'created' AND (NEW.subtask_started_at_ms IS NOT NULL OR NEW.subtask_finished_at_ms IS NOT NULL OR NEW.subtask_error IS NOT NULL)) \
           OR (NEW.subtask_status = 'running' AND (NEW.subtask_started_at_ms IS NULL OR NEW.subtask_finished_at_ms IS NOT NULL OR NEW.subtask_error IS NOT NULL)) \
           OR (NEW.subtask_status = 'completed' AND (NEW.subtask_started_at_ms IS NULL OR NEW.subtask_finished_at_ms IS NULL OR NEW.subtask_error IS NOT NULL)) \
           OR (NEW.subtask_status IN ('failed', 'cancelled', 'timed_out', 'interrupted') AND (NEW.subtask_started_at_ms IS NULL OR NEW.subtask_finished_at_ms IS NULL OR NEW.subtask_error IS NULL OR length(trim(NEW.subtask_error)) = 0)) \
           OR (NEW.subtask_started_at_ms IS NOT NULL AND NEW.subtask_started_at_ms < 0) \
           OR (NEW.subtask_finished_at_ms IS NOT NULL AND NEW.subtask_finished_at_ms < NEW.subtask_started_at_ms) \
         BEGIN SELECT RAISE(ABORT, 'invalid delegated-task lifecycle'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_session_lineage_task_unique \
         BEFORE INSERT ON agena_session_lineage \
         WHEN NEW.task_id IS NOT NULL AND EXISTS ( \
             SELECT 1 FROM agena_session_lineage existing \
             JOIN agena_sessions existing_session ON existing_session.id = existing.session_id \
             JOIN agena_sessions new_session ON new_session.id = NEW.session_id \
             WHERE existing.task_id = NEW.task_id \
               AND existing_session.parent_id = new_session.parent_id \
         ) \
         BEGIN SELECT RAISE(ABORT, 'delegated task identity already exists for parent'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_activity_messages_identity_immutable \
         BEFORE UPDATE OF message_id, session_id, turn_id, role, created_at_ms ON agena_activity_messages \
         WHEN OLD.message_id != NEW.message_id \
           OR OLD.session_id != NEW.session_id \
           OR OLD.turn_id IS NOT NEW.turn_id \
           OR OLD.role != NEW.role \
           OR OLD.created_at_ms != NEW.created_at_ms \
         BEGIN SELECT RAISE(ABORT, 'message identity and ownership are immutable'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_activity_parts_identity_immutable \
         BEFORE UPDATE OF part_id, message_id, part_index, kind, operation_id, created_at_ms ON agena_activity_parts \
         WHEN OLD.part_id != NEW.part_id \
           OR OLD.message_id != NEW.message_id \
           OR OLD.part_index != NEW.part_index \
           OR OLD.kind != NEW.kind \
           OR OLD.operation_id IS NOT NEW.operation_id \
           OR OLD.created_at_ms != NEW.created_at_ms \
         BEGIN SELECT RAISE(ABORT, 'part identity and ownership are immutable'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_events_append_only \
         BEFORE UPDATE ON agena_events \
         BEGIN SELECT RAISE(ABORT, 'event log rows are append-only'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_events_scope_insert_valid \
         BEFORE INSERT ON agena_events \
         WHEN (NEW.session_id IS NULL) != (NEW.seq_session IS NULL) \
           OR (NEW.session_id IS NOT NULL AND NEW.workspace_id IS NOT NULL AND NOT EXISTS ( \
             SELECT 1 FROM agena_sessions s \
             WHERE s.id = NEW.session_id AND s.workspace_id = NEW.workspace_id \
           )) \
         BEGIN SELECT RAISE(ABORT, 'invalid session event scope or workspace ownership'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_activity_messages_shape_insert_valid \
         BEFORE INSERT ON agena_activity_messages \
         WHEN NEW.part_count < 0 \
         BEGIN SELECT RAISE(ABORT, 'message part count cannot be negative'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_activity_messages_shape_update_valid \
         BEFORE UPDATE OF part_count ON agena_activity_messages \
         WHEN NEW.part_count < 0 \
         BEGIN SELECT RAISE(ABORT, 'message part count cannot be negative'); END",
        "CREATE TRIGGER IF NOT EXISTS agena_activity_parts_shape_insert_valid \
         BEFORE INSERT ON agena_activity_parts \
         WHEN NEW.part_index < 0 \
         BEGIN SELECT RAISE(ABORT, 'part index cannot be negative'); END",
    ] {
        db.execute(Statement::from_string(backend, sql.to_owned()))
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use sea_orm::{ConnectionTrait, Database};

    use super::*;

    #[tokio::test]
    async fn current_schema_creation_is_idempotent_and_enforces_foreign_keys() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        create(&db).await.expect("first schema creation");
        create(&db).await.expect("idempotent schema creation");

        ensure_sqlite_foreign_keys(&db)
            .await
            .expect("foreign keys enabled");
        assert!(
            sqlite_table_exists(&db, "agena_session_lineage")
                .await
                .expect("lineage table")
        );
        let row = db
            .query_one(Statement::from_string(
                DatabaseBackend::Sqlite,
                "PRAGMA user_version".to_owned(),
            ))
            .await
            .expect("read user_version")
            .expect("user_version row");
        assert_eq!(
            row.try_get::<i64>("", "user_version")
                .expect("user_version value"),
            0,
            "schema creation must not write a database version marker"
        );
    }
}
