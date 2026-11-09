//! Operator selection and queue controls over native session commands. Main
//! dispatch owns live-call, canonical-user, quote and operation-key authority.
use super::*;
use agena_domain::ActivityPayload;

#[derive(Serialize, Deserialize)]
struct ControlCursor {
    session: i64,
    action: BotAction,
    query: Option<String>,
    version: i64,
    offset: usize,
}

#[derive(Serialize, Deserialize)]
struct PreparedSelection {
    request: BotRequest,
    changes: Vec<SelectionChange>,
    before: Option<ExecutionSelection>,
}
#[derive(Serialize, Deserialize)]
struct PreparedQueue {
    request: BotRequest,
    status: String,
}

fn selection_summary(selection: &ExecutionSelection) -> Value {
    json!({"provider":selection.provider,"adapter":selection.adapter,"model":selection.model,
        "thinking_mode":selection.thinking_mode,"speed_mode":selection.speed_mode,
        "verbosity":selection.verbosity,"parallel_tool_calls":selection.parallel_tool_calls})
}

fn control_error(error: KernelError) -> AppError {
    match error {
        KernelError::Conflict(message) => AppError::PersistenceConflict(message),
        KernelError::StaleAttempt => AppError::PersistenceConflict(
            "the execution generation changed; inspect the queue again".into(),
        ),
        other => kernel_error(other),
    }
}

