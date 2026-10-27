use agena_domain::{PermissionMode, PermissionScope};
use agena_storage::backend::kernel::Scope;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, TransactionTrait};

use super::*;

pub(super) const SOURCE_SIZE_SQL: &str = "SELECT count(*) AS sources,COALESCE(sum(1024+6*capture_error_bytes),0) AS bytes FROM (SELECT c.capture_error_bytes FROM agena_round_inputs input JOIN agena_entries e ON e.id=input.entry_id JOIN agena_content_streams c ON c.entry_id=input.entry_id OR (e.kind='compaction' AND c.id=e.content_id) WHERE input.round_id=? LIMIT 10001)";
pub(super) const CAPTURE_SOURCES_SQL: &str = "INSERT INTO agena_round_input_sources (round_id,entry_id,content_id,head_json) SELECT ?,input.entry_id,c.id,json_object('id',c.id,'entry',c.entry_id,'epoch',c.epoch,'kind',c.kind,'activity_id',c.activity_id,'ordinal',c.ordinal,'capture_loss',json_object('bytes',c.lost_bytes,'packets',c.lost_packets,'committed_bytes',c.lost_bytes,'committed_packets',c.lost_packets),'accepted',c.committed_head,'committed',c.committed_head,'retained_start',c.retained_start,'state',c.capture_state,'capture_error',c.capture_error) FROM agena_round_inputs input JOIN agena_entries e ON e.id=input.entry_id JOIN agena_content_streams c ON c.entry_id=input.entry_id OR (e.kind='compaction' AND c.id=e.content_id) WHERE input.round_id=? LIMIT 10001 ON CONFLICT(round_id,content_id) DO NOTHING";

