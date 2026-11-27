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
        | Command::ImportExternalAgent { .. }
        | Command::DetachExternalAgent { .. }
        | Command::EnsureLocalBotSession { .. }
        | Command::DetachGlobal { .. }
        | Command::Configure { .. }
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
                    || change.sync_external_agent_title
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
                if change.sync_external_agent_title {
                    if config.external_agent.is_none() {
                        return Err(KernelError::Ownership);
                    }
                    config.external_agent_state.metadata.title = Some(title.clone());
                }
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
                super::external_agent::bound_configuration_state(&mut config)?;
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
            let body = encode_user_body(document, author.as_ref(), origin.as_ref())?;
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
            let body =
                encode_user_body(document, existing.author.as_ref(), existing.origin.as_ref())?;
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
            ..
        } => {
            let (view, _) = reads::execution(connection, *session, *execution).await?;
            if let Command::ClaimNext {
                expected_generation: Some(expected),
                ..
            } = command
                && *expected != view.generation
            {
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
            if matches!(command, Command::ClaimNext { .. }) {
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
        Command::BeginCurrentRound {
            scope,
            take_external_agent_resume,
        } => {
            let execution = check_scope(connection, *scope, true).await?;
            let window = super::input_window::select(
                connection,
                scope.session,
                execution.message_id,
                10000,
                false,
            )
            .await?;
            let round = begin_round(connection, *scope, &window.entries).await?;
            if !take_external_agent_resume {
                return Ok(round);
            }
            let Mutation::Round(round) = round else {
                unreachable!("begin-round result")
            };
            let config = lifecycle::configuration(connection, scope.session).await?;
            if config.external_agent.is_none() {
                return Err(invalid(
                    "external agent round requires an external agent session",
                ));
            }
            let link = config.native_session_link();
            let mut pending = config.external_agent_pending_turn;
            if let Some(prior) = pending.as_mut().filter(|prior| {
                prior.message == execution.message_id
                    && matches!(prior.delivery, ExternalAgentPromptDelivery::Keyed { .. })
            }) {
                // Keys deduplicate inserted prompts. A tool that began before
                // acknowledgment can have effects even without an insertion.
                let tools_started: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM agena_tool_calls WHERE round_id=?)",
                )
                .bind(prior.round.get())
                .fetch_one(&mut *connection)
                .await
                .map_err(sql_error)?;
                if tools_started {
                    prior.delivery = ExternalAgentPromptDelivery::Unconfirmed;
                }
            }
            Ok(Mutation::ExternalAgentRound {
                round,
                previous: config
                    .external_agent_resume
                    .filter(|resume| resume.session == scope.session),
                link,
                pending,
                selection: config.external_agent_selection,
                state: Box::new(config.external_agent_state),
            })
        }
        Command::RecordExternalAgentState {
            scope,
            round,
            state,
        } => {
            check_scope(connection, *scope, true).await?;
            check_round(connection, *scope, *round).await?;
            let snapshot = external_agent_state_snapshot(connection, scope.session, state).await?;
            let updated = sqlx::query(
                "UPDATE agena_sessions SET configuration_json=CASE
                    WHEN ? THEN json_remove(configuration_json,'$.external_agent_state')
                    ELSE json_set(configuration_json,'$.external_agent_state',json(?))
                 END WHERE id=? AND json_extract(configuration_json,'$.external_agent') IS NOT NULL
                    AND json_extract(configuration_json,'$.external_agent_resume') IS NULL",
            )
            .bind(snapshot.is_empty())
            .bind(encode(&snapshot)?)
            .bind(scope.session.get())
            .execute(&mut *connection)
            .await
            .map_err(sql_error)?;
            if updated.rows_affected() == 0 {
                return Err(KernelError::Ownership);
            }
            sync_external_agent_title(connection, scope.session, state).await?;
            Ok(Mutation::Round(*round))
        }
        Command::BindExternalAgentLink {
            scope,
            round,
            link,
            pending,
        } => bind_external_agent_link(connection, *scope, *round, link, pending).await,
        Command::RecordExternalAgentPromptDelivery {
            scope,
            round,
            delivery,
        } => record_external_agent_prompt_delivery(connection, *scope, *round, *delivery).await,
        Command::RefreshIdleExternalAgentState {
            session,
            round,
            native_id,
            state,
        } => {
            // An idle connection must never modify a conversation that was
            // reopened, cleared, detached, or deleted after the turn finished.
            let snapshot = external_agent_state_snapshot(connection, *session, state).await?;
            let updated = sqlx::query(
                "UPDATE agena_sessions SET configuration_json=CASE
                    WHEN ? THEN json_remove(configuration_json,'$.external_agent_state')
                    ELSE json_set(configuration_json,'$.external_agent_state',json(?))
                 END WHERE id=? AND json_extract(configuration_json,'$.external_agent_resume.native_id')=?
                    AND json_extract(configuration_json,'$.external_agent_resume.round')=?
                    AND deleted_at_ms IS NULL
                    AND NOT EXISTS (
                        SELECT 1 FROM agena_session_statistics
                        WHERE session_id=agena_sessions.id AND running_executions>0
                    )",
            )
            .bind(snapshot.is_empty())
            .bind(encode(&snapshot)?)
            .bind(session.get())
            .bind(native_id)
            .bind(round.get())
            .execute(&mut *connection)
            .await
            .map_err(sql_error)?;
            if updated.rows_affected() == 0 {
                return Err(KernelError::Ownership);
            }
            sync_external_agent_title(connection, *session, state).await?;
            Ok(Mutation::Round(*round))
        }
        Command::UpdateExternalAgentCursor {
            session,
            round,
            active,
            native_id,
            change,
        } => {
            let configuration = lifecycle::configuration(connection, *session).await?;
            let link = configuration
                .native_session_link()
                .filter(|link| &link.native_id == native_id)
                .ok_or(KernelError::Ownership)?;
            if let Some(scope) = active {
                if scope.session != *session {
                    return Err(KernelError::Ownership);
                }
                check_scope(connection, *scope, true).await?;
                check_round(connection, *scope, *round).await?;
            } else {
                let resume = configuration.external_agent_resume.as_ref();
                if !resume
                    .is_some_and(|resume| resume.round == *round && &resume.native_id == native_id)
                {
                    return Err(KernelError::Ownership);
                }
                super::external_agent::require_idle(connection, *session).await?;
            }
            super::external_agent::update_cursor(connection, *session, &link, change).await?;
            Ok(Mutation::Round(*round))
        }
        Command::SetExternalAgentInput {
            session,
            epoch,
            input,
            scope,
        } => set_external_agent_input(connection, *session, *epoch, *input, scope.as_ref()).await,
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
        Command::Finish {
            scope,
            phase,
            external_agent_resume,
        } => {
            let view = check_scope(connection, *scope, false).await?;
            if *phase == ExecutionPhase::Succeeded && !view.phase.is_terminal() {
                let pending: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agena_deliveries INDEXED BY idx_agena_delivery_execution WHERE target_execution_id=? AND state='pending')")
                    .bind(view.id.to_string()).fetch_one(&mut *connection).await.map_err(sql_error)?;
                if pending {
                    return Err(KernelError::PendingDelivery(view.id));
                }
            }
            if let Some(resume) = external_agent_resume
                && view.phase == ExecutionPhase::Running
            {
                save_external_agent_resume(connection, *scope, &view, *phase, resume).await?;
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
            content::require_live_source_slot(connection).await?;
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
                append_history(connection, scope.session, entry).await?;
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
        Command::WriteAssistantDocument {
            target,
            update,
            blobs,
        } => content::events::write_assistant_document(connection, target, update, blobs).await,
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

pub(super) async fn check_round(
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

async fn sync_external_agent_title(
    connection: &mut SqliteConnection,
    session: SessionId,
    state: &agena_storage::kernel::ExternalAgentState,
) -> Result<()> {
    if let Some(title) = state.metadata.title.as_ref()
        && !title.trim().is_empty()
        && title.len() <= 4096
        && !title.chars().any(char::is_control)
    {
        sqlx::query("UPDATE agena_sessions
            SET title=?,metadata_version=metadata_version+1,
                configuration_json=json_set(configuration_json,'$.title_state.source','automatic')
            WHERE id=? AND title!=?
                AND json_extract(configuration_json,'$.title_state.source') IN ('default','automatic')")
            .bind(title).bind(session.get()).bind(title)
            .execute(connection).await.map_err(sql_error)?;
    }
    Ok(())
}

async fn bind_external_agent_link(
    connection: &mut SqliteConnection,
    scope: Scope,
    round: RoundId,
    link: &ExternalAgentLink,
    pending: &ExternalAgentPendingTurn,
) -> Result<Mutation> {
    let view = check_scope(connection, scope, true).await?;
    check_round(connection, scope, round).await?;
    super::external_agent::validate_link(link)?;
    let metadata = reads::metadata(connection, scope.session).await?;
    let workspace = metadata.workspace_id.ok_or(KernelError::Ownership)?;
    let mut configuration = lifecycle::configuration(connection, scope.session).await?;
    let agent = configuration
        .external_agent
        .clone()
        .ok_or(KernelError::Ownership)?;
    if link.resume_at.is_some()
        || link.incarnation.is_none()
        || pending.round != round
        || pending.message != view.message_id
    {
        return Err(KernelError::Ownership);
    }
    let current = configuration.native_session_link();
    let same_link = current.as_ref() == Some(link);
    let prior_delivery = configuration
        .external_agent_pending_turn
        .as_ref()
        .filter(|prior| {
            same_link && prior.message == pending.message && prior.input == pending.input
        })
        .map_or(ExternalAgentPromptDelivery::Unsent, |prior| prior.delivery);
    if pending.delivery != prior_delivery {
        return Err(KernelError::Ownership);
    }
    let resumes_import = current.as_ref().is_some_and(|current| {
        current.resume_at.is_some()
            && current.cwd == link.cwd
            && current.native_id == link.native_id
            && current.incarnation == link.incarnation
    });
    if !same_link
        && !resumes_import
        && (link.incarnation != Some(round)
            || current.is_some_and(|current| current.incarnation == Some(round)))
    {
        return Err(KernelError::Ownership);
    }
    if super::external_agent::existing_link(connection, workspace, agent, link)
        .await?
        .is_some_and(|owner| owner != scope.session)
    {
        return Err(KernelError::Conflict(
            "this native conversation is already linked".into(),
        ));
    }
    let state = std::mem::take(&mut configuration.external_agent_state);
    configuration.external_agent_resume = None;
    configuration.external_agent_link = Some(link.clone());
    configuration.external_agent_pending_turn = Some(pending.clone());
    let base_bytes = encode(&configuration)?.len();
    configuration.external_agent_state =
        state.bounded_snapshot(super::external_agent::state_budget(base_bytes));
    sqlx::query("UPDATE agena_sessions SET configuration_json=? WHERE id=?")
        .bind(encode(&configuration)?)
        .bind(scope.session.get())
        .execute(connection)
        .await
        .map_err(sql_error)?;
    Ok(Mutation::Round(round))
}

async fn record_external_agent_prompt_delivery(
    connection: &mut SqliteConnection,
    scope: Scope,
    round: RoundId,
    delivery: ExternalAgentPromptDelivery,
) -> Result<Mutation> {
    use ExternalAgentPromptDelivery::{Accepted, Keyed, Unconfirmed, Unsent};

    let view = check_scope(connection, scope, true).await?;
    check_round(connection, scope, round).await?;
    let configuration = lifecycle::configuration(connection, scope.session).await?;
    let pending = configuration
        .external_agent_pending_turn
        .filter(|pending| pending.round == round && pending.message == view.message_id)
        .ok_or(KernelError::Ownership)?;
    if pending.delivery == delivery {
        return Ok(Mutation::Round(round));
    }
    if !matches!(
        (pending.delivery, delivery),
        (Unsent, Unconfirmed | Keyed { .. }) | (Unconfirmed | Keyed { .. }, Accepted | Unsent)
    ) {
        return Err(KernelError::Ownership);
    }
    if delivery == Unsent {
        let tools_started: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agena_tool_calls WHERE round_id=?)")
                .bind(round.get())
                .fetch_one(&mut *connection)
                .await
                .map_err(sql_error)?;
        if tools_started {
            return Ok(Mutation::Round(round));
        }
    }
    sqlx::query(
        "UPDATE agena_sessions SET configuration_json=json_set(
            configuration_json,'$.external_agent_pending_turn.delivery',json(?)) WHERE id=?",
    )
    .bind(encode(&delivery)?)
    .bind(scope.session.get())
    .execute(connection)
    .await
    .map_err(sql_error)?;
    Ok(Mutation::Round(round))
}

async fn set_external_agent_input(
    connection: &mut SqliteConnection,
    session: SessionId,
    epoch: agena_domain::KernelEpoch,
    input: RequestId,
    scope: Option<&ExternalAgentInputScope>,
) -> Result<Mutation> {
    let metadata = reads::metadata(connection, session).await?;
    let mut configuration = lifecycle::configuration(connection, session).await?;
    let mut presence = configuration
        .external_agent_inputs
        .take()
        .filter(|presence| presence.epoch == epoch)
        .unwrap_or_else(|| ExternalAgentInputPresence {
            epoch,
            ids: Vec::new(),
        });
    if let Some(scope) = scope {
        if configuration.external_agent.as_ref() != Some(&scope.agent) {
            return Err(KernelError::Ownership);
        }
        if let Some(resume) = &scope.idle_resume {
            if configuration.external_agent_resume.as_ref() != Some(resume) {
                return Err(KernelError::Ownership);
            }
            let running: bool = sqlx::query_scalar(
                "SELECT running_executions>0 FROM agena_session_statistics WHERE session_id=?",
            )
            .bind(session.get())
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
            if running {
                return Err(KernelError::Ownership);
            }
        }
        if !presence.ids.contains(&input) {
            if presence.ids.len() >= MAX_EXTERNAL_AGENT_INPUTS {
                return Err(KernelError::Conflict(
                    "too many external agent input requests".into(),
                ));
            }
            presence.ids.push(input);
        }
    } else {
        presence.ids.retain(|id| *id != input);
    }
    configuration.external_agent_inputs = (!presence.ids.is_empty()).then_some(presence);
    super::external_agent::bound_configuration_state(&mut configuration)?;
    sqlx::query("UPDATE agena_sessions SET configuration_json=? WHERE id=?")
        .bind(encode(&configuration)?)
        .bind(session.get())
        .execute(connection)
        .await
        .map_err(sql_error)?;
    Ok(Mutation::Session(Box::new(metadata)))
}

async fn external_agent_state_snapshot(
    connection: &mut SqliteConnection,
    session: SessionId,
    state: &agena_storage::kernel::ExternalAgentState,
) -> Result<agena_storage::kernel::ExternalAgentState> {
    let config_bytes: i64 = sqlx::query_scalar(
        "SELECT length(CAST(json_remove(configuration_json,'$.external_agent_state') AS BLOB))
         FROM agena_sessions WHERE id=? AND deleted_at_ms IS NULL",
    )
    .bind(session.get())
    .fetch_optional(connection)
    .await
    .map_err(sql_error)?
    .ok_or(KernelError::Ownership)?;
    Ok(state.bounded_snapshot(super::external_agent::state_budget(config_bytes as usize)))
}

async fn save_external_agent_resume(
    connection: &mut SqliteConnection,
    scope: Scope,
    view: &ExecutionView,
    phase: ExecutionPhase,
    resume: &ExternalAgentResume,
) -> Result<()> {
    if phase != ExecutionPhase::Succeeded
        || resume.session != scope.session
        || resume.message != view.message_id
        || !resume.cwd.is_absolute()
        || resume.native_id.is_empty()
        || resume.native_id.len() > 4096
    {
        return Err(invalid("invalid external agent continuation"));
    }
    let valid: bool = sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1 FROM agena_rounds r
            JOIN agena_sessions s ON s.id=?
            WHERE r.id=? AND r.execution_id=? AND r.generation=? AND r.phase='succeeded'
              AND json_extract(s.configuration_json,'$.external_agent') IS NOT NULL
        )",
    )
    .bind(scope.session.get())
    .bind(resume.round.get())
    .bind(scope.execution.to_string())
    .bind(scope.generation.get())
    .fetch_one(&mut *connection)
    .await
    .map_err(sql_error)?;
    if !valid {
        return Err(KernelError::Ownership);
    }
    let link = ExternalAgentLink {
        native_id: resume.native_id.clone(),
        cwd: resume.cwd.clone(),
        incarnation: resume.incarnation,
        resume_at: None,
    };
    super::external_agent::validate_link(&link)?;
    let metadata = reads::metadata(connection, scope.session).await?;
    let configuration = lifecycle::configuration(connection, scope.session).await?;
    if resume.incarnation.is_some() && configuration.external_agent_link.as_ref() != Some(&link) {
        return Err(KernelError::Ownership);
    }
    let workspace = metadata.workspace_id.ok_or(KernelError::Ownership)?;
    let agent = configuration
        .external_agent
        .clone()
        .ok_or(KernelError::Ownership)?;
    if super::external_agent::existing_link(connection, workspace, agent, &link)
        .await?
        .is_some_and(|owner| owner != scope.session)
    {
        return Err(KernelError::Conflict(
            "this native conversation is already linked".into(),
        ));
    }
    let base: String = sqlx::query_scalar(
        "SELECT json_set(json_remove(configuration_json,'$.external_agent_state','$.external_agent_pending_turn'),
                '$.external_agent_resume',json(?),'$.external_agent_link',json(?))
         FROM agena_sessions WHERE id=?",
    )
    .bind(encode(resume)?)
    .bind(encode(&link)?)
    .bind(scope.session.get())
    .fetch_one(&mut *connection)
    .await
    .map_err(sql_error)?;
    let snapshot = configuration
        .external_agent_state
        .bounded_snapshot(super::external_agent::state_budget(base.len()));
    sqlx::query(
        "UPDATE agena_sessions SET configuration_json=CASE WHEN ? THEN ?
            ELSE json_set(?,'$.external_agent_state',json(?)) END WHERE id=?",
    )
    .bind(snapshot.is_empty())
    .bind(&base)
    .bind(&base)
    .bind(encode(&snapshot)?)
    .bind(scope.session.get())
    .execute(connection)
    .await
    .map_err(sql_error)?;
    Ok(())
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
        if view.phase == ExecutionPhase::Running {
            sqlx::query("UPDATE agena_sessions SET configuration_json=CASE
                    WHEN json_extract(configuration_json,'$.external_agent_state.antigravity_legacy_history')=1
                    THEN json_set(configuration_json,'$.external_agent_state',json_object(
                        'antigravity_legacy_history',json('true')))
                    ELSE json_remove(configuration_json,'$.external_agent_state') END
                WHERE id=? AND json_type(configuration_json,'$.external_agent_state') IS NOT NULL
                    AND json_extract(configuration_json,'$.external_agent_resume') IS NULL")
                .bind(view.session_id.get())
                .execute(&mut *connection)
                .await
                .map_err(sql_error)?;
        }
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
        "UPDATE agena_sessions SET configuration_json=json_remove(configuration_json,'$.external_agent_state') WHERE id IN (SELECT session_id FROM agena_attempts WHERE phase='running') AND json_type(configuration_json,'$.external_agent_state') IS NOT NULL AND json_extract(configuration_json,'$.external_agent_resume') IS NULL",
        "UPDATE agena_interactions SET decision_json='{\"kind\":\"cancel\"}',decided_at_ms=? WHERE decision_json IS NULL AND tool_id IN (SELECT id FROM agena_tool_calls WHERE phase IN ('preparing','running'))",
        "UPDATE agena_entries SET revision=revision+1 WHERE tool_id IN (SELECT id FROM agena_tool_calls WHERE phase IN ('preparing','running'))",
        "UPDATE agena_tool_calls SET phase='interrupted',result_json='null',finished_at_ms=? WHERE phase IN ('preparing','running')",
        "UPDATE agena_rounds SET phase='interrupted',finished_at_ms=? WHERE phase='running'",
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
            )?;
            if body.len() > 256 * 1024 {
                return Err(invalid("composer document exceeds 256 KiB"));
            }
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
