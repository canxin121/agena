//! Instance operator identity and cross-project routing. All effects use the
//! existing workspace repository, session kernel and serialized supervisor.
use super::*;
use agena_plugin_host::sdk::host_api::{BotAction, BotRequest};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

mod control;
mod discovery;
mod manage;
mod registry;

const RESPONSE_BUDGET: usize = 64 * 1024;
/// Upper bound for one Bot management request. Large enough for a full
/// character sheet plus its JSON envelope; the persona ceiling is enforced by
/// `agena_assistant::MAX_BOT_INSTRUCTIONS_BYTES`.
const MAX_BOT_MANAGEMENT_INPUT_BYTES: usize = 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct Intent {
    request: BotRequest,
    result: Option<Value>,
    #[serde(default)]
    lifecycle: Option<manage::LifecyclePreparation>,
}

fn invalid(message: &str) -> AppError {
    AppError::Config(message.into())
}
fn bounded(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.into();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{} [preview limited]", &text[..end])
}
fn required_text<'a>(
    value: &'a Option<String>,
    field: &str,
    max: usize,
) -> Result<&'a str, AppError> {
    let text = value
        .as_deref()
        .ok_or_else(|| invalid(&format!("{field} is required")))?;
    if text.trim().is_empty() || text.len() > max {
        return Err(invalid(&format!("{field} is empty or exceeds {max} bytes")));
    }
    Ok(text)
}
fn cursor<T: serde::de::DeserializeOwned>(value: &Option<String>) -> Result<Option<T>, AppError> {
    value
        .as_ref()
        .map(|text| {
            if text.len() > 2048 {
                return Err(invalid("cursor exceeds size budget"));
            }
            serde_json::from_str(text).map_err(|_| invalid("invalid page cursor"))
        })
        .transpose()
}
fn link(id: i64) -> String {
    format!("/chat?sessionId={id}")
}

fn is_write(action: BotAction) -> bool {
    match action {
        BotAction::RegisterProject
        | BotAction::Create
        | BotAction::Rename
        | BotAction::Organize
        | BotAction::Fork
        | BotAction::Delete
        | BotAction::Configure
        | BotAction::QueuePause
        | BotAction::QueueEdit
        | BotAction::QueueRemove
        | BotAction::Send
        | BotAction::Stop => true,
        BotAction::Projects
        | BotAction::Project
        | BotAction::Sessions
        | BotAction::Read
        | BotAction::ReadText
        | BotAction::Status
        | BotAction::Models
        | BotAction::Queue
        | BotAction::Export => false,
    }
}