pub(super) async fn apply(
    connection: &mut SqliteConnection,
    command: &Command,
) -> Result<Mutation> {
    if let Some(value) = super::lifecycle::apply(connection, command).await? {
        return Ok(value);
    }
    let now = now_ms();
    match command {
        Command::RecordCompaction { .. }
        | Command::PresentInteraction { .. }
        | Command::CreateConfigured { .. }
        | Command::Configure { .. }
        | Command::FinishAgent { .. }
        | Command::Fork { .. }
        | Command::Withdraw { .. }
        | Command::DeleteSession { .. }
        | Command::Continue { .. }
        | Command::Notice { .. }
        | Command::Deliver { .. }
        | Command::DeliverToExecution { .. }
        | Command::ResumeDelivery { .. }
        | Command::ConsumeDelivery { .. } => unreachable!("lifecycle command handled above"),
        Command::CreateSession {
            workspace_id,
            title,
        } => {
            if *workspace_id <= 0 || title.len() > 4096 {
                return Err(invalid("invalid workspace or title"));
            }
            let id: i64 = sqlx::query_scalar("INSERT INTO agena_sessions (workspace_id,title,created_at_ms,updated_at_ms) VALUES (?,?,?,?) RETURNING id")
                .bind(workspace_id).bind(title).bind(now).bind(now).fetch_one(&mut *connection).await.map_err(sql_error)?;
            sqlx::query("INSERT INTO agena_session_statistics (session_id) VALUES (?)")
                .bind(id)
                .execute(&mut *connection)
                .await
                .map_err(sql_error)?;
            sqlx::query("INSERT INTO agena_prompt_windows (session_id) VALUES (?)")
                .bind(id)
                .execute(&mut *connection)
                .await
                .map_err(sql_error)?;
            Ok(Mutation::Session(Box::new(
                reads::metadata(connection, SessionId::try_from(id).map_err(storage)?).await?,
            )))
        }
        Command::Metadata { session, change } => {
            let mut metadata = reads::metadata(connection, *session).await?;
            let original = metadata.clone();
            let mut config = lifecycle::configuration(connection, *session).await?;
            let original_config = config.clone();
            if let Some(update) = &change.automatic_title {
                if change.title.is_some()
                    || change.favorite.is_some()
                    || change.pinned.is_some()
                    || change.auto_title.is_some()
                    || change.additional_system_prompt.is_some()
                {
                    return Err(invalid(
                        "automatic title cannot be combined with manual metadata",
                    ));
                }
                if metadata.metadata_version != update.expected_metadata_version
                    || metadata.membership_generation != update.expected_membership_generation
                    || config.title_state.source == SessionTitleSource::Manual
                    || config.relation_kind.is_subagent()
                {
                    return Err(KernelError::Conflict(
                        "background title was superseded".into(),
                    ));
                }
                let messages: i64 = sqlx::query_scalar(
                    "SELECT user_message_count FROM agena_session_statistics WHERE session_id=?",
                )
                .bind(session.get())
                .fetch_one(&mut *connection)
                .await
                .map_err(sql_error)?;
                if u64::try_from(messages).ok() != Some(update.user_message_count)
                    || update.user_message_count == 0
                {
                    return Err(KernelError::Conflict("title context has changed".into()));
                }
                let last = sqlx::query("SELECT e.id,e.revision FROM agena_session_segments s JOIN agena_segment_entries m ON m.segment_id=s.segment_id AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal JOIN agena_entries e ON e.id=m.entry_id WHERE s.session_id=? AND e.kind='user' ORDER BY s.sequence DESC,m.ordinal DESC LIMIT 1")
                    .bind(session.get()).fetch_optional(&mut *connection).await.map_err(sql_error)?;
                if last.as_ref().is_none_or(|row| {
                    row.try_get::<i64, _>("id").ok() != Some(update.last_user_message.get())
                        || row.try_get::<i64, _>("revision").ok()
                            != Some(update.last_user_revision.get())
                }) {
                    return Err(KernelError::Conflict("title input was edited".into()));
                }
                if let Some(title) = &update.title {
                    if title.trim().is_empty()
                        || title.len() > 4096
                        || title.chars().any(char::is_control)
                    {
                        return Err(invalid("invalid automatic title"));
                    }
                    metadata.title.clone_from(title);
                    config.title_state.source = SessionTitleSource::Automatic;
                }
                config.title_state.checked_user_messages = update.user_message_count;
                config.title_state.checked_at_ms = now;
                config.title_state.calibrated |= update.user_message_count >= 3;
            }
            if let Some(title) = &change.title {
                if title.len() > 4096 {
                    return Err(invalid("title exceeds 4096 bytes"));
                }
                metadata.title.clone_from(title);
                // Even assigning the same name is an explicit user decision
                // and must fence a generation already in flight.
                config.title_state.source = SessionTitleSource::Manual;
            }
            if let Some(value) = change.favorite {
                metadata.favorite = value;
            }
            if let Some(value) = change.pinned {
                metadata.pinned = value;
            }
            if let Some(enabled) = change.auto_title {
                config.title_state = if enabled {
                    SessionTitleState::automatic()
                } else {
                    SessionTitleState::default()
                };
            }
            if let Some(value) = &change.additional_system_prompt {
                config.additional_system_prompt.clone_from(value);
            }
            if metadata != original || config != original_config {
                let configuration = encode(&config)?;
                if configuration.len() > 65536 {
                    return Err(invalid("session configuration exceeds 64 KiB"));
                }
                metadata.metadata_version = metadata.metadata_version.next().map_err(storage)?;
                sqlx::query("UPDATE agena_sessions SET title=?,favorite=?,pinned=?,configuration_json=?,metadata_version=?,updated_at_ms=MAX(?,updated_at_ms+1) WHERE id=?")
                    .bind(&metadata.title).bind(metadata.favorite).bind(metadata.pinned).bind(configuration).bind(metadata.metadata_version.get()).bind(now).bind(session.get())
                    .execute(&mut *connection).await.map_err(sql_error)?;
            }
            Ok(Mutation::Session(Box::new(metadata)))
        }
        Command::Selection { session, change } => {
            let mut metadata = reads::metadata(connection, *session).await?;
            let mut selection = metadata.selection.clone().unwrap_or_default();
            match change {
                SelectionChange::Model(model) => {
                    selection.provider = model.as_ref().map(|m| m.provider_id.to_string());
                    selection.adapter = model
                        .as_ref()
                        .and_then(|m| m.adapter_id.as_ref().map(ToString::to_string));
                    selection.model = model.as_ref().map(|m| m.model_id.to_string());
                }
                SelectionChange::ThinkingMode(value) => selection.thinking_mode.clone_from(value),
                SelectionChange::SpeedMode(value) => selection.speed_mode.clone_from(value),
                SelectionChange::Verbosity(value) => selection.verbosity.clone_from(value),
                SelectionChange::ParallelToolCalls(value) => selection.parallel_tool_calls = *value,
            }
            selection.normalize();
            let selection = (!selection.is_empty()).then_some(selection);
            if selection != metadata.selection {
                metadata.metadata_version = metadata.metadata_version.next().map_err(storage)?;
                metadata.selection = selection;
                sqlx::query(
                    "UPDATE agena_sessions SET selection_json=?,metadata_version=?,updated_at_ms=MAX(?,updated_at_ms+1) WHERE id=?",
                )
                .bind(metadata.selection.as_ref().map(encode).transpose()?)
                .bind(metadata.metadata_version.get())
                .bind(now)
                .bind(session.get())
                .execute(&mut *connection)
                .await
                .map_err(sql_error)?;
            }
            Ok(Mutation::Session(Box::new(metadata)))
        }
        Command::Send {
            session,
            document,
            blobs,
        } => {
            reads::metadata(connection, *session).await?;
            if document.is_empty() {
                return Err(invalid("message is empty"));
            }
            // Paused supervisors release admission permits. Bound durable
            // paused input too, so the coherent control projection stays
            // complete and retaining a queue cannot consume unbounded memory.
            if super::lifecycle::configuration(connection, *session)
                .await?
                .message_queue_paused
            {
                let active: i64 = sqlx::query_scalar(
                    "SELECT queued_executions+running_executions FROM agena_session_statistics WHERE session_id=?",
                )
                .bind(session.get())
                .fetch_one(&mut *connection)
                .await
                .map_err(sql_error)?;
                if active >= 128 {
                    return Err(KernelError::Busy);
                }
            }
            let body = encode(document)?;
            if body.len() > 256 * 1024 {
                return Err(invalid("composer document exceeds 256 KiB"));
            }
            let execution = ExecutionId::new();
            let entry: i64 = sqlx::query_scalar("INSERT INTO agena_entries (origin_session_id,execution_id,generation,kind,body_json,created_at_ms) VALUES (?,?,1,'user',?,?) RETURNING id")
                .bind(session.get()).bind(execution.to_string()).bind(body).bind(now)
                .fetch_one(&mut *connection).await.map_err(sql_error)?;
            sqlx::query("INSERT INTO agena_executions (id,session_id,message_id,current_generation,created_at_ms) VALUES (?,?,?,1,?)")
                .bind(execution.to_string()).bind(session.get()).bind(entry).bind(now).execute(&mut *connection).await.map_err(sql_error)?;
            sqlx::query("INSERT INTO agena_attempts (execution_id,generation,session_id,phase) VALUES (?,1,?,'queued')")
                .bind(execution.to_string()).bind(session.get()).execute(&mut *connection).await.map_err(sql_error)?;
            append_history(connection, *session, &[entry]).await?;
            super::blobs::publish(connection, *session, entry, blobs).await?;
            sqlx::query("UPDATE agena_session_statistics SET user_message_count=user_message_count+1,queued_executions=queued_executions+1,last_message_at_ms=MAX(?,COALESCE(last_message_at_ms,?)) WHERE session_id=?")
                .bind(now).bind(now).bind(session.get()).execute(&mut *connection).await.map_err(sql_error)?;
            sqlx::query(
                "UPDATE agena_sessions SET updated_at_ms=MAX(?,updated_at_ms+1) WHERE id=?",
            )
            .bind(now)
            .bind(session.get())
            .execute(&mut *connection)
            .await
            .map_err(sql_error)?;
            Ok(Mutation::Execution(
                reads::execution(connection, *session, execution).await?.0,
            ))
        }
        Command::EditQueued {
            session,
            execution,
            expected_revision,
            document,
            blobs,
        } => {
            let (view, _) = reads::execution(connection, *session, *execution).await?;
            let (revision, started): (i64, bool) = sqlx::query_as("SELECT revision,EXISTS(SELECT 1 FROM agena_attempts a WHERE a.execution_id=? AND a.started_at_ms IS NOT NULL) FROM agena_entries WHERE id=?")
                .bind(execution.to_string()).bind(view.message_id.get()).fetch_one(&mut *connection).await.map_err(sql_error)?;
            if view.phase != ExecutionPhase::Queued || started {
                return Err(KernelError::Conflict(
                    "the message has already started or been removed".into(),
                ));
            }
            if revision != expected_revision.get() {
                return Err(KernelError::Conflict(
                    "the queued message was changed by another client".into(),
                ));
            }
            let frozen: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agena_round_inputs WHERE entry_id=?) OR EXISTS(SELECT 1 FROM agena_segment_entries m JOIN agena_session_segments s ON s.segment_id=m.segment_id WHERE m.entry_id=? AND s.session_id!=? AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal)")
                .bind(view.message_id.get()).bind(view.message_id.get()).bind(session.get()).fetch_one(&mut *connection).await.map_err(sql_error)?;
            if frozen {
                return Err(KernelError::Conflict(
                    "the input is already shared with another conversation or model round".into(),
                ));
            }
            let body = encode(document)?;
            if document.is_empty() || body.len() > 256 * 1024 {
                return Err(invalid("message must be nonempty and within 256 KiB"));
            }
            sqlx::query("UPDATE agena_entries SET body_json=?,revision=revision+1 WHERE id=?")
                .bind(body)
                .bind(view.message_id.get())
                .execute(&mut *connection)
                .await
                .map_err(sql_error)?;
            sqlx::query("DELETE FROM agena_blob_references WHERE entry_id=?")
                .bind(view.message_id.get())
                .execute(&mut *connection)
                .await
                .map_err(sql_error)?;
            super::blobs::publish(connection, *session, view.message_id.get(), blobs).await?;
            sqlx::query(
                "UPDATE agena_sessions SET updated_at_ms=MAX(?,updated_at_ms+1) WHERE id=?",
            )
            .bind(now)
            .bind(session.get())
            .execute(&mut *connection)
            .await
            .map_err(sql_error)?;
            Ok(Mutation::Execution(view))
        }
        Command::Claim {
            session,
            execution,
            epoch,
        }
        | Command::ClaimNext {
            session,
            execution,
            epoch,
        } => {
            // This check and the claim share the serialized writer transaction.
            // A stop must persist pause before releasing the current execution.
            if super::lifecycle::configuration(connection, *session)
                .await?
                .message_queue_paused
            {
                return Err(KernelError::QueuePaused);
            }
            let (view, _) = reads::execution(connection, *session, *execution).await?;
            if matches!(command, Command::ClaimNext { .. }) {
                let blocked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agena_executions x JOIN agena_attempts a ON a.execution_id=x.id AND a.generation=x.current_generation WHERE x.session_id=? AND (a.phase='running' OR (a.phase='queued' AND x.message_id<?)))")
                    .bind(session.get()).bind(view.message_id.get()).fetch_one(&mut *connection).await.map_err(sql_error)?;
                if blocked {
                    return Err(KernelError::QueueOutOfOrder);
                }
            }
            let mut state = ExecutionState::from_stored(view.generation, view.phase);
            state.claim()?;
            sqlx::query("UPDATE agena_attempts SET phase='running',owner_epoch=?,started_at_ms=? WHERE execution_id=? AND generation=?")
                .bind(epoch.to_string()).bind(now).bind(execution.to_string()).bind(view.generation.get())
                .execute(&mut *connection).await.map_err(sql_error)?;
            sqlx::query("UPDATE agena_session_statistics SET queued_executions=queued_executions-1,running_executions=running_executions+1 WHERE session_id=?")
                .bind(session.get()).execute(&mut *connection).await.map_err(sql_error)?;
            Ok(Mutation::Execution(
                reads::execution(connection, *session, *execution).await?.0,
            ))
        }
        Command::Cancel { session, execution } => {
            let (view, _) = reads::execution(connection, *session, *execution).await?;
            finish(connection, view, ExecutionPhase::Cancelled)
                .await
                .map(Mutation::Execution)
        }
        Command::CancelQueued { session, execution } => {
            let (view, _) = reads::execution(connection, *session, *execution).await?;
            if view.phase != ExecutionPhase::Queued {
                return Err(KernelError::Conflict(
                    "the queued message has already started or been removed".into(),
                ));
            }
            finish(connection, view, ExecutionPhase::Cancelled)
                .await
                .map(Mutation::Execution)
        }
        Command::ClearQueued { session } => {
            reads::metadata(connection, *session).await?;
            let ids: Vec<String> = sqlx::query_scalar("SELECT x.id FROM agena_executions x JOIN agena_attempts a ON a.execution_id=x.id AND a.generation=x.current_generation WHERE x.session_id=? AND a.phase='queued' ORDER BY x.message_id")
                .bind(session.get()).fetch_all(&mut *connection).await.map_err(sql_error)?;
            for id in &ids {
                let execution = id.parse().map_err(storage)?;
                let (view, _) = reads::execution(connection, *session, execution).await?;
                finish(connection, view, ExecutionPhase::Cancelled).await?;
            }
            Ok(Mutation::ClearedQueued(ids.len() as u64))
        }
        Command::Retry { session, execution } => {
            let (view, _) = reads::execution(connection, *session, *execution).await?;
            let mut state = ExecutionState::from_stored(view.generation, view.phase);
            let next = state.retry()?;
            sqlx::query("UPDATE agena_entries SET revision=revision+1 WHERE id=?")
                .bind(view.message_id.get())
                .execute(&mut *connection)
                .await
                .map_err(sql_error)?;
            sqlx::query("INSERT INTO agena_attempts (execution_id,generation,session_id,phase) VALUES (?,?,?,'queued')")
                .bind(execution.to_string()).bind(next.get()).bind(session.get()).execute(&mut *connection).await.map_err(sql_error)?;
            sqlx::query("UPDATE agena_executions SET current_generation=? WHERE id=?")
                .bind(next.get())
                .bind(execution.to_string())
                .execute(&mut *connection)
                .await
                .map_err(sql_error)?;
            sqlx::query("UPDATE agena_session_statistics SET queued_executions=queued_executions+1 WHERE session_id=?")
                .bind(session.get()).execute(&mut *connection).await.map_err(sql_error)?;
            Ok(Mutation::Execution(
                reads::execution(connection, *session, *execution).await?.0,
            ))
        }
        Command::BeginRound { scope, inputs } => begin_round(connection, *scope, inputs).await,
        Command::BeginCurrentRound { scope } => {
            let execution = check_scope(connection, *scope, true).await?;
            let window = super::input_window::select(
                connection,
                scope.session,
                execution.message_id,
                10000,
                false,
            )
            .await?;
            begin_round(connection, *scope, &window.entries).await
        }
        Command::BeginCompactionRound { scope, max_entries } => {
            let execution = check_scope(connection, *scope, true).await?;
            if !(2..=10000).contains(max_entries) {
                return Err(invalid("compaction prefix must admit 2-10000 entries"));
            }
            let window = super::input_window::select(
                connection,
                scope.session,
                execution.message_id,
                *max_entries,
                true,
            )
            .await?;
            let value = begin_round(connection, *scope, &window.entries).await?;
            let Mutation::Round(round) = value else {
                unreachable!("begin-round result")
            };
            sqlx::query("INSERT INTO agena_compaction_rounds (round_id,base_checkpoint_entry_id,rewrite_epoch) VALUES (?,?,?)")
                .bind(round.get()).bind(window.checkpoint).bind(window.rewrite_epoch).execute(connection).await.map_err(sql_error)?;
            Ok(Mutation::Round(round))
        }
        Command::CommitCompaction {
            scope,
            round,
            summary,
            through,
            provider_state,
        } => super::input_window::commit(
            connection,
            *scope,
            *round,
            *summary,
            *through,
            provider_state.as_ref(),
        )
        .await
        .map(Mutation::Checkpoint),
        Command::FinishRound {
            scope,
            round,
            provider_state,
            succeeded,
        } => {
            check_scope(connection, *scope, true).await?;
            let drafts: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM agena_tool_calls WHERE round_id=? AND phase='preparing'",
            )
            .bind(round.get())
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
            if *succeeded && drafts > 0 {
                return Err(KernelError::Conflict(
                    "provider round has unfinished tool arguments".into(),
                ));
            }
            if !*succeeded && drafts > 0 {
                sqlx::query("UPDATE agena_entries SET revision=revision+1 WHERE tool_id IN (SELECT id FROM agena_tool_calls WHERE round_id=? AND phase='preparing')")
                    .bind(round.get()).execute(&mut *connection).await.map_err(sql_error)?;
                sqlx::query("UPDATE agena_tool_calls SET phase='interrupted',result_json='null',finished_at_ms=? WHERE round_id=? AND phase='preparing'")
                    .bind(now).bind(round.get()).execute(&mut *connection).await.map_err(sql_error)?;
            }
            let phase = if *succeeded { "succeeded" } else { "failed" };
            let result = sqlx::query("UPDATE agena_rounds SET phase=?,provider_state_json=?,finished_at_ms=? WHERE id=? AND execution_id=? AND generation=? AND phase='running'")
                .bind(phase).bind(provider_state.as_ref().map(encode).transpose()?).bind(now)
                .bind(round.get()).bind(scope.execution.to_string()).bind(scope.generation.get())
                .execute(&mut *connection).await.map_err(sql_error)?;
            if result.rows_affected() != 1 {
                return Err(KernelError::Conflict(
                    "provider round is not running in this attempt".into(),
                ));
            }
            if *succeeded {
                super::stop::observe_round(connection, *scope, *round).await?;
            }
            Ok(Mutation::Round(*round))
        }
        Command::BeginTool {
            scope,
            round,
            tool,
            execution_kind,
            preparing,
        } => {
            let execution = check_scope(connection, *scope, true).await?;
            check_round(connection, *scope, *round).await?;
            let compaction: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM agena_compaction_rounds WHERE round_id=?)",
            )
            .bind(round.get())
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
            if compaction {
                return Err(KernelError::Conflict(
                    "compaction rounds cannot start tools".into(),
                ));
            }
            if tool.call_id.is_empty()
                || tool.name.is_empty()
                || tool.call_id.len() > 4096
                || tool.name.len() > 4096
            {
                return Err(invalid("invalid tool identity"));
            }
            if *preparing
                && (*execution_kind != ToolExecutionKind::Runtime || !tool.input.is_null())
            {
                return Err(invalid(
                    "tool draft must be a runtime call without executable input",
                ));
            }
            let id: i64 = sqlx::query_scalar("INSERT INTO agena_tool_calls (round_id,execution_id,generation,call_id,name,input_json,execution_kind,phase,created_at_ms) VALUES (?,?,?,?,?,?,?,?,?) RETURNING id")
                .bind(round.get()).bind(scope.execution.to_string()).bind(scope.generation.get()).bind(&tool.call_id).bind(&tool.name).bind(encode(&tool.input)?).bind(match execution_kind {ToolExecutionKind::Runtime=>"runtime",ToolExecutionKind::ProviderHosted=>"provider_hosted"}).bind(if *preparing {"preparing"} else {"running"}).bind(now)
                .fetch_one(&mut *connection).await.map_err(sql_error)?;
            let entry: i64 = sqlx::query_scalar("INSERT INTO agena_entries (origin_session_id,execution_id,generation,message_id,kind,round_id,tool_id,created_at_ms) VALUES (?,?,?,?,'tool',?,?,?) RETURNING id")
                .bind(scope.session.get()).bind(scope.execution.to_string()).bind(scope.generation.get()).bind(execution.message_id.get())
                .bind(round.get()).bind(id).bind(now).fetch_one(&mut *connection).await.map_err(sql_error)?;
            append_history(connection, scope.session, &[entry]).await?;
            Ok(Mutation::Tool(ToolId::try_from(id).map_err(storage)?))
        }
        Command::FinalizeTool { scope, tool, input } => {
            check_scope(connection, *scope, true).await?;
            let round: Option<i64>=sqlx::query_scalar("SELECT round_id FROM agena_tool_calls WHERE id=? AND execution_id=? AND generation=? AND execution_kind='runtime' AND phase='preparing'")
                .bind(tool.get()).bind(scope.execution.to_string()).bind(scope.generation.get()).fetch_optional(&mut *connection).await.map_err(sql_error)?;
            let round = round.ok_or_else(|| {
                KernelError::Conflict(
                    "tool arguments are no longer being assembled in this attempt".into(),
                )
            })?;
            check_round(
                connection,
                *scope,
                RoundId::try_from(round).map_err(storage)?,
            )
            .await?;
            sqlx::query("UPDATE agena_tool_calls SET input_json=?,phase='running' WHERE id=?")
                .bind(encode(input)?)
                .bind(tool.get())
                .execute(&mut *connection)
                .await
                .map_err(sql_error)?;
            sqlx::query("UPDATE agena_entries SET revision=revision+1 WHERE tool_id=?")
                .bind(tool.get())
                .execute(connection)
                .await
                .map_err(sql_error)?;
            Ok(Mutation::Tool(*tool))
        }
        Command::RequestInteraction {
            scope,
            tool,
            interaction,
        } => {
            let execution = check_scope(connection, *scope, true).await?;
            let round = check_tool(connection, *scope, *tool).await?;
            let kind: String =
                sqlx::query_scalar("SELECT execution_kind FROM agena_tool_calls WHERE id=?")
                    .bind(tool.get())
                    .fetch_one(&mut *connection)
                    .await
                    .map_err(sql_error)?;
            if kind != "runtime" {
                return Err(KernelError::Conflict(
                    "provider-hosted operations cannot request local interactions".into(),
                ));
            }
            let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM (SELECT id FROM agena_interactions WHERE tool_id=? LIMIT 128)").bind(tool.get()).fetch_one(&mut *connection).await.map_err(sql_error)?;
            if count >= 128 {
                return Err(invalid("tool interaction identity limit of 128 reached"));
            }
            let kind = match interaction {
                InteractionRequest::Permission { action_key, .. } => {
                    if action_key.is_empty() || action_key.len() > 4096 {
                        return Err(invalid("permission action key is empty or too long"));
                    }
                    "permission"
                }
                InteractionRequest::Input { request } => {
                    if request
                        .session_id
                        .is_some_and(|id| id != scope.session.get())
                    {
                        return Err(KernelError::Ownership);
                    }
                    "input"
                }
            };
            let id: i64 = sqlx::query_scalar("INSERT INTO agena_interactions (tool_id,session_id,execution_id,generation,request_kind,request_json,created_at_ms) VALUES (?,?,?,?,?,?,?) RETURNING id")
                .bind(tool.get()).bind(scope.session.get()).bind(scope.execution.to_string()).bind(scope.generation.get()).bind(kind).bind(encode(interaction)?).bind(now).fetch_one(&mut *connection).await.map_err(sql_error)?;
            let entry: i64 = sqlx::query_scalar("INSERT INTO agena_entries (origin_session_id,execution_id,generation,message_id,kind,round_id,tool_id,interaction_id,created_at_ms) VALUES (?,?,?,?,'interaction',?,?,?,?) RETURNING id")
                .bind(scope.session.get()).bind(scope.execution.to_string()).bind(scope.generation.get()).bind(execution.message_id.get())
                .bind(round.get()).bind(tool.get()).bind(id).bind(now).fetch_one(&mut *connection).await.map_err(sql_error)?;
            append_history(connection, scope.session, &[entry]).await?;
            Ok(Mutation::Interaction(
                InteractionId::try_from(id).map_err(storage)?,
            ))
        }
        Command::Reply {
            session,
            interaction,
            decision,
            remember,
        } => {
            reply(
                connection,
                *session,
                *interaction,
                decision,
                remember.as_ref(),
            )
            .await
        }
        Command::CompleteTool {
            scope,
            tool,
            result,
            blobs,
        } => {
            check_scope(connection, *scope, true).await?;
            check_tool(connection, *scope, *tool).await?;
            if matches!(result.phase, ToolPhase::Preparing | ToolPhase::Running) {
                return Err(invalid("complete requires a terminal tool outcome"));
            }
            let unresolved: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM agena_interactions WHERE tool_id=? AND decision_json IS NULL",
            )
            .bind(tool.get())
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
            if unresolved > 0 {
                return Err(KernelError::Conflict(
                    "tool has unresolved interactions".into(),
                ));
            }
            let entry: i64 =
                sqlx::query_scalar("SELECT id FROM agena_entries WHERE tool_id=? AND kind='tool'")
                    .bind(tool.get())
                    .fetch_one(&mut *connection)
                    .await
                    .map_err(sql_error)?;
            super::blobs::publish(connection, scope.session, entry, blobs).await?;
            sqlx::query(
                "UPDATE agena_tool_calls SET phase=?,result_json=?,finished_at_ms=? WHERE id=?",
            )
            .bind(enum_text(&result.phase)?)
            .bind(encode(&result.value)?)
            .bind(now)
            .bind(tool.get())
            .execute(&mut *connection)
            .await
            .map_err(sql_error)?;
            bump_tool_entries(connection, tool.get()).await?;
            Ok(Mutation::Tool(*tool))
        }
        Command::PrepareStop { scope, conditions } => {
            super::stop::prepare(connection, *scope, conditions).await
        }
        Command::Finish { scope, phase } => {
            let view = check_scope(connection, *scope, false).await?;
            if *phase == ExecutionPhase::Succeeded && !view.phase.is_terminal() {
                let pending: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agena_deliveries INDEXED BY idx_agena_delivery_execution WHERE target_execution_id=? AND state='pending')")
                    .bind(view.id.to_string()).fetch_one(&mut *connection).await.map_err(sql_error)?;
                if pending {
                    return Err(KernelError::PendingDelivery(view.id));
                }
            }
            finish(connection, view, *phase)
                .await
                .map(Mutation::Execution)
        }
        Command::BeginActivity {
            scope,
            tool,
            activity,
        } => super::activity::begin(connection, *scope, *tool, activity).await,
        Command::EndActivity { scope, outcome } => {
            super::activity::end(connection, *scope, outcome).await
        }
        Command::OpenActivityOutput { scope, kind } => {
            super::activity::open_output(connection, *scope, *kind).await
        }
        Command::OpenText {
            scope,
            round,
            tool,
            reasoning,
        } => {
            let execution = check_scope(connection, *scope, true).await?;
            if let Some(tool) = tool {
                if *reasoning || check_tool(connection, *scope, *tool).await? != *round {
                    return Err(KernelError::Ownership);
                }
            } else {
                check_round(connection, *scope, *round).await?;
            }
            let active: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM agena_content_streams WHERE capture_state='open'",
            )
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
            if active >= content::MAX_LIVE_SOURCES {
                return Err(KernelError::Busy);
            }
            let id = agena_domain::ContentId::new();
            let epoch = agena_domain::ContentEpoch::new();
            let entry: i64 = if let Some(tool) = tool {
                let entry = sqlx::query_scalar(
                    "SELECT id FROM agena_entries WHERE tool_id=? AND kind='tool'",
                )
                .bind(tool.get())
                .fetch_one(&mut *connection)
                .await
                .map_err(sql_error)?;
                content::bind_tool_entry(connection, entry).await?;
                entry
            } else {
                let kind = if *reasoning { "reasoning" } else { "text" };
                let entry: i64 = sqlx::query_scalar("INSERT INTO agena_entries (origin_session_id,execution_id,generation,message_id,kind,round_id,content_id,created_at_ms) VALUES (?,?,?,?,?,?,?,?) RETURNING id")
                    .bind(scope.session.get()).bind(scope.execution.to_string()).bind(scope.generation.get()).bind(execution.message_id.get())
                    .bind(kind).bind(round.get()).bind(id.to_string()).bind(now).fetch_one(&mut *connection).await.map_err(sql_error)?;
                append_history(connection, scope.session, &[entry]).await?;
                entry
            };
            sqlx::query("INSERT INTO agena_content_streams (id,entry_id,session_id,execution_id,generation,kind,epoch,capture_state) VALUES (?,?,?,?,?,'text',?,'open')")
                .bind(id.to_string()).bind(entry).bind(scope.session.get()).bind(scope.execution.to_string()).bind(scope.generation.get()).bind(epoch.to_string())
                .execute(&mut *connection).await.map_err(sql_error)?;
            Ok(Mutation::Content({
                content::assign_ordinal(connection, id).await?;
                content::stored_head(connection, id).await?
            }))
        }
        Command::OpenEvents {
            scope,
            round,
            tool,
            kind,
        } => content::events::open(connection, *scope, *round, *tool, *kind).await,
        Command::SealContent {
            scope,
            content,
            epoch,
            ending,
        } => {
            if ending
                .diagnostic()
                .is_some_and(|diagnostic| diagnostic.len() > 4096)
            {
                return Err(invalid("capture diagnostic exceeds 4096 bytes"));
            }
            super::activity::check_output(connection, *scope).await?;
            let owner = super::activity::owns_output(connection, *scope, *content, *epoch).await?;
            if !owner {
                return Err(KernelError::Ownership);
            }
            let head = content::stored_head(connection, *content).await?;
            if head.state != CaptureState::Open
                && (head.state != ending.state()
                    || head.capture_error.as_deref() != ending.diagnostic())
            {
                return Err(KernelError::Conflict(
                    "capture was already sealed differently".into(),
                ));
            }
            Ok(Mutation::Content(head))
        }
        Command::PrepareBlob { .. }
        | Command::PinBlob { .. }
        | Command::PinSnapshotBlob { .. }
        | Command::WriteBlob { .. }
        | Command::ReadyBlob { .. } => super::blobs::apply(connection, command).await,
        Command::PruneContent {
            session,
            content,
            before,
        } => content::prune(connection, *session, *content, *before)
            .await
            .map(Mutation::Retention),
    }
}

