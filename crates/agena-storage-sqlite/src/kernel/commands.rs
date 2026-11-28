use agena_domain::{PermissionMode, PermissionScope};
use agena_storage::backend::kernel::Scope;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, TransactionTrait};

use super::*;

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
        | Command::CreateGlobal { .. }
        | Command::CreateSeededAgent { .. }
        | Command::EnsureLocalBotSession { .. }
        | Command::DetachGlobal { .. }
        | Command::Configure { .. }
        | Command::RegisterWorkingDirectory { .. }
        | Command::RemoveWorkingDirectory { .. }
        | Command::RecoverChannelAuthority { .. }
        | Command::FinishAgent { .. }
        | Command::Fork { .. }
        | Command::Withdraw { .. }
        | Command::Truncate { .. }
        | Command::Clear { .. }
        | Command::DeleteSession { .. }
        | Command::Continue { .. }
        | Command::StartControlExecution { .. }
        | Command::Notice { .. }
        | Command::Deliver { .. }
        | Command::DeliverToExecution { .. }
        | Command::DeliverAndResume { .. }
        | Command::ResumeDelivery { .. }
        | Command::ConsumeDelivery { .. } => unreachable!("lifecycle command handled above"),
        Command::CreateSession {
            workspace_id,
            title,
        } => {
            if *workspace_id <= 0 || title.len() > 4096 {
                return Err(invalid("invalid workspace or title"));
            }
            create_session(connection, Some(*workspace_id), title, now).await
        }
        Command::Metadata { session, change } => {
            let mut metadata = reads::metadata(connection, *session).await?;
            if change
                .expected_version
                .is_some_and(|version| version != metadata.metadata_version.get())
            {
                return Err(KernelError::Conflict(
                    "session metadata changed; inspect before renaming".into(),
                ));
            }
            let original = metadata.clone();
            let mut config = lifecycle::configuration(connection, *session).await?;
            let original_config = config.clone();
            if let Some(update) = &change.automatic_title {
                if change.title.is_some()
                    || change.favorite.is_some()
                    || change.pinned.is_some()
                    || change.auto_title.is_some()
                    || change.prompts.is_some()
                    || change.profile.is_some()
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
                let last = reads::latest_user_entry(connection, *session).await?;
                if last
                    != Some((
                        update.last_user_message.get(),
                        update.last_user_revision.get(),
                    ))
                {
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
            if let Some(value) = &change.prompts {
                config.prompts.clone_from(value);
            }
            if let Some(value) = &change.profile {
                let selected = (!value.is_empty()).then(|| value.clone());
                lifecycle::validate_profile_change(
                    connection,
                    *session,
                    &config.profile,
                    &selected,
                )
                .await?;
                config.profile = selected;
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
            patch_selection(connection, *session, None, std::slice::from_ref(change)).await
        }
        Command::SelectionPatch {
            session,
            expected_version,
            changes,
        } => patch_selection(connection, *session, Some(*expected_version), changes).await,
        Command::QueueControl {
            session,
            expected_control_version,
            change,
        } => control_queue(connection, *session, *expected_control_version, change).await,
        Command::Send {
            session,
            document,
            blobs,
            author,
            origin,
            model_context,
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
            let body = encode_user_body(document, author.as_ref(), origin.as_ref(), model_context)?;
            let execution = ExecutionId::new();
            let entry: i64 = sqlx::query_scalar("INSERT INTO agena_entries (origin_session_id,execution_id,generation,kind,body_json,created_at_ms) VALUES (?,?,1,'user',?,?) RETURNING id")
                .bind(session.get()).bind(execution.to_string()).bind(body).bind(now)
                .fetch_one(&mut *connection).await.map_err(sql_error)?;
            sqlx::query("INSERT INTO agena_executions (id,session_id,message_id,current_generation,created_at_ms) VALUES (?,?,?,1,?)")
                .bind(execution.to_string()).bind(session.get()).bind(entry).bind(now).execute(&mut *connection).await.map_err(sql_error)?;
            sqlx::query("INSERT INTO agena_attempts (execution_id,generation,session_id,phase) VALUES (?,1,?,'queued')")
                .bind(execution.to_string()).bind(session.get()).execute(&mut *connection).await.map_err(sql_error)?;
            append_history(connection, *session, entry).await?;
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
            model_context,
        } => {
            let (view, _) = reads::execution(connection, *session, *execution).await?;
            let (revision, started): (i64, bool) = sqlx::query_as("SELECT revision,EXISTS(SELECT 1 FROM agena_attempts a WHERE a.execution_id=? AND a.started_at_ms IS NOT NULL) FROM agena_entries WHERE id=?")
                .bind(execution.to_string()).bind(view.message_id.get()).fetch_one(&mut *connection).await.map_err(sql_error)?;
            if view.purpose != ExecutionPurpose::Message {
                return Err(invalid(
                    "a control execution is not an editable user message",
                ));
            }
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
            let frozen: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agena_prompt_input_entries WHERE entry_id=?) OR EXISTS(SELECT 1 FROM agena_segment_entries m JOIN agena_session_segments s ON s.segment_id=m.segment_id WHERE m.entry_id=? AND s.session_id!=? AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal)")
                .bind(view.message_id.get()).bind(view.message_id.get()).bind(session.get()).fetch_one(&mut *connection).await.map_err(sql_error)?;
            if frozen {
                return Err(KernelError::Conflict(
                    "the input is already shared with another conversation or model round".into(),
                ));
            }
            // Editing preserves the message's recorded identity; only the
            // document changes.
            let existing =
                sqlx::query_scalar::<_, String>("SELECT body_json FROM agena_entries WHERE id=?")
                    .bind(view.message_id.get())
                    .fetch_one(&mut *connection)
                    .await
                    .map_err(sql_error)?;
            let existing = decode_user_body(&existing)?;
            if document.is_empty() {
                return Err(invalid("message is empty"));
            }
            let model_context = if *document == existing.document && !existing.model_context.is_empty() {
                existing.model_context.as_slice()
            } else {
                model_context.as_slice()
            };
            let body = encode_user_body(
                document,
                existing.author.as_ref(),
                existing.origin.as_ref(),
                model_context,
            )?;
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
            ..
        }
        | Command::ClaimNext {
            session,
            execution,
            ..
        }
        | Command::CancelNext {
            session,
            execution,
            ..
        } => {
            let (view, _) = reads::execution(connection, *session, *execution).await?;
            let expected_generation = match command {
                Command::ClaimNext { expected_generation, .. } => *expected_generation,
                Command::CancelNext { expected_generation, .. } => Some(*expected_generation),
                _ => None,
            };
            if expected_generation.is_some_and(|expected| expected != view.generation) {
                return Err(KernelError::StaleAttempt);
            }
            // This check and the claim share the serialized writer transaction.
            // A stop must persist pause before releasing the current execution.
            if super::lifecycle::configuration(connection, *session)
                .await?
                .message_queue_paused
            {
                return Err(KernelError::QueuePaused);
            }
            if !matches!(command, Command::Claim { .. }) {
                let blocked: bool = sqlx::query_scalar(
                    "SELECT EXISTS(
                        SELECT 1 FROM agena_attempts a
                        CROSS JOIN agena_executions x ON x.id=a.execution_id AND x.current_generation=a.generation
                        WHERE a.session_id=? AND a.phase IN ('queued','running')
                          AND (a.phase='running' OR x.message_id<?)
                    )",
                )
                    .bind(session.get()).bind(view.message_id.get()).fetch_one(&mut *connection).await.map_err(sql_error)?;
                if blocked {
                    return Err(KernelError::QueueOutOfOrder);
                }
            }
            let epoch = match command {
                Command::Claim { epoch, .. } | Command::ClaimNext { epoch, .. } => epoch,
                Command::CancelNext { notice, .. } => {
                    if view.phase != ExecutionPhase::Queued {
                        return Err(KernelError::StaleAttempt);
                    }
                    let view = finish(connection, view, ExecutionPhase::Cancelled).await?;
                    super::lifecycle::notice(connection, *session, view.message_id, notice).await?;
                    return Ok(Mutation::Execution(view));
                }
                _ => unreachable!(),
            };
            let mut state = ExecutionState::from_stored(view.generation, view.phase);
            state.claim()?;
            sqlx::query("UPDATE agena_attempts SET phase='running',owner_epoch=?,started_at_ms=? WHERE execution_id=? AND generation=?")
                .bind(epoch.to_string()).bind(now).bind(execution.to_string()).bind(view.generation.get())
                .execute(&mut *connection).await.map_err(sql_error)?;
            let first_start =
                history_runs::start_message(connection, *session, view.message_id).await?;
            sqlx::query(
                "UPDATE agena_session_statistics SET queued_executions=queued_executions-1,
                    running_executions=running_executions+1,
                    started_user_message_count=started_user_message_count+?
                 WHERE session_id=?",
            )
            .bind(first_start)
            .bind(session.get())
            .execute(&mut *connection)
            .await
            .map_err(sql_error)?;
            Ok(Mutation::Execution(
                reads::execution(connection, *session, *execution).await?.0,
            ))
        }
        Command::Cancel {
            session,
            execution,
            expected_generation,
            epoch,
        } => {
            let (view, _) = reads::execution(connection, *session, *execution).await?;
            if expected_generation.is_some_and(|expected| expected != view.generation) {
                return Err(KernelError::StaleAttempt);
            }
            let unclaimed = view.phase == ExecutionPhase::Queued;
            let finished = finish(connection, view, ExecutionPhase::Cancelled).await?;
            if unclaimed && let Some(epoch) = epoch {
                // Terminal rows permit ownership without a provider start.
                // This fence owns cancellation callbacks, not a fictitious
                // provider claim, and is committed atomically with cancellation.
                sqlx::query("UPDATE agena_attempts SET owner_epoch=? WHERE execution_id=? AND generation=? AND owner_epoch IS NULL")
                    .bind(epoch.to_string()).bind(execution.to_string())
                    .bind(finished.generation.get()).execute(connection).await.map_err(sql_error)?;
            }
            Ok(Mutation::Execution(finished))
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
            let ids: Vec<String> = sqlx::query_scalar(
                "SELECT x.id FROM agena_attempts a
                 CROSS JOIN agena_executions x ON x.id=a.execution_id AND x.current_generation=a.generation
                 WHERE a.session_id=? AND a.phase IN ('queued','running') AND a.phase='queued'
                 ORDER BY x.message_id",
            )
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
            if view.purpose == ExecutionPurpose::PluginCommand {
                return Err(invalid("invoke the plugin command again to start another job"));
            }
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
            append_history(connection, scope.session, entry).await?;
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
            append_history(connection, scope.session, entry).await?;
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
            activity_reads,
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
            if result.phase == ToolPhase::Succeeded {
                if activity_reads.len() > 16 {
                    return Err(invalid("a tool may acknowledge at most 16 activity outputs"));
                }
                for read in activity_reads {
                    super::activity::acknowledge_output(connection, scope.session, *read).await?;
                    if read.position.outcome_read {
                        super::activity::retire_pending_notice(connection, scope.session, read.activity).await?;
                    }
                }
            }
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
            output_retention,
        } => super::activity::begin(connection, *scope, *tool, activity, *output_retention).await,
        Command::BeginPluginCommandActivity { scope, origin, activity, retention } => {
            super::activity::begin_command(connection, *scope, origin, activity, *retention).await
        }
        Command::EndActivity { scope, outcome, notice, output_terminal } => {
            let result = super::activity::end(connection, *scope, outcome, output_terminal.as_ref()).await?;
            if let Some(notice) = notice {
                let key = super::activity::notice_key(scope.id);
                let mut payload = serde_json::json!({
                    "kind": "integration_notice",
                    "notice_id": key,
                    "origin_message_id": notice.origin_message_id.get(),
                    "text": notice.text,
                    "wake": notice.wake,
                });
                if let Some(budget) = &notice.wake_budget {
                    payload["wake_budget"] = serde_json::to_value(budget).map_err(storage)?;
                }
                if let Some(position) = notice.output_position {
                    payload["activity_output"] = serde_json::to_value(ActivityOutputRead {
                        activity: scope.id,
                        position,
                    }).map_err(storage)?;
                }
                super::lifecycle::publish_delivery(connection, scope.session, None, &key, &payload)
                    .await?;
            }
            Ok(result)
        }
        Command::AppendActivityOutput { scope, total, chunks } => {
            super::activity_output::append(connection, *scope, *total, chunks).await
        }
        Command::SetActivityOutputSpillPaths { scope, paths } => {
            super::activity_output::set_spill_paths(connection, *scope, paths).await
        }
        Command::AcknowledgeActivityOutput { session, read, expected } => {
            super::activity::acknowledge_direct_output(connection, *session, *read, *expected).await
        }
        Command::UnlistSettledActivity { session, activity } => {
            super::activity::unlist_settled(connection, *session, *activity).await
        }
        Command::OpenActivityOutput { scope, kind } => {
            super::activity::open_output(connection, *scope, *kind).await
        }
        Command::OpenText {
            scope,
            round,
            tool,
            reasoning,
            display,
        } => {
            let execution = check_scope(connection, *scope, true).await?;
            if *display && (*reasoning || tool.is_some()) {
                return Err(invalid("display streams require an assistant text entry"));
            }
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
            if active + i64::from(*display) >= content::MAX_LIVE_SOURCES {
                return Err(KernelError::Busy);
            }
            let id = agena_domain::ContentId::new();
            let display = display.then(agena_domain::ContentId::new);
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
                append_history(connection, scope.session, entry).await?;
                entry
            };
            let mut heads = Vec::with_capacity(1 + usize::from(display.is_some()));
            // A display stream is the second text source owned by this
            // assistant entry. Keep the primary entry pointer on raw text.
            for id in std::iter::once(id).chain(display) {
                let epoch = agena_domain::ContentEpoch::new();
                sqlx::query("INSERT INTO agena_content_streams (id,entry_id,session_id,execution_id,generation,kind,epoch,capture_state) VALUES (?,?,?,?,?,'text',?,'open')")
                    .bind(id.to_string()).bind(entry).bind(scope.session.get()).bind(scope.execution.to_string()).bind(scope.generation.get()).bind(epoch.to_string())
                    .execute(&mut *connection).await.map_err(sql_error)?;
                content::assign_ordinal(connection, id).await?;
                heads.push(content::stored_head(connection, id).await?);
            }
            let mut heads = heads.into_iter();
            let original = heads.next().expect("raw text source");
            Ok(match heads.next() {
                Some(display) => Mutation::TextWithDisplay { original, display },
                None => Mutation::Content(original),
            })
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

pub(super) async fn create_session(
    connection: &mut SqliteConnection,
    workspace_id: Option<i64>,
    title: &str,
    now: i64,
) -> Result<Mutation> {
    if workspace_id.is_some_and(|id| id <= 0) || title.len() > 4096 {
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
    // A plugin submission that lost its turn must still reach a new turn.
    // Execution guidance below remains tied to the stopped attempt.
    sqlx::query("UPDATE agena_deliveries SET target_execution_id=NULL WHERE target_execution_id=? AND state='pending' AND json_extract(payload_json,'$.kind')='plugin_prompt'")
        .bind(view.id.to_string()).execute(&mut *connection).await.map_err(sql_error)?;
    // Guidance is scoped to this attempt's lifetime. Cancellation, failure or
    // interruption must not leave instructions for a later retry/followup.
    sqlx::query("UPDATE agena_deliveries SET state='failed' WHERE target_execution_id=? AND state='pending'")
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
    agena_domain::validate_persisted_network_rule_key(action, remember.mode, std::env::consts::OS)
        .map_err(invalid)?;
    let (session, workspace) = match remember.scope {
        PermissionScope::Session => (Some(metadata.id.get()), None),
        PermissionScope::Workspace => (
            None,
            Some(metadata.workspace_id.ok_or_else(|| {
                invalid("global sessions do not have a workspace permission scope")
            })?),
        ),
        PermissionScope::Global => (None, None),
    };
    let scope = enum_text(&remember.scope)?;
    let mode = enum_text(&remember.mode)?;
    let now = now_ms();
    let predicate = match remember.scope {
        PermissionScope::Global => "session_id IS NULL AND workspace_id IS NULL",
        PermissionScope::Workspace => "session_id IS NULL AND workspace_id=?",
        PermissionScope::Session => "workspace_id IS NULL AND session_id=?",
    };
    let sql = format!(
        "SELECT id FROM agena_permission_rules WHERE action_key=? AND scope=? AND {predicate}"
    );
    let mut query = sqlx::query_scalar(&sql).bind(action).bind(&scope);
    if let Some(subject) = session.or(workspace) {
        query = query.bind(subject);
    }
    let existing: Option<i64> = query
        .fetch_optional(&mut *connection)
        .await
        .map_err(sql_error)?;
    if let Some(id) = existing {
        sqlx::query("UPDATE agena_permission_rules SET mode=?,source='interaction',revoked_at_ms=NULL,revoked_reason=NULL,revoked_by=NULL,updated_at_ms=? WHERE id=?")
            .bind(mode).bind(now).bind(id).execute(connection).await.map_err(sql_error)?;
    } else {
        sqlx::query("INSERT INTO agena_permission_rules (action_key,mode,scope,session_id,workspace_id,source,created_at_ms,updated_at_ms) VALUES (?,?,?,?,?,'interaction',?,?)")
            .bind(action).bind(mode).bind(scope).bind(session).bind(workspace).bind(now).bind(now).execute(connection).await.map_err(sql_error)?;
    }
    Ok(())
}

/// Allocate the next entry ID inside the writer transaction so a notice
/// can use its own ID as its foreign-key anchor in one INSERT.
pub(super) async fn next_entry_id(connection: &mut SqliteConnection) -> Result<i64> {
    let previous: i64 = sqlx::query_scalar(
        "SELECT COALESCE((SELECT seq FROM sqlite_sequence WHERE name='agena_entries'),0)",
    )
    .fetch_one(connection)
    .await
    .map_err(sql_error)?;
    previous.checked_add(1).ok_or_else(|| invalid("entry identity exhausted"))
}

/// Detach the tail inside the write transaction. Reuse it when there are no
/// remaining edges, and copy the visible range when a branch or snapshot owns it.
pub(super) async fn append_history(
    connection: &mut SqliteConnection,
    session: SessionId,
    entry: i64,
) -> Result<()> {
    let generation: i64 = sqlx::query_scalar(
        "SELECT membership_generation FROM agena_sessions WHERE id=? AND deleted_at_ms IS NULL",
    )
    .bind(session.get())
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?
    .ok_or_else(|| KernelError::NotFound("session".into()))?;
    let next = MembershipGeneration::try_from(generation)
        .and_then(MembershipGeneration::next)
        .map_err(storage)?;
    let tail: Option<(i64, i64, i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT sequence,segment_id,first_ordinal,end_ordinal,user_message_count,max_entry_created_at_ms
         FROM agena_session_segments WHERE session_id=? ORDER BY sequence DESC LIMIT 1",
    )
    .bind(session.get())
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?;
    let (sequence, segment, ordinal, users, latest) = match tail {
        Some((sequence, old, first, end, users, latest)) if end - first < 128 => {
            sqlx::query("INSERT INTO agena_retired_session_segments (session_id,sequence,segment_id,first_ordinal,end_ordinal,since_generation,until_generation) SELECT session_id,sequence,segment_id,first_ordinal,end_ordinal,since_generation,? FROM agena_session_segments edge WHERE session_id=? AND sequence=? AND EXISTS(SELECT 1 FROM agena_history_snapshots root WHERE root.session_id=edge.session_id AND root.generation>=edge.since_generation AND root.generation<?)")
                .bind(next.get()).bind(session.get()).bind(sequence).bind(next.get())
                .execute(&mut *connection).await.map_err(sql_error)?;
            sqlx::query("DELETE FROM agena_session_segments WHERE session_id=? AND sequence=?")
                .bind(session.get())
                .bind(sequence)
                .execute(&mut *connection)
                .await
                .map_err(sql_error)?;
            let referenced: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM agena_session_segments WHERE segment_id=?1)
                     OR EXISTS(SELECT 1 FROM agena_retired_session_segments WHERE segment_id=?1)",
            )
            .bind(old)
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
            let segment = if !referenced && first == 0 {
                // A rewind can leave an invisible suffix on the reused segment.
                sqlx::query("DELETE FROM agena_segment_entries WHERE segment_id=? AND ordinal>=?")
                    .bind(old)
                    .bind(end)
                    .execute(&mut *connection)
                    .await
                    .map_err(sql_error)?;
                old
            } else {
                let segment = new_history_segment(connection).await?;
                sqlx::query("INSERT INTO agena_segment_entries SELECT ?,ordinal-?,entry_id FROM agena_segment_entries WHERE segment_id=? AND ordinal>=? AND ordinal<? ORDER BY ordinal")
                    .bind(segment).bind(first).bind(old).bind(first).bind(end)
                    .execute(&mut *connection).await.map_err(sql_error)?;
                if !referenced {
                    sqlx::query("DELETE FROM agena_history_segments WHERE id=?")
                        .bind(old)
                        .execute(&mut *connection)
                        .await
                        .map_err(sql_error)?;
                }
                segment
            };
            if first != 0 {
                // A shared slice can start at a nonzero ordinal. Copying the
                // tail rebases its positions, including the prompt checkpoint.
                sqlx::query("UPDATE agena_prompt_windows SET through_ordinal=through_ordinal-? WHERE session_id=? AND through_sequence=?")
                    .bind(first)
                    .bind(session.get())
                    .bind(sequence)
                    .execute(&mut *connection)
                    .await
                    .map_err(sql_error)?;
            }
            (sequence, segment, end - first, users, Some(latest))
        }
        Some((sequence, ..)) => (
            sequence
                .checked_add(1)
                .ok_or_else(|| storage("segment sequence exhausted"))?,
            new_history_segment(connection).await?,
            0,
            0,
            None,
        ),
        None => (0, new_history_segment(connection).await?, 0, 0, None),
    };
    sqlx::query("INSERT INTO agena_segment_entries (segment_id,ordinal,entry_id) VALUES (?,?,?)")
        .bind(segment)
        .bind(ordinal)
        .bind(entry)
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    sqlx::query(
        "INSERT INTO agena_session_segments (
             session_id,sequence,segment_id,first_ordinal,end_ordinal,since_generation,
             user_message_count,max_entry_created_at_ms
         )
         SELECT ?1,?2,?3,0,?4,?5,?6+(e.kind='user'),MAX(COALESCE(?7,e.created_at_ms),e.created_at_ms)
         FROM agena_entries e WHERE e.id=?8",
    )
    .bind(session.get())
    .bind(sequence)
    .bind(segment)
    .bind(ordinal + 1)
    .bind(next.get())
    .bind(users)
    .bind(latest)
    .bind(entry)
    .execute(&mut *connection)
    .await
    .map_err(sql_error)?;
    history_runs::append(connection, session, entry, sequence).await?;
    sqlx::query("UPDATE agena_sessions SET membership_generation=? WHERE id=?")
        .bind(next.get())
        .bind(session.get())
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    sqlx::query("UPDATE agena_session_statistics SET entry_count=entry_count+1 WHERE session_id=?")
        .bind(session.get())
        .execute(connection)
        .await
        .map_err(sql_error)?;
    Ok(())
}