/// Refuse ignored inputs: a misplaced model/guard/instruction must never look successful.
fn validate_request(r: &BotRequest) -> Result<(), AppError> {
    use BotAction::*;
    let fields: &[&str] = match r.action {
        Projects => &["query", "cursor", "offset", "limit"],
        Project => &[
            "workspace_id",
            "query",
            "favorite",
            "pinned",
            "states",
            "parent_session_id",
            "cursor",
            "limit",
        ],
        RegisterProject => &["path"],
        Sessions => &[
            "workspace_id",
            "query",
            "favorite",
            "pinned",
            "states",
            "parent_session_id",
            "cursor",
            "offset",
            "limit",
        ],
        Create => &["workspace_id", "title"],
        Read => &["session_id", "cursor", "limit"],
        ReadText => &["session_id", "entry_id", "byte_offset", "max_bytes"],
        Status => &["session_id", "cursor", "limit"],
        Rename => &["session_id", "title", "expected_version"],
        Organize => &["session_id", "favorite", "pinned", "expected_version"],
        Fork => &["session_id", "at_message_id", "title"],
        Delete => &[
            "session_id",
            "expected_version",
            "expected_control_version",
            "expected_entry_count",
        ],
        Models => &["session_id", "query", "cursor", "offset", "limit"],
        Configure => &[
            "session_id",
            "expected_version",
            "model",
            "thinking_mode",
            "speed_mode",
            "verbosity",
            "parallel_tool_calls",
            "reset_model",
        ],
        Queue => &["session_id", "cursor", "offset", "limit"],
        QueuePause => &["session_id", "paused", "expected_control_version"],
        QueueEdit => &[
            "session_id",
            "execution_id",
            "expected_generation",
            "expected_control_version",
            "instruction",
        ],
        QueueRemove => &[
            "session_id",
            "execution_id",
            "expected_generation",
            "expected_control_version",
        ],
        Export => &["session_id"],
        Send => &["session_id", "instruction"],
        Stop => &["session_id", "execution_id", "expected_generation"],
    };
    for (name, value) in serde_json::to_value(r)?
        .as_object()
        .expect("request object")
    {
        let active = !value.is_null()
            && !(name == "offset" && value == 0)
            && !(name == "reset_model" && value == false)
            && !(name == "states" && value.as_array().is_some_and(Vec::is_empty));
        if active
            && name != "action"
            && !fields.contains(&name.as_str())
            && !matches!(name.as_str(), "user_quote" | "request_key")
        {
            return Err(invalid(&format!(
                "field {name} is unused for this action; remove it to avoid an unintended operation"
            )));
        }
    }
    if fields.contains(&"session_id") && r.session_id.is_none_or(|id| id <= 0) {
        return Err(invalid(
            "session_id is required; resolve the name with sessions",
        ));
    }
    if r.workspace_id.is_some_and(|id| id <= 0)
        || r.parent_session_id.is_some_and(|id| id <= 0)
        || r.entry_id.is_some_and(|id| id <= 0)
        || r.at_message_id.is_some_and(|id| id <= 0)
    {
        return Err(invalid(
            "IDs must be positive native IDs returned by discovery",
        ));
    }
    match r.action {
        Project | Create if r.workspace_id.is_none() => {
            return Err(invalid(
                "workspace_id is required; resolve the project with projects",
            ));
        }
        Rename | Organize | Configure | Delete if r.expected_version.is_none() => {
            return Err(invalid(
                "expected_version is required; inspect status first",
            ));
        }
        _ => {}
    }
    match r.action {
        QueuePause | QueueEdit | QueueRemove | Delete if r.expected_control_version.is_none() => {
            return Err(invalid(
                "expected_control_version is required; inspect status/queue first",
            ));
        }
        Organize if r.favorite.is_none() && r.pinned.is_none() => {
            return Err(invalid(
                "organize requires explicit favorite or pinned state",
            ));
        }
        Delete if r.expected_entry_count.is_none() => {
            return Err(invalid("delete requires expected_entry_count from status"));
        }
        ReadText if r.entry_id.is_none() => {
            return Err(invalid("read_text requires entry_id returned by read"));
        }
        QueuePause if r.paused.is_none() => {
            return Err(invalid("queue_pause requires explicit paused state"));
        }
        Configure if r.reset_model && r.model.is_some() => {
            return Err(invalid("reset_model and model are mutually exclusive"));
        }
        _ => {}
    }
    if r.cursor.is_some() && r.offset != 0 {
        return Err(invalid(
            "cursor and offset cannot be combined; use the returned cursor unchanged",
        ));
    }
    Ok(())
}

// Serialize duplicate card-bearing retries without blocking unrelated Bot work
// or the ordinary native interaction reply path. Weak entries are reclaimed.
type CardLocks = StdMutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>;
fn card_lock(key: String) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: std::sync::OnceLock<CardLocks> = std::sync::OnceLock::new();
    let mut locks = LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(&key).and_then(std::sync::Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    lock
}

impl SessionManager {
    async fn bot_state<T: serde::de::DeserializeOwned>(
        &self,
        key: &str,
    ) -> Result<Option<T>, AppError> {
        self.kernel_runtime
            .instance_state
            .get_payload(key)
            .await
            .map_err(|error| AppError::internal_error(&error))?
            .map(|text| serde_json::from_str(&text).map_err(AppError::from))
            .transpose()
    }
    async fn save_bot_state(&self, key: &str, value: &impl Serialize) -> Result<(), AppError> {
        self.kernel_runtime
            .instance_state
            .set_payload(key, serde_json::to_string(value)?)
            .await
            .map_err(|error| AppError::internal_error(&error))
    }

    fn require_project_path(path: &std::path::Path) -> Result<(), AppError> {
        let private = agena_storage_layout::StorageLayout::current()
            .root()
            .join("channel-scopes");
        if path.starts_with(private) {
            return Err(invalid(
                "isolated channel workspaces are not operator project targets",
            ));
        }
        Ok(())
    }

    async fn bot_workspace(&self, workspace: i64) -> Result<std::path::PathBuf, AppError> {
        let path = self.workspace_root_by_id(workspace).await?;
        Self::require_project_path(&path)?;
        Ok(path)
    }