pub(super) fn enum_text<T: serde::Serialize>(value: &T) -> Result<String> {
    serde_json::to_value(value)
        .map_err(storage)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| storage("invalid internal enum encoding"))
}

pub(super) async fn check_scope(
    connection: &mut SqliteConnection,
    scope: Scope,
    running: bool,
) -> Result<ExecutionView> {
    let (view, epoch) = reads::execution(connection, scope.session, scope.execution).await?;
    if view.generation != scope.generation {
        return Err(KernelError::StaleAttempt);
    }
    if epoch != Some(scope.epoch) {
        return Err(KernelError::Ownership);
    }
    if running && view.phase != ExecutionPhase::Running {
        return Err(KernelError::Conflict("attempt is no longer running".into()));
    }
    Ok(view)
}

async fn check_round(
    connection: &mut SqliteConnection,
    scope: Scope,
    round: RoundId,
) -> Result<()> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agena_rounds WHERE id=? AND execution_id=? AND generation=? AND phase='running')")
        .bind(round.get()).bind(scope.execution.to_string()).bind(scope.generation.get())
        .fetch_one(connection).await.map_err(sql_error)?;
    if !exists {
        return Err(KernelError::Conflict(
            "round is not running in this attempt".into(),
        ));
    }
    Ok(())
}

