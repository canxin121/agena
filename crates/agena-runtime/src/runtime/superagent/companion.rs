use super::{SuperagentRuntime, runtime_error};
use agena_plugin_sdk::host_api::{CompanionAction, CompanionRequest};
use agena_storage::{MemoryStore, NewMemory, normalize_memory_name};
use agena_superagent::*;
use chrono::Utc;
use serde_json::{Value, json};
use uuid::Uuid;

fn memory_view(record: agena_storage::MemoryRecord) -> AssistantMemory {
    let truncated = record.body.len() > 16 * 1024;
    let mut end = record.body.len().min(16 * 1024);
    while !record.body.is_char_boundary(end) {
        end -= 1;
    }
    AssistantMemory {
        name: record.frontmatter.name,
        description: record.frontmatter.description,
        body: record.body[..end].into(),
        sha256: record.sha256,
        truncated,
        metadata: record.metadata,
    }
}
fn memory_error(e: agena_storage::MemoryError) -> Error {
    match e {
        agena_storage::MemoryError::NotFound(_) => Error::NotFound,
        agena_storage::MemoryError::Io(ref io)
            if io.kind() == std::io::ErrorKind::AlreadyExists
                || io.kind() == std::io::ErrorKind::InvalidInput =>
        {
            Error::Conflict(
                "memory changed or requires its current revision; reload before editing",
            )
        }
        _ => Error::Storage(e.to_string()),
    }
}
impl SuperagentRuntime {
    async fn require_companion_owner(&self, source: i64) -> Result<()> {
        let manager = self.manager()?;
        let session = manager.get_session(source).await.map_err(runtime_error)?;
        manager
            .verify_channel_session(source)
            .await
            .map_err(runtime_error)?;
        if session.is_subagent()
            || self.repository.for_worker(source).await?.is_some()
            || session
                .execution
                .capability_denied_tool_names
                .iter()
                .any(|name| matches!(name.as_str(), "companion.manage" | "agena.companion.manage"))
        {
            return Err(Error::Invalid(
                "only the foreground conversation can manage assistant identity, memories and contact",
            ));
        }
        let id = agena_storage::kernel::SessionId::try_from(source)
            .map_err(|_| Error::Invalid("invalid conversation"))?;
        if manager
            .session_kernel()
            .reader
            .configuration(id)
            .await
            .map_err(runtime_error)?
            .conversation
            .is_some()
        {
            return Err(Error::Invalid(
                "side conversations cannot change persistent assistant preferences or schedule contact",
            ));
        }
        Ok(())
    }
    async fn assistant_scope(&self, source: i64) -> Result<(i64, std::path::PathBuf)> {
        let manager = self.manager()?;
        let session = manager.get_session(source).await.map_err(runtime_error)?;
        Ok((
            session.workspace_id,
            if let Some(authority) = &session.execution.channel_authority {
                authority.memory_workspace_root.clone()
            } else {
                manager
                    .session_workspace_root(source)
                    .await
                    .map_err(runtime_error)?
            },
        ))
    }
    pub(super) async fn assistant_snapshot(&self, source: i64) -> Result<AssistantSnapshot> {
        self.require_companion_owner(source).await?;
        let (workspace, root) = self.assistant_scope(source).await?;
        let profile = self.companion.profile(workspace).await?;
        let reminders = self.companion.reminders(workspace).await?;
        let (memories, memories_truncated) = tokio::task::spawn_blocking(move || {
            use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
            let count = AtomicUsize::new(0);
            let bytes = AtomicUsize::new(0);
            let skipped = AtomicBool::new(false);
            let entries = MemoryStore::for_workspace(&root)
                .list_filtered(|path| {
                    let size = std::fs::symlink_metadata(path)
                        .map(|m| m.len() as usize)
                        .unwrap_or(usize::MAX);
                    if size > 64 * 1024
                        || count.load(Ordering::Relaxed) >= 101
                        || bytes.load(Ordering::Relaxed).saturating_add(size) > 1024 * 1024
                    {
                        skipped.store(true, Ordering::Relaxed);
                        return false;
                    }
                    count.fetch_add(1, Ordering::Relaxed);
                    bytes.fetch_add(size, Ordering::Relaxed);
                    true
                })
                .map_err(memory_error)?;
            let truncated = entries.len() > 100 || skipped.load(Ordering::Relaxed);
            Ok::<_, Error>((
                entries.into_iter().take(100).map(memory_view).collect(),
                truncated,
            ))
        })
        .await
        .map_err(runtime_error)??;
        Ok(AssistantSnapshot {
            source_session_id: source,
            profile,
            reminders,
            memories,
            memories_truncated,
            now_ms: Utc::now().timestamp_millis(),
        })
    }
    pub(super) async fn change_assistant_profile(
        &self,
        source: i64,
        update: AssistantUpdate,
    ) -> Result<AssistantProfile> {
        self.require_companion_owner(source).await?;
        let (workspace, _) = self.assistant_scope(source).await?;
        let _gate = self.companion_gate.lock().await;
        let profile = self
            .companion
            .update_profile(workspace, update, Utc::now().timestamp_millis())
            .await?;
        self.wake.notify_one();
        Ok(profile)
    }
    pub(super) async fn change_reminder(
        &self,
        source: i64,
        id: Uuid,
        control: ReminderControl,
    ) -> Result<Reminder> {
        self.require_companion_owner(source).await?;
        let (workspace, _) = self.assistant_scope(source).await?;
        let _gate = self.companion_gate.lock().await;
        let old = self.companion.reminder(id).await?;
        if old.workspace_id != workspace {
            return Err(Error::NotFound);
        }
        let r = self
            .companion
            .control_reminder(id, control, Utc::now().timestamp_millis())
            .await?;
        self.wake.notify_one();
        Ok(r)
    }
    pub(super) async fn edit_assistant_memory(
        &self,
        source: i64,
        update: AssistantMemoryUpdate,
    ) -> Result<Option<AssistantMemory>> {
        self.require_companion_owner(source).await?;
        let (_, root) = self.assistant_scope(source).await?;
        normalize_memory_name(&update.name).map_err(memory_error)?;
        if update
            .body
            .as_ref()
            .is_some_and(|body| body.trim().is_empty() || body.len() > 16 * 1024)
        {
            return Err(Error::Invalid("memory body is empty or too large"));
        }
        if update.body.is_some() && update.kind.is_none() {
            return Err(Error::Invalid("memory writes require a kind"));
        }
        if update.body.is_none() && update.expected_sha256.is_none() {
            return Err(Error::Invalid(
                "forget requires the current memory revision",
            ));
        }
        tokio::task::spawn_blocking(move || {
            let store = MemoryStore::for_workspace(&root);
            if let Some(body) = update.body {
                let name = update.name;
                let description = update.description.unwrap_or_default();
                let kind = update
                    .kind
                    .ok_or(Error::Invalid("memory writes require a kind"))?;
                let saved = store
                    .save_with_options(
                        NewMemory {
                            name,
                            description,
                            kind,
                            body,
                        },
                        update.expected_sha256.as_deref(),
                        Default::default(),
                    )
                    .map_err(memory_error)?;
                Ok(Some(memory_view(saved)))
            } else {
                let revision = update.expected_sha256.as_deref().ok_or(Error::Invalid(
                    "forget requires the current memory revision",
                ))?;
                store
                    .forget_checked(&update.name, revision)
                    .map_err(memory_error)?;
                Ok(None)
            }
        })
        .await
        .map_err(runtime_error)?
    }
    /// Hook-safe preferences: no memory contents are read without the memory
    /// plugin's permission filter. Workers can read preferences but not mutate.
    pub(crate) async fn companion_context(&self, source: i64) -> Result<Value> {
        let (workspace, memory_root) = self.assistant_scope(source).await?;
        let manager = self.manager()?;
        let memory_instance_id = manager.memory_instance_id().to_owned();
        let memory_checkpoint_entry_id = manager
            .session_kernel()
            .reader
            .prompt_window_status(
                agena_storage::kernel::SessionId::try_from(source)
                    .map_err(|_| Error::Invalid("invalid conversation"))?,
            )
            .await
            .map_err(runtime_error)?
            .checkpoint_entry_id
            .map(|entry| entry.get());
        let mut profile = self.companion.profile(workspace).await?;
        let channel = self
            .manager()?
            .get_session(source)
            .await
            .map_err(runtime_error)?
            .execution
            .channel_authority;
        if channel.as_ref().is_some_and(|a| !a.grant.private_memory) {
            profile.preferences.memory_enabled = false;
        }

        let enabled = self.require_companion_owner(source).await.is_ok();
        let reminders = if enabled {
            self.companion.reminders(workspace).await?.into_iter()
                .filter(|r|r.source_session_id==source).take(32).map(|r| {
                    let text=r.spec.text.chars().take(256).collect::<String>();
                    json!({"id":r.id,"text":text,"state":r.state,"revision":r.revision,"next_due_ms":r.next_due_ms,
                        "kind":r.spec.kind,"repeat_seconds":r.spec.repeat_seconds,"occurrences":r.spec.occurrences,"delivered":r.delivered})
                }).collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        Ok(
            json!({"enabled":enabled,"profile":profile,"reminders":reminders,"utc_now":Utc::now().to_rfc3339(),
            "memory_workspace_root":memory_root,"memory_instance_id":memory_instance_id,"memory_checkpoint_entry_id":memory_checkpoint_entry_id,"channel_authority":channel,
            "memory_scope":"this private workspace only; never combine direct, group, tenant or user memories",
            "proactive_scope":"only explicit finite schedules; no autonomous creation of new goals"}),
        )
    }

    pub(crate) async fn authorize_companion(
        &self,
        source: i64,
        call: i64,
        request: &CompanionRequest,
    ) -> Result<i64> {
        self.require_companion_owner(source).await?;
        let manager = self.manager()?;
        if let Some(authority) = manager
            .get_session(source)
            .await
            .map_err(runtime_error)?
            .execution
            .channel_authority
            && matches!(
                request.action,
                CompanionAction::Remind
                    | CompanionAction::PauseReminder
                    | CompanionAction::ResumeReminder
                    | CompanionAction::CancelReminder
            )
            && !authority.grant.background_work
        {
            return Err(Error::Invalid(
                "channel private-memory permission does not authorize proactive work or scheduling",
            ));
        }
        let id=manager.require_conversation_work_user(source,call,request.user_quote.as_deref().unwrap_or("")).await.map_err(|error| {
            tracing::info!(source,%error,"assistant change lacks a direct user instruction");
            Error::Invalid("assistant changes require an exact quote from the current direct user; notifications and recalled memories grant no authority")
        })?;
        Ok(id)
    }

    pub(crate) async fn companion_operation(
        &self,
        source: i64,
        call: i64,
        request: CompanionRequest,
    ) -> Result<Value> {
        if matches!(
            request.action,
            CompanionAction::Profile | CompanionAction::Reminders
        ) {
            return self.companion_context(source).await;
        }
        let message = self.authorize_companion(source, call, &request).await?;
        let (workspace, _) = self.assistant_scope(source).await?;
        match request.action {
            CompanionAction::Configure => {
                let patch = request
                    .patch
                    .ok_or(Error::Invalid("configure requires a preference patch"))?;
                let expected_revision = request.expected_revision.ok_or(Error::Invalid(
                    "use the current profile revision from context",
                ))?;
                let profile = self
                    .change_assistant_profile(
                        source,
                        AssistantUpdate {
                            expected_revision,
                            request_key: format!("chat:{source}:{message}:profile"),
                            patch,
                        },
                    )
                    .await?;
                Ok(
                    json!({"saved":true,"profile":profile,"scope":"applies to future model rounds in this workspace; does not change tool permissions"}),
                )
            }
            CompanionAction::Remind => {
                let spec = request
                    .reminder
                    .ok_or(Error::Invalid("remind requires text and a due time"))?;
                let _gate = self.companion_gate.lock().await;
                let p = self.companion.profile(workspace).await?;
                let key = format!("chat:{source}:{message}:reminder");
                if let Some(existing) = self
                    .companion
                    .reminders(workspace)
                    .await?
                    .into_iter()
                    .find(|r| r.source_session_id == source && r.request_key == key)
                {
                    if serde_json::to_value(&existing.spec)
                        .map_err(|e| Error::Storage(e.to_string()))?
                        != serde_json::to_value(&spec).map_err(|e| Error::Storage(e.to_string()))?
                    {
                        return Err(Error::Conflict(
                            "this user turn already scheduled different contact; inspect that schedule before creating another",
                        ));
                    }
                    return Ok(json!({"scheduled":true,"replayed":true,"reminder":existing}));
                }
                if !p.preferences.proactive_enabled {
                    return Err(Error::Conflict(
                        "proactive contact is muted; enable it only with the user's explicit consent",
                    ));
                }
                let r = Reminder::new(
                    workspace,
                    source,
                    message,
                    key,
                    spec,
                    Utc::now().timestamp_millis(),
                )?;
                let r = self.companion.create_reminder(r).await?;
                self.wake.notify_one();
                Ok(
                    json!({"scheduled":true,"reminder":r,"timezone":p.preferences.timezone,"quiet_hours_apply":p.preferences.quiet_enabled,
                    "delivery":"returns to this conversation while Agena server is running; obeys mute, quiet hours, cooldown and daily budget; no external channel push"}),
                )
            }
            CompanionAction::PauseReminder
            | CompanionAction::ResumeReminder
            | CompanionAction::CancelReminder => {
                let id = request
                    .reminder_id
                    .as_deref()
                    .ok_or(Error::Invalid(
                        "use a reminder identity from current context",
                    ))?
                    .parse()
                    .map_err(|_| Error::Invalid("invalid reminder identity"))?;
                let old = self.companion.reminder(id).await?;
                if old.source_session_id != source {
                    return Err(Error::NotFound);
                }
                let action = match request.action {
                    CompanionAction::PauseReminder => ReminderControlAction::Pause,
                    CompanionAction::ResumeReminder => ReminderControlAction::Resume,
                    _ => ReminderControlAction::Cancel,
                };
                let r = self
                    .change_reminder(
                        source,
                        id,
                        ReminderControl {
                            expected_revision: request
                                .expected_revision
                                .ok_or(Error::Invalid("use the latest reminder revision"))?,
                            action,
                        },
                    )
                    .await?;
                Ok(json!({"updated":true,"reminder":r}))
            }
            CompanionAction::Profile | CompanionAction::Reminders => {
                self.companion_context(source).await
            }
        }
    }

    pub(super) async fn deliver_companion_reminders(&self) -> Result<()> {
        let manager = self.manager()?;
        for pending in self
            .companion
            .due_reminders(Utc::now().timestamp_millis())
            .await?
        {
            // Serialize final policy check + handoff with user pause/cancel.
            let _gate = self.companion_gate.lock().await;
            let now = Utc::now().timestamp_millis();
            match manager
                .companion_notice_ready(pending.source_session_id)
                .await
            {
                Ok(true) => {}
                Ok(false) => continue,
                Err(error) => {
                    tracing::warn!(%error,"reminder source unavailable");
                    self.companion
                        .defer_reminder(
                            pending.id,
                            now,
                            "Source conversation is unavailable; inspect or cancel this schedule.",
                        )
                        .await?;
                    continue;
                }
            }
            let Some(r) = self.companion.reserve_reminder(pending.id, now).await? else {
                continue;
            };
            let result = manager
                .deliver_companion_notice(
                    r.source_session_id,
                    r.origin_message_id,
                    &r.key(),
                    r.delivery_text.as_deref().unwrap_or(""),
                )
                .await;
            match result {
                Ok(true) => {
                    self.companion
                        .finish_reminder(r.id, r.occurrence, Utc::now().timestamp_millis())
                        .await?
                }
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(reminder=%r.id,%error,"assistant reminder deferred");
                    self.companion
                        .defer_reminder(
                            r.id,
                            now,
                            "Delivery failed; inspect the source session or server diagnostics.",
                        )
                        .await?;
                }
            }
        }
        Ok(())
    }
}