async fn new_history_segment(connection: &mut SqliteConnection) -> Result<i64> {
    sqlx::query_scalar("INSERT INTO agena_history_segments DEFAULT VALUES RETURNING id")
        .fetch_one(connection)
        .await
        .map_err(sql_error)
}

pub(super) async fn recover(database: &sea_orm::DatabaseConnection) -> Result<()> {
    let transaction = database.begin().await.map_err(storage)?;
    // Recovery is indexed by active phase, never a transcript/full-session
    // load. No tool or provider side effect is repeated during startup.
    let now = now_ms();
    for sql in [
        "DELETE FROM agena_history_snapshots",
        "UPDATE agena_sessions SET control_version=control_version+1 WHERE id IN (SELECT session_id FROM agena_attempts WHERE phase='running' UNION SELECT session_id FROM agena_background_activities WHERE phase='running' UNION SELECT session_id FROM agena_content_streams WHERE capture_state='open' UNION SELECT id FROM agena_sessions WHERE deleted_at_ms IS NULL AND agent_status IN ('created','running'))",
        "UPDATE agena_sessions SET configuration_json=json_set(configuration_json,'$.agent.status','interrupted','$.agent.finished_at_ms',?),metadata_version=metadata_version+1,updated_at_ms=MAX(?,updated_at_ms+1) WHERE deleted_at_ms IS NULL AND agent_status IN ('created','running')",
        // Agent launches require a supervised handoff and their retained
        // budgets. Even an unclaimed launch waits for explicit followup;
        // generic queued-input recovery must never start it unsupervised.
        // Queued compaction is independent maintenance: preserve its purpose
        // and queue count without restarting or settling the agent's task.
        "UPDATE agena_entries SET revision=revision+1 WHERE id IN (SELECT x.message_id FROM agena_executions x JOIN agena_attempts a ON a.execution_id=x.id AND a.generation=x.current_generation WHERE a.phase='queued' AND a.session_id IN (SELECT id FROM agena_sessions WHERE deleted_at_ms IS NULL AND agent_status='interrupted') AND NOT EXISTS(SELECT 1 FROM agena_execution_purposes purpose WHERE purpose.execution_id=x.id AND purpose.purpose='compaction'))",
        "UPDATE agena_session_statistics SET queued_executions=(SELECT count(*) FROM agena_attempts a JOIN agena_executions x ON x.id=a.execution_id AND x.current_generation=a.generation JOIN agena_execution_purposes purpose ON purpose.execution_id=x.id WHERE a.session_id=agena_session_statistics.session_id AND a.phase='queued' AND purpose.purpose='compaction'),last_message_at_ms=MAX(?,COALESCE(last_message_at_ms,?)) WHERE queued_executions>0 AND session_id IN (SELECT id FROM agena_sessions WHERE deleted_at_ms IS NULL AND agent_status='interrupted')",
        "UPDATE agena_attempts SET phase='interrupted',finished_at_ms=? WHERE phase='queued' AND session_id IN (SELECT id FROM agena_sessions WHERE deleted_at_ms IS NULL AND agent_status='interrupted') AND NOT EXISTS(SELECT 1 FROM agena_execution_purposes purpose WHERE purpose.execution_id=agena_attempts.execution_id AND purpose.purpose='compaction')",
        "UPDATE agena_entries SET revision=revision+1 WHERE id IN (SELECT x.message_id FROM agena_executions x JOIN agena_attempts a ON a.execution_id=x.id AND a.generation=x.current_generation WHERE a.phase='running') OR id IN (SELECT response_entry_id FROM agena_attempts WHERE phase='running')",
        "UPDATE agena_entries SET revision=revision+1 WHERE id IN (SELECT entry_id FROM agena_background_activities WHERE phase='running')",
        "UPDATE agena_background_activities SET phase='interrupted',outcome_json='{\"outcome\":\"interrupted\",\"diagnostic\":\"service restarted; external activity outcome is unknown\"}',finished_at_ms=MAX(?,created_at_ms) WHERE phase='running'",
        "UPDATE agena_session_statistics SET running_executions=0,last_message_at_ms=MAX(?,COALESCE(last_message_at_ms,?)) WHERE session_id IN (SELECT session_id FROM agena_attempts WHERE phase='running')",
        "UPDATE agena_sessions SET updated_at_ms=MAX(?,updated_at_ms+1) WHERE id IN (SELECT session_id FROM agena_attempts WHERE phase='running')",
        "UPDATE agena_interactions SET decision_json='{\"kind\":\"cancel\"}',decided_at_ms=? WHERE decision_json IS NULL AND tool_id IN (SELECT id FROM agena_tool_calls WHERE phase IN ('preparing','running'))",
        "UPDATE agena_entries SET revision=revision+1 WHERE tool_id IN (SELECT id FROM agena_tool_calls WHERE phase IN ('preparing','running'))",
        "UPDATE agena_tool_calls SET phase='interrupted',result_json='null',finished_at_ms=? WHERE phase IN ('preparing','running')",
        "UPDATE agena_rounds SET phase='interrupted',finished_at_ms=? WHERE phase='running'",
        "UPDATE agena_deliveries SET target_execution_id=NULL WHERE state='pending' AND target_execution_id IS NOT NULL AND json_extract(payload_json,'$.kind')='plugin_prompt'",
        "UPDATE agena_deliveries SET state='failed' WHERE state='pending' AND target_execution_id IS NOT NULL AND EXISTS(SELECT 1 FROM agena_executions x JOIN agena_attempts a ON a.execution_id=x.id AND a.generation=x.current_generation WHERE x.id=agena_deliveries.target_execution_id AND a.phase!='queued')",
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
    if inputs.len() > prompt::MAX_INPUTS {
        return Err(invalid("provider round input identity budget exceeded"));
    }
    let active: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agena_rounds WHERE execution_id=? AND generation=? AND phase='running')")
        .bind(scope.execution.to_string()).bind(scope.generation.get()).fetch_one(&mut *connection).await.map_err(sql_error)?;
    if active {
        return Err(KernelError::Conflict(
            "provider round already running".into(),
        ));
    }
    if inputs
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len()
        != inputs.len()
    {
        return Err(KernelError::Ownership);
    }
    let metadata = reads::metadata(connection, scope.session).await?;
    let id: i64 = sqlx::query_scalar("INSERT INTO agena_rounds (execution_id,generation,session_id,ordinal,phase,created_at_ms,input_metadata_json) SELECT ?,?,?,COALESCE(MAX(ordinal)+1,0),'running',?,? FROM agena_rounds WHERE execution_id=? AND generation=? RETURNING id")
        .bind(scope.execution.to_string()).bind(scope.generation.get()).bind(scope.session.get()).bind(now).bind(encode(&metadata)?)
        .bind(scope.execution.to_string()).bind(scope.generation.get()).fetch_one(&mut *connection).await.map_err(sql_error)?;
    prompt_inputs::capture(connection, id, scope.session, inputs).await?;
    prompt_replays::capture(connection, id, scope.session).await?;
    let input_bytes = prompt::check_input_budget(connection, id, None)
        .await?
        .bytes;
    prompt_sources::capture(connection, id, input_bytes).await?;
    sqlx::query("UPDATE agena_attempts SET stop_requested=0 WHERE execution_id=? AND generation=?")
        .bind(scope.execution.to_string())
        .bind(scope.generation.get())
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    Ok(Mutation::Round(RoundId::try_from(id).map_err(storage)?))
}