pub(super) async fn check_tool(
    connection: &mut SqliteConnection,
    scope: Scope,
    tool: ToolId,
) -> Result<RoundId> {
    let id: Option<i64> = sqlx::query_scalar("SELECT round_id FROM agena_tool_calls WHERE id=? AND execution_id=? AND generation=? AND phase='running'")
        .bind(tool.get()).bind(scope.execution.to_string()).bind(scope.generation.get())
        .fetch_optional(connection).await.map_err(sql_error)?;
    RoundId::try_from(
        id.ok_or_else(|| KernelError::Conflict("tool is not running in this attempt".into()))?,
    )
    .map_err(storage)
}

pub(super) async fn finish(
    connection: &mut SqliteConnection,
    mut view: ExecutionView,
    phase: ExecutionPhase,
) -> Result<ExecutionView> {
    let mut state = ExecutionState::from_stored(view.generation, view.phase);
    if state.finish(view.generation, phase)? == TransitionEffect::Unchanged {
        return Ok(view);
    }
    let active: i64 = sqlx::query_scalar("SELECT count(*) FROM agena_tool_calls WHERE execution_id=? AND generation=? AND phase IN ('preparing','running')")
        .bind(view.id.to_string()).bind(view.generation.get()).fetch_one(&mut *connection).await.map_err(sql_error)?;
    let rounds: i64 = sqlx::query_scalar("SELECT count(*) FROM agena_rounds WHERE execution_id=? AND generation=? AND phase='running'")
        .bind(view.id.to_string()).bind(view.generation.get()).fetch_one(&mut *connection).await.map_err(sql_error)?;
    if phase == ExecutionPhase::Succeeded && (active > 0 || rounds > 0) {
        return Err(KernelError::Conflict(
            "cannot succeed while tools or provider rounds are running".into(),
        ));
    }
    let now = now_ms();
    sqlx::query("UPDATE agena_entries SET revision=revision+1 WHERE id IN (SELECT response_entry_id FROM agena_attempts WHERE execution_id=? AND generation=?) OR id=?")
        .bind(view.id.to_string()).bind(view.generation.get()).bind(view.message_id.get()).execute(&mut *connection).await.map_err(sql_error)?;
    if phase != ExecutionPhase::Succeeded {
        settle_children(connection, &view, now).await?;
    }
    sqlx::query("UPDATE agena_attempts SET phase=?,finished_at_ms=MAX(?,COALESCE(started_at_ms,?)) WHERE execution_id=? AND generation=?")
        .bind(enum_text(&phase)?).bind(now).bind(now).bind(view.id.to_string()).bind(view.generation.get())
        .execute(&mut *connection).await.map_err(sql_error)?;
    sqlx::query("UPDATE agena_session_statistics SET queued_executions=queued_executions-?,running_executions=running_executions-?,last_message_at_ms=MAX(?,COALESCE(last_message_at_ms,?)) WHERE session_id=?")
        .bind(i64::from(view.phase == ExecutionPhase::Queued))
        .bind(i64::from(view.phase == ExecutionPhase::Running))
        .bind(now).bind(now).bind(view.session_id.get())
        .execute(&mut *connection).await.map_err(sql_error)?;
    sqlx::query("UPDATE agena_sessions SET updated_at_ms=MAX(?,updated_at_ms+1) WHERE id=?")
        .bind(now)
        .bind(view.session_id.get())
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    view.phase = phase;
    Ok(view)
}