impl SessionManager {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn bot_control(
        &self,
        source: i64,
        _call: i64,
        r: &BotRequest,
        command: RequestId,
        origin: Option<i64>,
        _key: Option<&str>,
    ) -> Result<Value, AppError> {
        let target = if matches!(r.action, BotAction::Models | BotAction::Queue) {
            self.bot_target(source, r.session_id).await?
        } else {
            // Main validates target scope before persisting the first Intent.
            // A prepared native command must remain replayable after deletion.
            r.session_id
                .ok_or_else(|| invalid("session_id is required"))?
        };
        let session = session_id(target)?;
        match r.action {
            BotAction::Models => self.bot_models(target, r).await,
            BotAction::Queue => self.bot_queue(target, r).await,
            BotAction::Configure => {
                let dispatch = self.session_dispatch(session);
                let _submission = dispatch.submission.lock().await;
                let expected = r
                    .expected_version
                    .ok_or_else(|| invalid("configure requires the observed expected_version"))?;
                if r.reset_model && r.model.is_some() {
                    return Err(invalid("reset_model cannot be combined with model"));
                }
                let preparation_key = format!("operator-controls-selection:{command}");
                let prepared = match self
                    .bot_state::<PreparedSelection>(&preparation_key)
                    .await?
                {
                    Some(prepared) => {
                        if prepared.request != *r {
                            return Err(invalid(
                                "operation key was reused with different selection fields",
                            ));
                        }
                        prepared
                    }
                    None => {
                        let before = self
                            .kernel()
                            .reader
                            .metadata(session)
                            .await
                            .map_err(control_error)?;
                        if before.metadata_version.get() != expected {
                            return Err(AppError::PersistenceConflict(
                                "session selection changed; inspect before configuring".into(),
                            ));
                        }
                        let mut proposed = before.selection.clone().unwrap_or_default();
                        let mut changes = Vec::new();
                        if r.reset_model || r.model.is_some() {
                            // Native Model(None) removes only the route override. Any
                            // unmentioned mode or permission field remains intact.
                            proposed.provider = r.model.as_ref().map(|m| m.provider_id.to_string());
                            proposed.adapter = r
                                .model
                                .as_ref()
                                .and_then(|m| m.adapter_id.as_ref().map(ToString::to_string));
                            proposed.model = r.model.as_ref().map(|m| m.model_id.to_string());
                            changes.push(SelectionChange::Model(r.model.clone()));
                        }
                        for (value, field, change) in [
                            (
                                &r.thinking_mode,
                                &mut proposed.thinking_mode,
                                SelectionChange::ThinkingMode
                                    as fn(Option<String>) -> SelectionChange,
                            ),
                            (
                                &r.speed_mode,
                                &mut proposed.speed_mode,
                                SelectionChange::SpeedMode,
                            ),
                            (
                                &r.verbosity,
                                &mut proposed.verbosity,
                                SelectionChange::Verbosity,
                            ),
                        ] {
                            if let Some(value) = value {
                                if value.trim().is_empty() || value.len() > 128 {
                                    return Err(invalid(
                                        "model mode must be nonempty and within 128 bytes",
                                    ));
                                }
                                let value = value.trim().to_owned();
                                *field = Some(value.clone());
                                changes.push(change(Some(value)));
                            }
                        }
                        if let Some(value) = r.parallel_tool_calls {
                            proposed.parallel_tool_calls = Some(value);
                            changes.push(SelectionChange::ParallelToolCalls(Some(value)));
                        }
                        if changes.is_empty() {
                            return Err(invalid("configure requires at least one selection field"));
                        }
                        let state = self.state.load_full();
                        let effective = state
                            .config
                            .default_selection
                            .overlay_with_cascade(&proposed);
                        let model = state
                            .providers
                            .resolve_default_model_selection(&effective)?;
                        if let Some(model) = model {
                            // Use the existing human provider inventory and explicit
                            // switch budget check; arbitrary model strings are refused.
                            let available = state.providers.selection_model_inventory();
                            let mut canonical = model.clone();
                            canonical.adapter_id =
                                state.providers.tool_execution_adapter(&effective)?;
                            if !available.iter().any(|entry| entry.reference() == canonical) {
                                return Err(invalid(
                                    "model is not in the configured provider inventory; inspect models",
                                ));
                            }
                            validate_modes(&state.providers, &model, &effective)?;
                            self.ensure_model_fits_session(target, &model).await?;
                        } else if !r.reset_model
                            || effective.thinking_mode.is_some()
                            || effective.speed_mode.is_some()
                            || effective.verbosity.is_some()
                            || effective.parallel_tool_calls.is_some()
                        {
                            return Err(invalid(
                                "select a configured provider/model before configuring modes",
                            ));
                        }
                        let prepared = PreparedSelection {
                            request: r.clone(),
                            changes,
                            before: before.selection.clone(),
                        };
                        self.save_bot_state(&preparation_key, &prepared).await?;
                        prepared
                    }
                };
                let metadata = persist(|| {
                    self.kernel().sessions.patch_selection(
                        command,
                        session,
                        expected,
                        prepared.changes.clone(),
                    )
                })
                .await
                .map_err(control_error)?
                .value;
                Ok(
                    json!({"session_id":target,"version":metadata.metadata_version.get(),
                    "status":if metadata.selection == prepared.before {"no_change"} else {"configured"},
                    "selection":metadata.selection.as_ref().map(selection_summary),"in_flight_run_affected":false,
                    "applies_to":"future_runs","origin":{"kind":"operator_bot","session_id":source,"user_message_id":origin},"url":link(target)}),
                )
            }
            BotAction::QueuePause | BotAction::QueueEdit | BotAction::QueueRemove => {
                let expected = r.expected_control_version.ok_or_else(|| {
                    invalid("queue mutation requires the observed expected_control_version")
                })?;
                let change = if r.action == BotAction::QueuePause {
                    QueueChange::Pause(
                        r.paused
                            .ok_or_else(|| invalid("queue_pause requires paused"))?,
                    )
                } else {
                    let execution = required_text(&r.execution_id, "execution_id", 128)?
                        .parse()
                        .map_err(|_| invalid("invalid execution_id"))?;
                    let generation =
                        AttemptGeneration::try_from(r.expected_generation.ok_or_else(|| {
                            invalid("queue mutation requires the observed expected_generation")
                        })?)
                        .map_err(|_| invalid("invalid expected_generation"))?;
                    if r.action == BotAction::QueueEdit {
                        QueueChange::EditText {
                            execution,
                            generation,
                            text: required_text(&r.instruction, "instruction", 16 * 1024)?.into(),
                        }
                    } else {
                        QueueChange::Remove {
                            execution,
                            generation,
                        }
                    }
                };
                let dispatch = self.session_dispatch(session);
                let _submission = dispatch.submission.lock().await;
                let preparation_key = format!("operator-controls-queue:{command}");
                let prepared = match self.bot_state::<PreparedQueue>(&preparation_key).await? {
                    Some(prepared) => {
                        if prepared.request != *r {
                            return Err(invalid(
                                "operation key was reused with different queue fields",
                            ));
                        }
                        prepared
                    }
                    None => {
                        let before = self
                            .kernel()
                            .reader
                            .control_snapshot(session, 128)
                            .await
                            .map_err(control_error)?;
                        let status = match &change {
                            QueueChange::Pause(value)
                                if before.configuration.message_queue_paused == *value =>
                            {
                                "no_change"
                            }
                            QueueChange::Pause(true) => "paused",
                            QueueChange::Pause(false) => "resumed",
                            QueueChange::EditText {
                                execution, text, ..
                            } => {
                                let run = self
                                    .kernel()
                                    .reader
                                    .execution(session, *execution)
                                    .await
                                    .map_err(control_error)?;
                                let entry = self
                                    .kernel()
                                    .reader
                                    .entry(session, run.message_id)
                                    .await
                                    .map_err(control_error)?;
                                if matches!(&entry.payload,EntryPayload::User {document} if document.text() == *text)
                                {
                                    "no_change"
                                } else {
                                    "edited"
                                }
                            }
                            QueueChange::Remove { .. } => "removed",
                        };
                        let prepared = PreparedQueue {
                            request: r.clone(),
                            status: status.into(),
                        };
                        self.save_bot_state(&preparation_key, &prepared).await?;
                        prepared
                    }
                };
                let receipt = persist(|| {
                    self.kernel()
                        .sessions
                        .control_queue(command, session, expected, change.clone())
                })
                .await
                .map_err(control_error)?;
                // The command is already durable. A supervision failure must
                // return an accepted receipt so callers never repeat an effect
                // under a new operation key. Replaying this key retries recovery.
                let dispatch_error = if matches!(change, QueueChange::Pause(false)) {
                    self.restore_queued_executions(session)
                        .await
                        .err()
                        .map(|error| bounded(&error.to_string(), 1024))
                } else {
                    None
                };
                let control_version = self
                    .kernel()
                    .reader
                    .control_version(session)
                    .await
                    .ok()
                    .map(|v| *v);
                let status = &prepared.status;
                Ok(
                    json!({"session_id":target,"accepted":true,"status":status,"control_version":control_version,
                    "target_deleted":control_version.is_none(),"dispatch":if dispatch_error.is_some() {"blocked"} else {"ready"},"dispatch_error":dispatch_error,
                    "version":receipt.value.metadata_version.get(),"execution_id":r.execution_id,
                    "generation":r.expected_generation,"attachments_preserved":r.action == BotAction::QueueEdit,
                    "origin":{"kind":"operator_bot","session_id":source,"user_message_id":origin},"url":link(target)}),
                )
            }
            _ => Err(invalid("unsupported control action")),
        }
    }