    async fn require_bot_root(&self, id: i64) -> Result<(), AppError> {
        let config = self
            .kernel()
            .reader
            .configuration(session_id(id)?)
            .await
            .map_err(kernel_error)?;
        if config.parent_session_id.is_some()
            || config.relation_kind != agena_domain::SessionRelationKind::Root
        {
            return Err(invalid(
                "the operator Bot must remain its server-owned root conversation",
            ));
        }
        let ancestry = self
            .kernel()
            .reader
            .management_ancestry(session_id(id)?)
            .await
            .map_err(kernel_error)?;
        if ancestry.len() != 1
            || ancestry
                .iter()
                .any(|ancestor| ancestor.workspace_id.is_some() || ancestor.excluded_scope)
        {
            return Err(invalid(
                "the operator Bot must be an isolated global root conversation",
            ));
        }
        Ok(())
    }

    /// Ordinary children/forks/rewinds remain human-continuable sessions.
    /// Check ancestry too: a fork must not escape a channel or Subagent boundary.
    async fn require_project_session(&self, id: i64) -> Result<(), AppError> {
        let ancestry = self
            .kernel()
            .reader
            .management_ancestry(session_id(id)?)
            .await
            .map_err(kernel_error)?;
        for ancestor in ancestry.iter() {
            self.bot_workspace(ancestor.workspace_id.ok_or_else(|| {
                invalid(
                    "select a real project session; global conversations are not project targets",
                )
            })?)
            .await?;
            if let Some(path) = &ancestor.effective_workspace_root {
                Self::require_project_path(path)?;
            }
            if ancestor.excluded_scope {
                return Err(invalid(
                    "Bot routing excludes inherited Subagent, side and isolated channel conversations",
                ));
            }
        }
        Ok(())
    }

    async fn require_operator_bot(&self, source: i64) -> Result<(), AppError> {
        if self.bot_profile_for_session(source).await?.is_none() {
            return Err(invalid(
                "cross-project session management requires an active registered Bot",
            ));
        }
        self.require_bot_root(source).await
    }

    pub async fn bot_context(&self, source: i64) -> Result<Value, AppError> {
        let Some(profile) = self.bot_profile_for_session(source).await? else {
            return Ok(json!({"enabled":false}));
        };
        self.require_bot_root(source).await?;
        Ok(
            json!({"enabled":true,"session_id":source,"role":"instance_operator_bot","bot_id":profile.id,"id":profile.id,"name":profile.name,"revision":profile.revision}),
        )
    }

    async fn bot_target(&self, source: i64, target: Option<i64>) -> Result<i64, AppError> {
        self.require_operator_bot(source).await?;
        let target = target.ok_or_else(|| invalid("session_id is required"))?;
        self.require_project_session(target).await?;
        let excluded = self.bot_session_ids().await?;
        let ancestry = self
            .kernel()
            .reader
            .management_ancestry(session_id(target)?)
            .await
            .map_err(kernel_error)?;
        if ancestry.iter().any(|a| excluded.contains(&a.session.get())) {
            return Err(invalid(
                "Bot cannot target any Bot conversation, archived history, or descendants",
            ));
        }
        Ok(target)
    }

    pub async fn bot_operation(
        &self,
        source: i64,
        call: i64,
        request: BotRequest,
    ) -> Result<Value, AppError> {
        let action = serde_json::to_value(request.action)?
            .as_str()
            .unwrap_or("unknown")
            .to_owned();
        let target = request
            .session_id
            .map(|id| format!("session {id} ({})", link(id)))
            .or_else(|| request.workspace_id.map(|id| format!("workspace {id}")))
            .or_else(|| request.path.as_ref().map(|path| format!("project {path}")))
            .unwrap_or_else(|| "registered projects".into());
        self.bot_operation_inner(source, call, request)
            .await
            .map_err(|error| {
                invalid(&format!(
                    "bot.sessions action={action} target={target}: {error}"
                ))
            })
    }