async fn settle_children(
    connection: &mut SqliteConnection,
    view: &ExecutionView,
    now: i64,
) -> Result<()> {
    // Guidance is scoped to this attempt's lifetime. Cancellation, failure or
    // interruption must not leave instructions for a later retry/followup.
    sqlx::query("UPDATE agena_deliveries SET state='failed',owner_epoch=NULL,lease_until_ms=NULL WHERE target_execution_id=? AND state='pending'")
        .bind(view.id.to_string()).execute(&mut *connection).await.map_err(sql_error)?;
    sqlx::query("UPDATE agena_interactions SET decision_json=?,decided_at_ms=? WHERE decision_json IS NULL AND tool_id IN (SELECT id FROM agena_tool_calls WHERE execution_id=? AND generation=?)")
        .bind(encode(&InteractionDecision::Cancel)?).bind(now).bind(view.id.to_string()).bind(view.generation.get())
        .execute(&mut *connection).await.map_err(sql_error)?;
    sqlx::query("UPDATE agena_entries SET revision=revision+1 WHERE tool_id IN (SELECT id FROM agena_tool_calls WHERE execution_id=? AND generation=? AND phase IN ('preparing','running'))")
        .bind(view.id.to_string()).bind(view.generation.get()).execute(&mut *connection).await.map_err(sql_error)?;
    sqlx::query("UPDATE agena_tool_calls SET phase='interrupted',result_json='null',finished_at_ms=? WHERE execution_id=? AND generation=? AND phase IN ('preparing','running')")
        .bind(now).bind(view.id.to_string()).bind(view.generation.get()).execute(&mut *connection).await.map_err(sql_error)?;
    sqlx::query("UPDATE agena_rounds SET phase='interrupted',finished_at_ms=? WHERE execution_id=? AND generation=? AND phase='running'")
        .bind(now).bind(view.id.to_string()).bind(view.generation.get()).execute(&mut *connection).await.map_err(sql_error)?;
    Ok(())
}