    async fn bot_models(&self, target: i64, r: &BotRequest) -> Result<Value, AppError> {
        let metadata = self
            .kernel()
            .reader
            .metadata(session_id(target)?)
            .await
            .map_err(control_error)?;
        let state = self.state.load_full();
        let effective = state
            .config
            .default_selection
            .overlay_with_cascade(&metadata.selection.clone().unwrap_or_default());
        // Read only registered configured/cached routes; the public catalog
        // and default strings are never used to invent selectable models.
        let mut inventory = state.providers.selection_model_inventory();
        inventory.sort_by_key(|entry| entry.reference().to_string());
        inventory.dedup_by(|a, b| a.reference() == b.reference());
        let query = r.query.as_deref().unwrap_or("").trim().to_lowercase();
        if query.len() > 512 {
            return Err(invalid("model query exceeds 512 bytes"));
        }
        inventory.retain(|m| {
            m.reference().to_string().to_lowercase().contains(&query)
                || m.display_name
                    .as_deref()
                    .is_some_and(|name| name.to_lowercase().contains(&query))
        });
        let offset = control_offset(r, target, metadata.metadata_version.get())?;
        let limit = r.limit.unwrap_or(20).clamp(1, 50) as usize;
        let entries = inventory.iter().skip(offset).take(limit).map(|m| {
            let reference = m.reference();
            json!({"model":reference,"display_name":m.display_name,
                "thinking_modes":m.thinking_modes.iter().map(|mode|json!({"name":mode.selector(),"default":mode.is_default})).collect::<Vec<_>>(),
                "speed_modes":m.speed_modes.iter().map(|(name, mode)|json!({"name":name,"default":mode.is_default})).collect::<Vec<_>>(),
                "verbosity":m.metadata.supported_verbosity_levels_for_model(&m.id),
                "default_verbosity":m.metadata.default_verbosity,
                "parallel_tool_calls":m.metadata.supports_parallel_tool_calls_for_model()})
        }).collect::<Vec<_>>();
        let next = next_control_cursor(
            r,
            target,
            metadata.metadata_version.get(),
            offset,
            entries.len(),
            inventory.len(),
        )?;
        let mut resolved = self
            .options_for_selection(&metadata.selection.clone().unwrap_or_default())
            .ok();
        if let Some(options) = &mut resolved {
            self.apply_model_mode_requests(options)?;
            if options.verbosity.is_none() {
                options.verbosity = state
                    .providers
                    .model_metadata(&options.model)?
                    .default_verbosity;
            }
        }
        Ok(
            json!({"session_id":target,"version":metadata.metadata_version.get(),"models":entries,
            "selection":metadata.selection.as_ref().map(selection_summary),"effective_selection":selection_summary(&effective),
            "effective_model":resolved.as_ref().map(|o| &o.model),
            "effective_modes":resolved.map(|o|json!({"thinking_mode":o.thinking_mode,"speed_mode":o.speed_mode,"verbosity":o.verbosity,"parallel_tool_calls":o.model_supports_parallel_tool_calls})),
            "default_selection":selection_summary(&state.config.default_selection),"has_more":next.is_some(),"next_cursor":next,
            "inventory_source":"native_configured_and_cached_provider_inventory","url":link(target)}),
        )
    }

