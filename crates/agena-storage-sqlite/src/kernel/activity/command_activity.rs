use super::*;

/// Persist one operation per command invocation. Its terminal execution
/// anchors captures and delivery without scheduling a provider round.
pub(in crate::kernel) async fn begin_command(
    connection: &mut SqliteConnection,
    scope: Scope,
    origin: &PluginCommandActivityOrigin,
    activity: &BackgroundActivityStart,
    retention: ActivityOutputRetention,
) -> Result<Mutation> {
    if scope.execution != origin.invocation
        || scope.generation != AttemptGeneration::INITIAL
        || [&origin.plugin_id, &origin.command_id]
            .into_iter()
            .any(|name| {
                name.trim().is_empty() || name.len() > 256 || name.chars().any(char::is_control)
            })
    {
        return Err(invalid("invalid plugin command activity origin"));
    }
    reads::metadata(connection, scope.session).await?;
    let existing = sqlx::query(
        "SELECT e.id, e.round_id, e.tool_id, e.body_json, e.kind, e.origin_session_id,
                x.session_id, x.current_generation, a.phase
         FROM agena_executions x
         JOIN agena_entries e ON e.id=x.message_id
         JOIN agena_attempts a ON a.execution_id=x.id AND a.generation=x.current_generation
         WHERE x.id=?",
    )
    .bind(scope.execution.to_string())
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?;
    let operation = if let Some(row) = existing {
        let owner: i64 = row.try_get("session_id").map_err(storage)?;
        let entry_owner: i64 = row.try_get("origin_session_id").map_err(storage)?;
        let generation: i64 = row.try_get("current_generation").map_err(storage)?;
        let phase: &str = row.try_get("phase").map_err(storage)?;
        let kind: &str = row.try_get("kind").map_err(storage)?;
        if owner != scope.session.get()
            || entry_owner != owner
            || generation != scope.generation.get()
            || phase != "succeeded"
            || kind != "notice"
        {
            return Err(KernelError::Ownership);
        }
        let body: &str = row.try_get("body_json").map_err(storage)?;
        let EntryPayload::PluginCommand {
            plugin_id,
            command_id,
            operation,
        } = decode(body)?
        else {
            return Err(KernelError::Ownership);
        };
        if plugin_id != origin.plugin_id
            || command_id != origin.command_id
            || operation.get() != row.try_get::<i64, _>("tool_id").map_err(storage)?
        {
            return Err(KernelError::Ownership);
        }
        ActivityOrigin {
            scope,
            round: RoundId::try_from(row.try_get::<i64, _>("round_id").map_err(storage)?)
                .map_err(storage)?,
            tool: operation,
            entry: row.try_get("id").map_err(storage)?,
        }
    } else {
        create_operation(connection, scope, origin).await?
    };
    let anchor = EntryId::try_from(operation.entry).map_err(storage)?;
    super::super::lifecycle::boundary(connection, scope.session, anchor).await?;
    let id = insert(connection, operation, activity, Some(retention)).await?;
    Ok(Mutation::PluginCommandActivity { anchor, id })
}

async fn create_operation(
    connection: &mut SqliteConnection,
    scope: Scope,
    origin: &PluginCommandActivityOrigin,
) -> Result<ActivityOrigin> {
    let entry = commands::next_entry_id(connection).await?;
    let now = now_ms();
    let execution = scope.execution.to_string();
    let session = scope.session.get();
    sqlx::query(
        "INSERT INTO agena_executions (id,session_id,message_id,current_generation,created_at_ms)
         VALUES (?,?,?,1,?)",
    )
    .bind(&execution)
    .bind(session)
    .bind(entry)
    .bind(now)
    .execute(&mut *connection)
    .await
    .map_err(sql_error)?;
    sqlx::query(
        "INSERT INTO agena_attempts (execution_id,generation,session_id,phase,started_at_ms,finished_at_ms)
         VALUES (?,1,?,'succeeded',?,?)",
    )
    .bind(&execution).bind(session).bind(now).bind(now)
    .execute(&mut *connection).await.map_err(sql_error)?;
    let round: i64 = sqlx::query_scalar(
        "INSERT INTO agena_rounds (execution_id,generation,session_id,ordinal,phase,input_metadata_json,created_at_ms,finished_at_ms)
         VALUES (?,1,?,0,'succeeded','{}',?,?) RETURNING id",
    )
    .bind(&execution).bind(session).bind(now).bind(now)
    .fetch_one(&mut *connection).await.map_err(sql_error)?;
    // The operation row gives captured output the same lifetime and foreign
    // keys as output from tools. The entry below determines its origin.
    let tool: i64 = sqlx::query_scalar(
        "INSERT INTO agena_tool_calls (round_id,execution_id,generation,call_id,name,input_json,phase,result_json,created_at_ms,finished_at_ms)
         VALUES (?,?,1,?,?,'{}','succeeded','{}',?,?) RETURNING id",
    )
    .bind(round).bind(&execution).bind(&execution)
    .bind(format!("{}:{}", origin.plugin_id, origin.command_id))
    .bind(now).bind(now).fetch_one(&mut *connection).await.map_err(sql_error)?;
    let body = encode(&EntryPayload::PluginCommand {
        plugin_id: origin.plugin_id.clone(),
        command_id: origin.command_id.clone(),
        operation: ToolId::try_from(tool).map_err(storage)?,
    })?;
    sqlx::query(
        "INSERT INTO agena_entries (id,origin_session_id,execution_id,generation,message_id,kind,body_json,round_id,tool_id,created_at_ms)
         VALUES (?,?,?,1,?,'notice',?,?,?,?)",
    )
    .bind(entry).bind(session).bind(&execution).bind(entry).bind(body)
    .bind(round).bind(tool).bind(now).execute(&mut *connection).await.map_err(sql_error)?;
    commands::append_history(connection, scope.session, entry).await?;
    sqlx::query("UPDATE agena_sessions SET updated_at_ms=MAX(?,updated_at_ms+1) WHERE id=?")
        .bind(now)
        .bind(session)
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    Ok(ActivityOrigin {
        scope,
        round: RoundId::try_from(round).map_err(storage)?,
        tool: ToolId::try_from(tool).map_err(storage)?,
        entry,
    })
}