async fn bump_tool_entries(connection: &mut SqliteConnection, tool: i64) -> Result<()> {
    sqlx::query("UPDATE agena_entries SET revision=revision+1 WHERE tool_id=?")
        .bind(tool)
        .execute(connection)
        .await
        .map_err(sql_error)?;
    Ok(())
}

async fn reply(
    connection: &mut SqliteConnection,
    session: SessionId,
    interaction: InteractionId,
    decision: &InteractionDecision,
    remember: Option<&RememberPermission>,
) -> Result<Mutation> {
    let metadata = reads::metadata(connection, session).await?;
    let row = sqlx::query("SELECT i.request_json,i.decision_json,i.remember_json,i.created_at_ms,t.execution_id,t.generation,t.phase FROM agena_interactions i JOIN agena_tool_calls t ON t.id=i.tool_id JOIN agena_executions e ON e.id=t.execution_id WHERE i.id=? AND e.session_id=? AND e.current_generation=t.generation")
        .bind(interaction.get()).bind(session.get()).fetch_optional(&mut *connection).await.map_err(sql_error)?.ok_or(KernelError::Ownership)?;
    let previous: Option<String> = row.try_get("decision_json").map_err(storage)?;
    if let Some(previous) = previous {
        let previous_rule = row
            .try_get::<Option<String>, _>("remember_json")
            .map_err(storage)?
            .as_deref()
            .map(decode::<RememberPermission>)
            .transpose()?;
        if decode::<InteractionDecision>(&previous)? != *decision
            || previous_rule.as_ref() != remember
        {
            return Err(KernelError::Conflict(
                "interaction was already decided differently".into(),
            ));
        }
        return Ok(Mutation::Decision(decision.clone()));
    }
    let execution: ExecutionId = row
        .try_get::<String, _>("execution_id")
        .map_err(storage)?
        .parse()
        .map_err(storage)?;
    let (view, _) = reads::execution(connection, session, execution).await?;
    if view.phase != ExecutionPhase::Running
        || row.try_get::<String, _>("phase").map_err(storage)? != "running"
    {
        return Err(KernelError::Conflict(
            "interaction is no longer active".into(),
        ));
    }
    let request: InteractionRequest =
        decode(&row.try_get::<String, _>("request_json").map_err(storage)?)?;
    match (&request, decision) {
        (
            InteractionRequest::Permission { .. },
            InteractionDecision::Permission { reply, allowed },
        ) => {
            use agena_domain::{PermissionReplyKind, PermissionScope};
            if reply.request_id != interaction.get().to_string()
                || reply
                    .reason
                    .as_ref()
                    .is_some_and(|reason| reason.len() > 4096)
            {
                return Err(invalid("permission reply identity or reason is invalid"));
            }
            let expected = match reply.kind {
                PermissionReplyKind::AllowOnce | PermissionReplyKind::AllowAlways if !*allowed => {
                    return Err(invalid("allow reply has a denied outcome"));
                }
                PermissionReplyKind::DenyOnce | PermissionReplyKind::DenyAlways if *allowed => {
                    return Err(invalid("deny reply has an allowed outcome"));
                }
                PermissionReplyKind::AllowAlways => Some(RememberPermission {
                    mode: PermissionMode::Allow,
                    scope: reply.scope.unwrap_or(PermissionScope::Session),
                }),
                PermissionReplyKind::DenyAlways => Some(RememberPermission {
                    mode: PermissionMode::Deny,
                    scope: reply.scope.unwrap_or(PermissionScope::Session),
                }),
                _ => None,
            };
            if remember != expected.as_ref() {
                return Err(invalid(
                    "remembered rule must match the complete permission reply",
                ));
            }
        }
        (InteractionRequest::Permission { .. }, InteractionDecision::Cancel)
            if remember.is_none() => {}
        (
            InteractionRequest::Input { .. },
            InteractionDecision::Input(_) | InteractionDecision::Cancel,
        ) if remember.is_none() => {}
        _ => return Err(invalid("decision is incompatible with this interaction")),
    }
    if let Some(remember) = remember {
        let InteractionRequest::Permission { action_key, .. } = &request else {
            return Err(invalid("only permission decisions can save a rule"));
        };
        let expected = match decision {
            InteractionDecision::Permission { allowed: true, .. } => PermissionMode::Allow,
            InteractionDecision::Permission { allowed: false, .. } => PermissionMode::Deny,
            _ => return Err(invalid("cancel cannot save a permission rule")),
        };
        if remember.mode != expected {
            return Err(invalid("remembered rule must agree with the decision"));
        }
        remember_rule(connection, &metadata, action_key, remember).await?;
    }
    sqlx::query(
        "UPDATE agena_interactions SET decision_json=?,remember_json=?,decided_at_ms=? WHERE id=?",
    )
    .bind(encode(decision)?)
    .bind(remember.map(encode).transpose()?)
    .bind(now_ms().max(row.try_get::<i64, _>("created_at_ms").map_err(storage)?))
    .bind(interaction.get())
    .execute(&mut *connection)
    .await
    .map_err(sql_error)?;
    sqlx::query("UPDATE agena_entries SET revision=revision+1 WHERE interaction_id=?")
        .bind(interaction.get())
        .execute(connection)
        .await
        .map_err(sql_error)?;
    Ok(Mutation::Decision(decision.clone()))
}

async fn remember_rule(
    connection: &mut SqliteConnection,
    metadata: &SessionMetadata,
    action: &str,
    remember: &RememberPermission,
) -> Result<()> {
    let (session, workspace) = match remember.scope {
        PermissionScope::Session => (Some(metadata.id.get()), None),
        PermissionScope::Workspace => (None, Some(metadata.workspace_id)),
        PermissionScope::Global => (None, None),
    };
    let scope = enum_text(&remember.scope)?;
    let mode = enum_text(&remember.mode)?;
    let now = now_ms();
    let existing: Option<i64> = sqlx::query_scalar("SELECT id FROM agena_permission_rules WHERE action_key=? AND scope=? AND session_id IS ? AND workspace_id IS ?")
        .bind(action).bind(&scope).bind(session).bind(workspace).fetch_optional(&mut *connection).await.map_err(sql_error)?;
    if let Some(id) = existing {
        sqlx::query("UPDATE agena_permission_rules SET mode=?,source='interaction',revoked_at_ms=NULL,revoked_reason=NULL,revoked_by=NULL,updated_at_ms=? WHERE id=?")
            .bind(mode).bind(now).bind(id).execute(connection).await.map_err(sql_error)?;
    } else {
        sqlx::query("INSERT INTO agena_permission_rules (action_key,mode,scope,session_id,workspace_id,source,created_at_ms,updated_at_ms) VALUES (?,?,?,?,?,'interaction',?,?)")
            .bind(action).bind(mode).bind(scope).bind(session).bind(workspace).bind(now).bind(now).execute(connection).await.map_err(sql_error)?;
    }
    Ok(())
}