/// Merge under the writer transaction; no detached session value can overwrite
/// another client's fields, and a multi-field patch has one version increment.
async fn patch_selection(
    connection: &mut SqliteConnection,
    session: SessionId,
    expected: Option<i64>,
    changes: &[SelectionChange],
) -> Result<Mutation> {
    let mut metadata = reads::metadata(connection, session).await?;
    if expected.is_some_and(|version| version != metadata.metadata_version.get()) {
        return Err(KernelError::Conflict(
            "session selection changed; inspect before configuring".into(),
        ));
    }
    let mut selection = metadata.selection.clone().unwrap_or_default();
    for change in changes {
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
    }
    selection.normalize();
    let selection = (!selection.is_empty()).then_some(selection);
    if selection != metadata.selection {
        metadata.metadata_version = metadata.metadata_version.next().map_err(storage)?;
        metadata.selection = selection;
        sqlx::query("UPDATE agena_sessions SET selection_json=?,metadata_version=?,updated_at_ms=MAX(?,updated_at_ms+1) WHERE id=?")
            .bind(metadata.selection.as_ref().map(encode).transpose()?)
            .bind(metadata.metadata_version.get()).bind(now_ms()).bind(session.get())
            .execute(connection).await.map_err(sql_error)?;
    }
    Ok(Mutation::Session(Box::new(metadata)))
}