    async fn bot_operation_inner(
        &self,
        source: i64,
        call: i64,
        request: BotRequest,
    ) -> Result<Value, AppError> {
        self.require_operator_bot(source).await?;
        let request_bytes = serde_json::to_vec(&request)?.len();
        // A whole Bot character sheet may legitimately be tens of kilobytes.
        // Bound the tool request well above the persona ceiling, not at a
        // generic 32 KiB JSON budget.
        if request_bytes > MAX_BOT_MANAGEMENT_INPUT_BYTES {
            return Err(invalid(&format!(
                "management input exceeds {MAX_BOT_MANAGEMENT_INPUT_BYTES} bytes; send a bounded instruction"
            )));
        }
        validate_request(&request)?;
        // Even reads require a real, live callback; supplied integers confer no authority.
        self.live_tool(source, call)?;
        let _gate = self.bot_operations.lock().await;
        // A queued management call can have been cancelled while another call
        // was waiting for a native permission decision.
        self.live_tool(source, call)?;
        self.require_operator_bot(source).await?;
        let mutation = is_write(request.action);
        let authority = if mutation {
            Some(
                self.require_current_user_quote(
                    source,
                    call,
                    request.user_quote.as_deref().unwrap_or(""),
                )
                .await?,
            )
        } else {
            None
        };
        let key = if let Some(message) = authority {
            let key = required_text(&request.request_key, "request_key", 128)?;
            Some(format!(
                "bot_op_{}",
                request_key(&format!("bot:{source}:{message}"), key)
            ))
        } else {
            None
        };
        let mut lifecycle = None;
        if let Some(key) = &key {
            if let Some(intent) = self.bot_state::<Intent>(key).await? {
                if intent.request != request {
                    return Err(invalid(
                        "retry key already belongs to a different operation; reuse the original payload",
                    ));
                }
                if let Some(result) = intent.result {
                    return Ok(result);
                }
                lifecycle = intent.lifecycle;
            } else {
                // Persist an intent only after exact target and isolation checks.
                // A retry can then replay a native receipt even if the target was deleted.
                if !matches!(
                    request.action,
                    BotAction::Create | BotAction::RegisterProject
                ) {
                    self.bot_target(source, request.session_id).await?;
                }
                lifecycle = self.bot_prepare_lifecycle(&request).await?;
                self.save_bot_state(
                    key,
                    &Intent {
                        request: request.clone(),
                        result: None,
                        lifecycle: lifecycle.clone(),
                    },
                )
                .await?;
            }
        }
        // Human cards are delivered through the ordinary Bot interaction path.
        // Never retain global management serialization while awaiting their reply.
        let _gate = if matches!(
            request.action,
            BotAction::Delete | BotAction::RegisterProject
        ) {
            drop(_gate);
            None
        } else {
            Some(_gate)
        };
        let _card_gate = if matches!(
            request.action,
            BotAction::Delete | BotAction::RegisterProject
        ) {
            let lock = card_lock(format!(
                "{}:{}",
                self.kernel_runtime.instance_id,
                key.as_deref().expect("write key")
            ));
            let gate = lock.lock_owned().await;
            self.live_tool(source, call)?;
            if let Some(intent) = self
                .bot_state::<Intent>(key.as_deref().expect("write key"))
                .await?
            {
                if let Some(result) = intent.result {
                    return Ok(result);
                }
                lifecycle = intent.lifecycle;
            }
            Some(gate)
        } else {
            None
        };
        let result = self
            .bot_effect(
                source,
                call,
                authority,
                key.as_deref(),
                &request,
                lifecycle.as_ref(),
            )
            .await?;
        if serde_json::to_vec(&result)?.len() > RESPONSE_BUDGET {
            return Err(invalid("response exceeds budget; request a smaller page"));
        }
        if let Some(key) = &key {
            self.save_bot_state(
                key,
                &Intent {
                    request,
                    result: Some(result.clone()),
                    lifecycle,
                },
            )
            .await?;
        }
        Ok(result)
    }

    #[allow(clippy::too_many_arguments)]
    async fn bot_effect(
        &self,
        source: i64,
        call: i64,
        origin: Option<i64>,
        key: Option<&str>,
        r: &BotRequest,
        lifecycle: Option<&manage::LifecyclePreparation>,
    ) -> Result<Value, AppError> {
        self.require_operator_bot(source).await?;
        let command = request_key("bot-effect", key.unwrap_or("read"));
        let result = match r.action {
            BotAction::Projects
            | BotAction::Project
            | BotAction::Sessions
            | BotAction::Read
            | BotAction::ReadText
            | BotAction::Status => self.bot_discovery(source, r).await,
            BotAction::Models
            | BotAction::Configure
            | BotAction::Queue
            | BotAction::QueuePause
            | BotAction::QueueEdit
            | BotAction::QueueRemove => {
                self.bot_control(source, call, r, command, origin, key)
                    .await
            }
            BotAction::RegisterProject
            | BotAction::Create
            | BotAction::Rename
            | BotAction::Organize
            | BotAction::Fork
            | BotAction::Delete
            | BotAction::Export => {
                self.bot_manage(source, call, r, command, origin, key, lifecycle)
                    .await
            }
            BotAction::Send => {
                self.submit_bot_instruction(
                    r.session_id.expect("validated target"),
                    source,
                    origin.expect("write authority"),
                    key.expect("write key"),
                    required_text(&r.instruction, "instruction", 16 * 1024)?,
                )
                .await
            }
            BotAction::Stop => {
                let target = r.session_id.expect("validated target");
                let execution = required_text(&r.execution_id, "execution_id", 128)?
                    .parse()
                    .map_err(|_| invalid("invalid execution_id"))?;
                let generation =
                    AttemptGeneration::try_from(r.expected_generation.ok_or_else(|| {
                        invalid("stop requires the observed expected_generation")
                    })?)
                    .map_err(|_| invalid("invalid generation"))?;
                let outcome = self
                    .cancel_execution_attempt_with_outcome(target, execution, generation)
                    .await?;
                Ok(json!({"session_id":target,"outcome":outcome,"url":link(target)}))
            }
        }?;
        Ok(if is_write(r.action) {
            let mut result = result;
            if let Some(object) = result.as_object_mut() {
                object.insert(
                    "origin".into(),
                    json!({"kind":"operator_bot","session_id":source,"user_message_id":origin}),
                );
            }
            result
        } else {
            result
        })
    }