/// Only the bounded tail is copied. Published segments are never edited;
/// branches can keep referencing an old segment without an ancestry walk.
pub(super) async fn append_history(
    connection: &mut SqliteConnection,
    session: SessionId,
    mut entries: &[i64],
) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let ids = encode(entries)?;
    sqlx::query("UPDATE agena_attempts SET response_entry_id=(SELECT MIN(e.id) FROM agena_entries e WHERE e.execution_id=agena_attempts.execution_id AND e.generation=agena_attempts.generation AND e.kind IN ('text','reasoning','tool') AND e.id IN (SELECT value FROM json_each(?))) WHERE response_entry_id IS NULL AND execution_id IN (SELECT execution_id FROM agena_entries WHERE id IN (SELECT value FROM json_each(?)))")
        .bind(&ids).bind(&ids).execute(&mut *connection).await.map_err(sql_error)?;
    let appended = i64::try_from(entries.len()).map_err(storage)?;
    let metadata = reads::metadata(connection, session).await?;
    let next = metadata.membership_generation.next().map_err(storage)?;
    let tail=sqlx::query("SELECT sequence,segment_id,first_ordinal,end_ordinal FROM agena_session_segments WHERE session_id=? ORDER BY sequence DESC LIMIT 1")
        .bind(session.get()).fetch_optional(&mut *connection).await.map_err(sql_error)?;
    let mut sequence = 0i64;
    let mut copy = None;
    let mut obsolete = None;
    if let Some(tail) = tail {
        sequence = tail.try_get("sequence").map_err(storage)?;
        let first: i64 = tail.try_get("first_ordinal").map_err(storage)?;
        let end: i64 = tail.try_get("end_ordinal").map_err(storage)?;
        if end - first < 128 {
            let segment: i64 = tail.try_get("segment_id").map_err(storage)?;
            copy = Some((segment, first, end));
            obsolete = Some(segment);
        } else {
            sequence = sequence
                .checked_add(1)
                .ok_or_else(|| storage("segment sequence exhausted"))?;
        }
    }
    while !entries.is_empty() {
        let segment: i64 =
            sqlx::query_scalar("INSERT INTO agena_history_segments DEFAULT VALUES RETURNING id")
                .fetch_one(&mut *connection)
                .await
                .map_err(sql_error)?;
        let count = if let Some((old, first, end)) = copy.take() {
            // The bounded boundary copy stays inside SQLite. It does not
            // materialize old members or issue one statement per membership.
            sqlx::query("INSERT INTO agena_segment_entries SELECT ?,ordinal-?,entry_id FROM agena_segment_entries WHERE segment_id=? AND ordinal>=? AND ordinal<? ORDER BY ordinal")
                .bind(segment).bind(first).bind(old).bind(first).bind(end)
                .execute(&mut *connection).await.map_err(sql_error)?;
            sqlx::query("INSERT INTO agena_retired_session_segments (session_id,sequence,segment_id,first_ordinal,end_ordinal,since_generation,until_generation) SELECT session_id,sequence,segment_id,first_ordinal,end_ordinal,since_generation,? FROM agena_session_segments edge WHERE session_id=? AND sequence=? AND EXISTS(SELECT 1 FROM agena_history_snapshots root WHERE root.session_id=edge.session_id AND root.generation>=edge.since_generation AND root.generation<?)")
                .bind(next.get()).bind(session.get()).bind(sequence).bind(next.get())
                .execute(&mut *connection).await.map_err(sql_error)?;
            sqlx::query("DELETE FROM agena_session_segments WHERE session_id=? AND sequence=?")
                .bind(session.get())
                .bind(sequence)
                .execute(&mut *connection)
                .await
                .map_err(sql_error)?;
            (end - first) as usize
        } else {
            0
        };
        let take = entries.len().min(128 - count);
        let mut insert = sqlx::QueryBuilder::new(
            "INSERT INTO agena_segment_entries (segment_id,ordinal,entry_id) ",
        );
        insert.push_values(
            entries[..take].iter().enumerate(),
            |mut row, (ordinal, entry)| {
                row.push_bind(segment)
                    .push_bind((count + ordinal) as i64)
                    .push_bind(*entry);
            },
        );
        insert
            .build()
            .execute(&mut *connection)
            .await
            .map_err(sql_error)?;
        sqlx::query("INSERT INTO agena_session_segments (session_id,sequence,segment_id,first_ordinal,end_ordinal,since_generation) VALUES (?,?,?,0,?,?)")
            .bind(session.get())
            .bind(sequence)
            .bind(segment)
            .bind((count + take) as i64)
            .bind(next.get())
            .execute(&mut *connection)
            .await
            .map_err(sql_error)?;
        entries = &entries[take..];
        sequence = sequence
            .checked_add(1)
            .ok_or_else(|| storage("segment sequence exhausted"))?;
    }
    if let Some(old) = obsolete {
        // Shared branches retain old segments. An unreferenced tail is
        // reclaimed atomically so ordinary appends do not retain old copies.
        sqlx::query("DELETE FROM agena_history_segments WHERE id=? AND NOT EXISTS (SELECT 1 FROM agena_session_segments WHERE segment_id=?) AND NOT EXISTS (SELECT 1 FROM agena_retired_session_segments WHERE segment_id=?)")
            .bind(old).bind(old).bind(old).execute(&mut *connection).await.map_err(sql_error)?;
    }
    sqlx::query("UPDATE agena_sessions SET membership_generation=? WHERE id=?")
        .bind(next.get())
        .bind(session.get())
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    sqlx::query("UPDATE agena_session_statistics SET entry_count=entry_count+? WHERE session_id=?")
        .bind(appended)
        .bind(session.get())
        .execute(connection)
        .await
        .map_err(sql_error)?;
    Ok(())
}