async fn control_queue(
    connection: &mut SqliteConnection,
    session: SessionId,
    expected: i64,
    change: &QueueChange,
) -> Result<Mutation> {
    let version: i64 = sqlx::query_scalar(
        "SELECT control_version FROM agena_sessions WHERE id=? AND deleted_at_ms IS NULL",
    )
    .bind(session.get())
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?
    .ok_or_else(|| KernelError::NotFound("session".into()))?;
    if expected != version {
        return Err(KernelError::Conflict(
            "message queue changed; inspect before modifying".into(),
        ));
    }
    match change {
        QueueChange::Pause(paused) => {
            // Delegate to the ordinary configuration writer, in this transaction.
            let command = Command::Configure {
                session,
                change: SessionConfigurationChange {
                    message_queue_paused: Some(*paused),
                    ..Default::default()
                },
            };
            lifecycle::apply(connection, &command)
                .await?
                .ok_or_else(|| invalid("queue pause command was not handled"))?;
        }
        QueueChange::EditText {
            execution,
            generation,
            text,
        } => {
            let (view, _) = reads::execution(connection, session, *execution).await?;
            check_queued_attempt(&view, *generation)?;
            let row = sqlx::query("SELECT body_json,EXISTS(SELECT 1 FROM agena_attempts WHERE execution_id=? AND started_at_ms IS NOT NULL) AS started FROM agena_entries WHERE id=?")
                .bind(execution.to_string()).bind(view.message_id.get()).fetch_one(&mut *connection).await.map_err(sql_error)?;
            if row.try_get::<bool, _>("started").map_err(storage)? {
                return Err(KernelError::Conflict(
                    "the queued input has already run; text edit refused".into(),
                ));
            }
            let frozen: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agena_prompt_input_entries WHERE entry_id=?) OR EXISTS(SELECT 1 FROM agena_segment_entries m JOIN agena_session_segments s ON s.segment_id=m.segment_id WHERE m.entry_id=? AND s.session_id!=? AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal)")
                .bind(view.message_id.get()).bind(view.message_id.get()).bind(session.get())
                .fetch_one(&mut *connection).await.map_err(sql_error)?;
            if frozen {
                return Err(KernelError::Conflict(
                    "input is already shared with another conversation or model round".into(),
                ));
            }
            let mut document: agena_domain::ComposerDocument =
                decode_user_body(&row.try_get::<String, _>("body_json").map_err(storage)?)?
                    .document;
            if text.trim().is_empty() || text.len() > 16 * 1024 {
                return Err(invalid("queued text must be nonempty and within 16 KiB"));
            }
            if document.text() == *text {
                return Ok(Mutation::Session(Box::new(
                    reads::metadata(connection, session).await?,
                )));
            }
            // Keep activities byte-for-byte and never delete/re-publish blob references.
            // Place replacement text at the first original text position.
            let mut replaced = false;
            document.0.retain_mut(|node| match node {
                agena_domain::ComposerNode::Text { text: original } if !replaced => {
                    original.clone_from(text);
                    replaced = true;
                    true
                }
                agena_domain::ComposerNode::Text { .. } => false,
                _ => true,
            });
            if !replaced {
                document
                    .0
                    .insert(0, agena_domain::ComposerNode::Text { text: text.clone() });
            }
            // Editing preserves the message's recorded identity; only the
            // document changes.
            let existing =
                decode_user_body(&row.try_get::<String, _>("body_json").map_err(storage)?)?;
            let body = encode_user_body(
                &document,
                existing.author.as_ref(),
                existing.origin.as_ref(),
                &[],
            )?;
            sqlx::query("UPDATE agena_entries SET body_json=?,revision=revision+1 WHERE id=?")
                .bind(body)
                .bind(view.message_id.get())
                .execute(&mut *connection)
                .await
                .map_err(sql_error)?;
            sqlx::query(
                "UPDATE agena_sessions SET updated_at_ms=MAX(?,updated_at_ms+1) WHERE id=?",
            )
            .bind(now_ms())
            .bind(session.get())
            .execute(&mut *connection)
            .await
            .map_err(sql_error)?;
        }
        QueueChange::Remove {
            execution,
            generation,
        } => {
            let (view, _) = reads::execution(connection, session, *execution).await?;
            check_queued_attempt(&view, *generation)?;
            finish(connection, view, ExecutionPhase::Cancelled).await?;
        }
    }
    Ok(Mutation::Session(Box::new(
        reads::metadata(connection, session).await?,
    )))
}

fn check_queued_attempt(view: &ExecutionView, generation: AttemptGeneration) -> Result<()> {
    if view.generation != generation {
        return Err(KernelError::StaleAttempt);
    }
    if view.phase != ExecutionPhase::Queued {
        return Err(KernelError::Conflict(
            "the queued message has already started or been removed".into(),
        ));
    }
    if view.purpose != ExecutionPurpose::Message {
        return Err(invalid(
            "only native queued user messages support edit/remove",
        ));
    }
    Ok(())
}