    async fn submit_bot_instruction(
        &self,
        target: i64,
        source: i64,
        origin: i64,
        key: &str,
        text: &str,
    ) -> Result<Value, AppError> {
        let session = session_id(target)?;
        let dispatch = self.session_dispatch(session);
        let _submission = dispatch.submission.lock().await;
        let payload = json!({"kind":"bot_instruction","source_session_id":source,"source_message_id":origin,"text":text});
        let notice = format!(
            "Operator Bot instruction (Bot session {source}, user message {origin}). Execute this bounded project request here using the existing tools and Subagents. The user can directly continue this session. Preserve this project's permissions; ask the real user through native permission cards when needed.\n\n{text}"
        );
        let command = request_key("bot-instruction", key);
        let execution = persist(|| {
            self.kernel().executor.deliver_control_notice(
                command,
                session,
                key.into(),
                payload.clone(),
                notice.clone(),
            )
        })
        .await
        .map_err(kernel_error)?
        .value;
        let mut result = json!({"session_id":target,"execution_id":execution.id,"generation":execution.generation.get(),"phase":execution.phase,"accepted":true,"origin":{"kind":"operator_bot","session_id":source,"user_message_id":origin},"url":link(target)});
        // Acknowledgment always states that native delivery committed. Failure
        // to start does not imply the accepted queued instruction disappeared.
        match self.kernel().reader.execution(session, execution.id).await {
            Ok(current) => {
                result["phase"] = serde_json::to_value(current.phase)?;
                if current.phase == ExecutionPhase::Queued {
                    let start = async {
                        let snapshot = self
                            .kernel()
                            .reader
                            .control_snapshot(session, 1)
                            .await
                            .map_err(kernel_error)?;
                        if snapshot.configuration.message_queue_paused {
                            return Ok("paused");
                        }
                        drop(snapshot);
                        let metadata = self
                            .kernel()
                            .reader
                            .metadata(session)
                            .await
                            .map_err(kernel_error)?;
                        let mut options = self
                            .options_for_session(
                                session,
                                &metadata.selection.clone().unwrap_or_default(),
                            )
                            .await?;
                        self.prepare_options(session, &mut options).await?;
                        if metadata.selection.is_none() {
                            self.update_session_selection(target, options.clone())
                                .await?;
                        }
                        self.restore_queued_executions(session).await?;
                        Ok::<_, AppError>("scheduled")
                    }
                    .await;
                    match start {
                        Err(error) => {
                            result["dispatch"] = json!("blocked");
                            result["reason"] = json!(format!(
                                "send to session {target} was accepted into its native queue but could not start: {error}. Inspect models/status and resolve the blocker; do not resend with a new request key."
                            ));
                        }
                        Ok(dispatch) => {
                            result["dispatch"] = json!(dispatch);
                        }
                    }
                }
            }
            Err(KernelError::NotFound(_) | KernelError::Ownership) => {
                result["target_deleted"] = json!(matches!(
                    self.kernel().reader.metadata(session).await,
                    Err(KernelError::NotFound(_))
                ));
                result["dispatch"] = json!("no_longer_active");
            }
            Err(error) => {
                result["dispatch"] = json!("accepted_state_unavailable");
                result["reason"] = json!(format!(
                    "accepted native send; inspect status to recover current state: {error}"
                ));
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