pub(super) async fn recover(database: &sea_orm::DatabaseConnection) -> Result<()> {
    let transaction = database.begin().await.map_err(storage)?;
    // Recovery is indexed by active phase, never a transcript/full-session
    // load. No tool or provider side effect is repeated during startup.
    let now = now_ms();
    for sql in [
        "DELETE FROM agena_history_snapshots",
        "UPDATE agena_sessions SET control_version=control_version+1 WHERE id IN (SELECT session_id FROM agena_attempts WHERE phase='running' UNION SELECT session_id FROM agena_background_activities WHERE phase='running' UNION SELECT session_id FROM agena_content_streams WHERE capture_state='open' UNION SELECT id FROM agena_sessions WHERE deleted_at_ms IS NULL AND json_extract(configuration_json,'$.agent.status') IN ('created','running'))",
        "UPDATE agena_sessions SET configuration_json=json_set(configuration_json,'$.agent.status','interrupted','$.agent.finished_at_ms',?),metadata_version=metadata_version+1,updated_at_ms=MAX(?,updated_at_ms+1) WHERE deleted_at_ms IS NULL AND json_extract(configuration_json,'$.agent.status') IN ('created','running')",
        // Agent launches require a supervised handoff and their retained
        // budgets. Even an unclaimed launch waits for explicit followup;
        // generic queued-input recovery must never start it unsupervised.
        "UPDATE agena_entries SET revision=revision+1 WHERE id IN (SELECT x.message_id FROM agena_executions x JOIN agena_attempts a ON a.execution_id=x.id AND a.generation=x.current_generation WHERE a.phase='queued' AND a.session_id IN (SELECT id FROM agena_sessions WHERE deleted_at_ms IS NULL AND agent_status='interrupted'))",
        "UPDATE agena_session_statistics SET queued_executions=0,last_message_at_ms=MAX(?,COALESCE(last_message_at_ms,?)) WHERE queued_executions>0 AND session_id IN (SELECT id FROM agena_sessions WHERE deleted_at_ms IS NULL AND agent_status='interrupted')",
        "UPDATE agena_attempts SET phase='interrupted',finished_at_ms=? WHERE phase='queued' AND session_id IN (SELECT id FROM agena_sessions WHERE deleted_at_ms IS NULL AND agent_status='interrupted')",
        "UPDATE agena_entries SET revision=revision+1 WHERE id IN (SELECT x.message_id FROM agena_executions x JOIN agena_attempts a ON a.execution_id=x.id AND a.generation=x.current_generation WHERE a.phase='running') OR id IN (SELECT response_entry_id FROM agena_attempts WHERE phase='running')",
        "UPDATE agena_entries SET revision=revision+1 WHERE id IN (SELECT entry_id FROM agena_background_activities WHERE phase='running')",
        "UPDATE agena_background_activities SET phase='interrupted',outcome_json='{\"outcome\":\"interrupted\",\"diagnostic\":\"service restarted; external activity outcome is unknown\"}',finished_at_ms=MAX(?,created_at_ms) WHERE phase='running'",
        "UPDATE agena_session_statistics SET running_executions=0,last_message_at_ms=MAX(?,COALESCE(last_message_at_ms,?)) WHERE session_id IN (SELECT session_id FROM agena_attempts WHERE phase='running')",
        "UPDATE agena_sessions SET updated_at_ms=MAX(?,updated_at_ms+1) WHERE id IN (SELECT session_id FROM agena_attempts WHERE phase='running')",
        "UPDATE agena_interactions SET decision_json='{\"kind\":\"cancel\"}',decided_at_ms=? WHERE decision_json IS NULL AND tool_id IN (SELECT id FROM agena_tool_calls WHERE phase IN ('preparing','running'))",
        "UPDATE agena_entries SET revision=revision+1 WHERE tool_id IN (SELECT id FROM agena_tool_calls WHERE phase IN ('preparing','running'))",
        "UPDATE agena_tool_calls SET phase='interrupted',result_json='null',finished_at_ms=? WHERE phase IN ('preparing','running')",
        "UPDATE agena_rounds SET phase='interrupted',finished_at_ms=? WHERE phase='running'",
        "UPDATE agena_deliveries SET state='failed',owner_epoch=NULL,lease_until_ms=NULL WHERE state='pending' AND target_execution_id IS NOT NULL AND EXISTS(SELECT 1 FROM agena_executions x JOIN agena_attempts a ON a.execution_id=x.id AND a.generation=x.current_generation WHERE x.id=agena_deliveries.target_execution_id AND a.phase!='queued')",
        "UPDATE agena_attempts SET phase='interrupted',finished_at_ms=MAX(?,COALESCE(started_at_ms,?)) WHERE phase='running'",
        "UPDATE agena_session_statistics SET processing_state='ready' WHERE queued_executions>0 AND running_executions=0 AND EXISTS(SELECT 1 FROM agena_sessions s WHERE s.id=agena_session_statistics.session_id AND COALESCE(json_extract(s.configuration_json,'$.message_queue_paused'),0)=1)",
        "DELETE FROM agena_blob_pins",
    ] {
        let values = (0..sql.matches('?').count()).map(|_| sea_orm::Value::from(now));
        transaction
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                sql,
                values,
            ))
            .await
            .map_err(storage)?;
    }
    transaction.execute(Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "UPDATE agena_content_streams SET capture_state='interrupted',epoch=? WHERE capture_state='open'",
        [agena_domain::ContentEpoch::new().to_string().into()],
    )).await.map_err(storage)?;
    transaction.commit().await.map_err(storage)
}

async fn begin_round(
    connection: &mut SqliteConnection,
    scope: Scope,
    inputs: &[EntryId],
) -> Result<Mutation> {
    let now = now_ms();
    check_scope(connection, scope, true).await?;
    if inputs.len() > 10000 {
        return Err(invalid("provider round input identity budget exceeded"));
    }
    let active: i64 = sqlx::query_scalar("SELECT count(*) FROM agena_rounds WHERE execution_id=? AND generation=? AND phase='running'")
        .bind(scope.execution.to_string()).bind(scope.generation.get()).fetch_one(&mut *connection).await.map_err(sql_error)?;
    if active > 0 {
        return Err(KernelError::Conflict(
            "provider round already running".into(),
        ));
    }
    let identities = encode(inputs)?;
    let members: i64 = sqlx::query_scalar("SELECT count(*) FROM agena_entries e WHERE e.id IN (SELECT value FROM json_each(?)) AND EXISTS(SELECT 1 FROM agena_segment_entries m JOIN agena_session_segments s ON s.segment_id=m.segment_id WHERE s.session_id=? AND m.entry_id=e.id AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal)")
        .bind(&identities).bind(scope.session.get()).fetch_one(&mut *connection).await.map_err(sql_error)?;
    if members as usize != inputs.len() {
        return Err(KernelError::Ownership);
    }
    let metadata = reads::metadata(connection, scope.session).await?;
    let id: i64 = sqlx::query_scalar("INSERT INTO agena_rounds (execution_id,generation,session_id,ordinal,phase,created_at_ms,input_metadata_json) SELECT ?,?,?,COALESCE(MAX(ordinal)+1,0),'running',?,? FROM agena_rounds WHERE execution_id=? AND generation=? RETURNING id")
        .bind(scope.execution.to_string()).bind(scope.generation.get()).bind(scope.session.get()).bind(now).bind(encode(&metadata)?)
        .bind(scope.execution.to_string()).bind(scope.generation.get()).fetch_one(&mut *connection).await.map_err(sql_error)?;
    sqlx::query("INSERT INTO agena_round_inputs (round_id,ordinal,entry_id,entry_revision,response_entry_id,execution_phase,tool_phase,interaction_decided) SELECT ?,CAST(j.key AS INTEGER),e.id,e.revision,(SELECT a.response_entry_id FROM agena_attempts a WHERE a.execution_id=e.execution_id AND a.generation=e.generation),(SELECT a.phase FROM agena_attempts a WHERE a.execution_id=e.execution_id AND a.generation=CASE WHEN e.kind='user' THEN (SELECT x.current_generation FROM agena_executions x WHERE x.id=e.execution_id) ELSE e.generation END),CASE WHEN e.kind='tool' THEN t.phase END,CASE WHEN i.decision_json IS NOT NULL THEN 1 ELSE 0 END FROM json_each(?) j JOIN agena_entries e ON e.id=j.value LEFT JOIN agena_tool_calls t ON t.id=e.tool_id LEFT JOIN agena_interactions i ON i.id=e.interaction_id")
        .bind(id).bind(identities).execute(&mut *connection).await.map_err(sql_error)?;
    sqlx::query("INSERT INTO agena_round_input_replays (round_id,source_round_id,phase,has_provider_state) SELECT DISTINCT ?,r.id,r.phase,r.provider_state_json IS NOT NULL FROM agena_round_inputs input JOIN agena_entries e ON e.id=input.entry_id JOIN agena_rounds r ON r.id=e.round_id WHERE input.round_id=?")
        .bind(id).bind(id).execute(&mut *connection).await.map_err(sql_error)?;
    let input_bytes = super::prompt::check_budget(connection, id, None).await?;
    let source_size = sqlx::query(SOURCE_SIZE_SQL)
        .bind(id)
        .fetch_one(&mut *connection)
        .await
        .map_err(sql_error)?;
    let sources: i64 = source_size.try_get("sources").map_err(storage)?;
    let source_bytes = usize::try_from(source_size.try_get::<i64, _>("bytes").map_err(storage)?)
        .map_err(storage)?;
    if sources > 10000 || input_bytes.saturating_add(source_bytes) > super::prompt::MAX_INPUT_BYTES
    {
        return Err(invalid("provider input source/byte budget exceeded"));
    }
    // Only the small boundary descriptors are copied. Payload blocks,
    // tool results and opaque replay remain single immutable facts.
    sqlx::query(CAPTURE_SOURCES_SQL)
        .bind(id)
        .bind(id)
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    let sources: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agena_round_input_sources WHERE round_id=?")
            .bind(id)
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
    if sources > 10000 {
        return Err(invalid("provider input source budget exceeded"));
    }
    super::prompt::check_budget(connection, id, None).await?;
    sqlx::query("UPDATE agena_attempts SET stop_requested=0 WHERE execution_id=? AND generation=?")
        .bind(scope.execution.to_string())
        .bind(scope.generation.get())
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    Ok(Mutation::Round(RoundId::try_from(id).map_err(storage)?))
}