    async fn bot_queue(&self, target: i64, r: &BotRequest) -> Result<Value, AppError> {
        let session = session_id(target)?;
        let snapshot = self
            .kernel()
            .reader
            .control_snapshot(session, 128)
            .await
            .map_err(control_error)?;
        if snapshot.active_executions.has_more {
            return Err(invalid(
                "native active queue exceeds snapshot budget; inspect native session",
            ));
        }
        let offset = control_offset(r, target, snapshot.control_version)?;
        let limit = r.limit.unwrap_or(20).clamp(1, 50) as usize;
        let mut queued = snapshot
            .active_executions
            .executions
            .iter()
            .filter(|run| run.phase == ExecutionPhase::Queued)
            .collect::<Vec<_>>();
        queued.sort_by_key(|run| run.message_id);
        let mut entries = Vec::new();
        let mut response_bytes = 0usize;
        for run in queued.iter().skip(offset).take(limit) {
            let entry = self
                .kernel()
                .reader
                .entry(session, run.message_id)
                .await
                .map_err(control_error)?;
            let (text, attachments) = match &entry.payload {
                EntryPayload::User { document } => {
                    let attachments = document.0.iter().filter_map(|node| match node {
                        ComposerNode::Activity { activity } => Some(match &activity.payload {
                            ActivityPayload::Resource(resource) => json!({"activity_id":activity.id,"kind":resource.kind,"name":bounded(&resource.name,256),"media_type":resource.media_type.as_ref().map(|kind|bounded(kind,128)),"size_bytes":resource.size_bytes}),
                            _ => json!({"activity_id":activity.id,"kind":"activity"}),
                        }),
                        _ => None,
                    }).collect::<Vec<_>>();
                    (document.text(), attachments)
                }
                EntryPayload::Notice { text } => (text.clone(), Vec::new()),
                _ => (String::new(), Vec::new()),
            };
            let attachment_count = attachments.len();
            let attachments = attachments.into_iter().take(8).collect::<Vec<_>>();
            let preview = json!({"execution_id":run.id,"generation":run.generation.get(),"entry_id":run.message_id.get(),
                "revision":entry.revision.get(),"purpose":run.purpose,"text":bounded(&text,1024),"text_has_more":text.len()>1024,
                "attachments":attachments,"attachment_count":attachment_count,"attachments_has_more":attachment_count>8,
                "editable":run.purpose == ExecutionPurpose::Message && !entry.execution_started});
            let bytes = serde_json::to_vec(&preview)?.len();
            if response_bytes.saturating_add(bytes) > RESPONSE_BUDGET.saturating_sub(8192) {
                break;
            }
            response_bytes += bytes;
            entries.push(preview);
        }
        // Entry text is a separate bounded native read: fence it against an edit,
        // removal, claim or pause before returning actionable version guards.
        let after = *self
            .kernel()
            .reader
            .control_version(session)
            .await
            .map_err(control_error)?;
        if after != snapshot.control_version {
            return Err(AppError::PersistenceConflict(
                "queue changed while reading; retry queue".into(),
            ));
        }
        let next = next_control_cursor(
            r,
            target,
            snapshot.control_version,
            offset,
            entries.len(),
            queued.len(),
        )?;
        Ok(
            json!({"session_id":target,"control_version":snapshot.control_version,"paused":snapshot.configuration.message_queue_paused,
            "entries":entries,"total":queued.len(),"has_more":next.is_some(),"next_cursor":next,"url":link(target)}),
        )
    }
}

fn validate_modes(
    providers: &ProviderRegistry,
    model: &ModelRef,
    selection: &ExecutionSelection,
) -> Result<(), AppError> {
    if let Some(name) = &selection.thinking_mode
        && !providers
            .model_thinking_modes(model)?
            .iter()
            .any(|mode| mode.selector().as_deref() == Some(name))
    {
        return Err(invalid(
            "thinking_mode is not supported by the selected model",
        ));
    }
    if let Some(name) = &selection.speed_mode
        && !providers.model_speed_modes(model)?.contains_key(name)
    {
        return Err(invalid("speed_mode is not supported by the selected model"));
    }
    let metadata = providers.model_metadata(model)?;
    if let Some(name) = &selection.verbosity
        && !metadata.supports_verbosity_level_for_model(&model.model_id, name)
    {
        return Err(invalid("verbosity is not supported by the selected model"));
    }
    if selection.parallel_tool_calls == Some(true)
        && !metadata.supports_parallel_tool_calls_for_model()
    {
        return Err(invalid(
            "parallel_tool_calls is not supported by the selected model",
        ));
    }
    Ok(())
}

fn control_offset(r: &BotRequest, target: i64, version: i64) -> Result<usize, AppError> {
    match cursor::<ControlCursor>(&r.cursor)? {
        Some(cursor)
            if cursor.session == target
                && cursor.action == r.action
                && cursor.query == r.query
                && cursor.version == version =>
        {
            Ok(cursor.offset)
        }
        Some(_) => Err(invalid(
            "control cursor does not match this target, query or current version; restart paging",
        )),
        None => usize::try_from(r.offset).map_err(|_| invalid("offset exceeds platform limit")),
    }
}
fn next_control_cursor(
    r: &BotRequest,
    target: i64,
    version: i64,
    offset: usize,
    count: usize,
    total: usize,
) -> Result<Option<String>, AppError> {
    (offset.saturating_add(count) < total)
        .then(|| {
            serde_json::to_string(&ControlCursor {
                session: target,
                action: r.action,
                query: r.query.clone(),
                version,
                offset: offset.saturating_add(count),
            })
            .map_err(AppError::from)
        })
        .transpose()
}

#[cfg(test)]
#[path = "control_tests.rs"]
mod tests;
