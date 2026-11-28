//! Cordis tree membership is derived from durable, same-workspace subagent edges.

use agena_domain::{PermissionDecision, StructuredObject, ToolInvocation};
use agena_plugin_host::{
    host::ImportedToolDecision,
    sdk::{
        PluginKey,
        host_api::{
            AgentMessageRequest, AgentTargetRequest, AgentWaitRequest, AgentWakeReason,
            CordisSubagentControl,
        },
    },
};
use serde_json::{Value, json};

use super::{messaging, *};
use crate::session::manager::execution::PermissionInvocation;

const ROOT_NAME: &str = "lead";
const MAX_DEPTH: usize = 128;
const MAX_CATALOG_ENTRIES: usize = 1024;
const DEFAULT_CORDIS_MAX_ACTIVE_CHILDREN: u64 = 8;

struct Member {
    id: SessionId,
    name: Option<String>,
    reserved_id: Option<String>,
    running: bool,
}

impl Member {
    fn visible_id(&self) -> String {
        self.reserved_id
            .clone()
            .unwrap_or_else(|| self.id.get().to_string())
    }
}

pub(super) struct Lineage(Vec<Member>);

impl Lineage {
    fn root(&self) -> &Member {
        self.0.first().expect("lineage includes its root")
    }

    fn caller(&self) -> &Member {
        self.0.last().expect("lineage includes its caller")
    }

    pub(super) fn root_id(&self) -> SessionId {
        self.root().id
    }

    pub(super) fn ancestor_ids(&self) -> Vec<i64> {
        self.0.iter().map(|member| member.id.get()).collect()
    }

    fn depth(&self) -> usize {
        self.0.len() - 1
    }

    pub(super) fn admit_child(&self, max_depth: Option<u64>) -> Result<(), AppError> {
        let depth = self.depth() + 1;
        if depth > MAX_DEPTH || max_depth.is_some_and(|max| depth as u64 > max) {
            return Err(AppError::Config(format!(
                "Cordis child at depth {depth} exceeds maxDepth"
            )));
        }
        Ok(())
    }

    pub(super) fn child_name(&self, name: &str, child: SessionId) -> String {
        if self.depth() == 0 {
            return name.to_owned();
        }
        member_path(&self.0).map_or_else(
            || child.get().to_string(),
            |parent| format!("{parent}/{name}"),
        )
    }
}

fn member_path(members: &[Member]) -> Option<String> {
    if members.len() == 1 {
        return Some(ROOT_NAME.into());
    }
    let mut segments = Vec::with_capacity(members.len() - 1);
    for member in &members[1..] {
        let name = member.name.as_deref().filter(|name| *name != ROOT_NAME)?;
        segments.push(name);
    }
    Some(segments.join("/"))
}

fn member_name(members: &[Member]) -> String {
    member_path(members).unwrap_or_else(|| {
        members
            .last()
            .expect("non-empty ancestry")
            .id
            .get()
            .to_string()
    })
}

fn activity(running: bool) -> &'static str {
    if running { "running" } else { "inactive" }
}

fn outside_tree() -> AppError {
    AppError::Config("The target is outside this Cordis agent tree".into())
}

impl SessionManager {
    pub async fn cordis_subagent_control(
        &self,
        session: i64,
        call: Option<i64>,
        plugin: PluginKey,
        request: CordisSubagentControl,
        cancel: CancellationToken,
    ) -> Result<Value, AppError> {
        let source = session_id(session)?;
        let control = call
            .map(|call| self.live_tool(session, call).map(|(_, control)| control))
            .transpose()?;
        let state = match &control {
            Some(control) => control.state.clone(),
            None => self.state_for_session(source).await?,
        };
        let context = self
            .execution_context_for_execution(
                source,
                &state,
                control.as_ref().map(|control| control.execution),
            )
            .await?;
        let executor = state.executor.for_session_resource_context(&context);
        if context.global
            || context.owner.is_some()
            || context.channel_authority.is_some()
            || !executor.plugin_is_enabled(&plugin)
        {
            return Err(AppError::Config(
                "Cordis agent coordination requires an enabled plugin in a project session".into(),
            ));
        }
        let lineage = self.cordis_lineage(source).await?;
        if cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        match request {
            CordisSubagentControl::Scope => {
                let root = lineage.root();
                let active_children = self.cordis_active_agent_count(root.id).await?;
                let parent =
                    (lineage.depth() > 0).then(|| member_name(&lineage.0[..lineage.depth()]));
                Ok(json!({
                    "rootId": root.visible_id(), "rootStatus": activity(root.running),
                    "selfId": lineage.caller().visible_id(),
                    "self": member_name(&lineage.0), "parent": parent, "depth": lineage.depth(),
                    "status": activity(lineage.caller().running),
                    "activeChildren": active_children,
                }))
            }
            CordisSubagentControl::List { parent_id } => {
                let parent = self
                    .cordis_scoped_target(&lineage, &parent_id)
                    .await?
                    .caller()
                    .id;
                self.cordis_list_agents(parent, &cancel).await
            }
            CordisSubagentControl::Resolve { name } => {
                let id = self
                    .cordis_resolve_name(lineage.root().id, &name, &cancel)
                    .await?;
                Ok(json!(id))
            }
            CordisSubagentControl::ModelRoutes => {
                let current_model = match control {
                    Some(control) => control.model.clone(),
                    None => {
                        self.options_for_session(source, &context.selection)
                            .await?
                            .model
                    }
                };
                let describe = |model: &ModelRef| -> Result<Value, AppError> {
                    let metadata = state
                        .providers
                        .model_metadata(model)
                        .map_err(AppError::from)?;
                    let efforts = state
                        .providers
                        .model_thinking_modes(model)
                        .map_err(AppError::from)?;
                    let efforts = efforts
                        .iter()
                        .filter_map(|mode| {
                            let id = mode.selector()?;
                            Some(json!({
                                "id": id,
                                "name": mode.display_name.as_deref().unwrap_or(&id),
                                "description": mode.description,
                                "default": mode.is_default,
                            }))
                        })
                        .collect::<Vec<_>>();
                    Ok(json!({
                        "provider": model.provider_id,
                        "model": model.model_id,
                        "description": metadata.description,
                        "efforts": efforts,
                    }))
                };
                let current = describe(&current_model)?;
                let mut seen = std::collections::BTreeSet::new();
                let mut routes = Vec::new();
                seen.insert((
                    current_model.provider_id.to_string(),
                    current_model.model_id.to_string(),
                ));
                routes.push(current.clone());
                for candidate in state.providers.selection_model_inventory() {
                    let candidate_ref = candidate.reference();
                    let key = (
                        candidate_ref.provider_id.to_string(),
                        candidate_ref.model_id.to_string(),
                    );
                    if seen.contains(&key) {
                        continue;
                    }
                    let Ok(resolved) = state.providers.resolve_model_selection(
                        candidate_ref.provider_id.as_ref(),
                        None,
                        Some(candidate_ref.model_id.as_ref()),
                    ) else {
                        continue;
                    };
                    if let Ok(entry) = describe(&resolved) {
                        seen.insert(key);
                        routes.push(entry);
                    }
                }
                routes.sort_by(|left, right| {
                    (left["provider"].as_str(), left["model"].as_str())
                        .cmp(&(right["provider"].as_str(), right["model"].as_str()))
                });
                Ok(json!({
                    "provider": current["provider"],
                    "model": current["model"],
                    "description": current["description"],
                    "efforts": current["efforts"],
                    "routes": routes,
                }))
            }
            CordisSubagentControl::Message {
                target_id,
                text,
                attachments,
            } => {
                self.cordis_scoped_target(&lineage, &target_id).await?;
                let has_attachments = !attachments.is_empty();
                let effective_text = if text.trim().is_empty() && has_attachments {
                    "[attachment]".to_owned()
                } else {
                    text
                };
                let input = json!({"target":target_id,"message":effective_text});
                let request: AgentMessageRequest = serde_json::from_value(
                    self.prepare_cordis_control(session, call, "message", input, &cancel)
                        .await?,
                )?;
                self.cordis_send_message(&lineage, request, attachments, &cancel)
                    .await
            }
            CordisSubagentControl::Interrupt { target_id } => {
                self.cordis_interrupt_target(&lineage, &target_id).await?;
                let request: AgentTargetRequest = serde_json::from_value(
                    self.prepare_cordis_control(
                        session,
                        call,
                        "cancel",
                        json!({"target":target_id}),
                        &cancel,
                    )
                    .await?,
                )?;
                let target = self
                    .cordis_interrupt_target(&lineage, &request.target)
                    .await?;
                self.cordis_interrupt(target, &cancel).await?;
                Ok(Value::Null)
            }
            CordisSubagentControl::Wait { timeout_ms } => {
                let control = control.ok_or_else(|| {
                    AppError::Config("wait_agent requires a live Cordis tool call".into())
                })?;
                let timeout = timeout_ms.clamp(1, 300_000);
                // Native validation accepts up to five minutes. Cordis permits
                // a longer wait, with the same policy check on its native tool.
                let request: AgentWaitRequest = serde_json::from_value(
                    self.prepare_cordis_control(
                        session,
                        call,
                        "wait",
                        json!({"timeout_ms":timeout}),
                        &cancel,
                    )
                    .await?,
                )?;
                let effective_timeout = if request.timeout_ms == Some(timeout) {
                    timeout_ms
                } else {
                    request.timeout_ms.unwrap_or(30_000)
                };
                if !(10_000..=3_600_000).contains(&effective_timeout) {
                    return Err(AppError::Config(
                        "timeout_ms must be an integer from 10000 through 3600000".into(),
                    ));
                }
                let outcome = self.with_agent_wait_permit(session, call, Some(&control), async {
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => Err(AppError::Cancelled),
                        result = self.wait_agent_mailbox(source, Some(&control), Duration::from_millis(effective_timeout)) => result,
                    }
                }).await?;
                Ok(json!({"timedOut": matches!(outcome, AgentWakeReason::Timeout)}))
            }
            CordisSubagentControl::WaitForTreeWork => {
                if lineage.depth() != 0 {
                    return Ok(Value::Bool(false));
                }
                let joined = self
                    .with_agent_wait_permit(
                        session,
                        call,
                        control.as_deref(),
                        self.wait_for_cordis_tree_work(source, &cancel),
                    )
                    .await?;
                Ok(Value::Bool(joined))
            }
        }
    }

    async fn wait_for_cordis_tree_work(
        &self,
        root: SessionId,
        cancel: &CancellationToken,
    ) -> Result<bool, AppError> {
        let wait = async {
            let mut joined = false;
            loop {
                // Storage also sees queued attempts started by the inbox,
                // which have no direct Cordis activation handle.
                let durable = self.cordis_active_agent_count(root).await?;
                if durable == 0 && !self.cordis_tree_work.lock().contains_key(&root) {
                    return Ok(joined);
                }
                joined = true;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        };
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(AppError::Cancelled),
            _ = self.shutdown.cancelled() => Err(AppError::Cancelled),
            result = wait => result,
        }
    }

    pub(super) async fn cordis_active_agent_count(&self, root: SessionId) -> Result<u64, AppError> {
        Ok(*read_with_busy_retry(root, "cordis_agent_capacity", || {
            self.kernel().reader.active_agent_count(root)
        })
        .await?)
    }

    pub(in crate::session::manager) async fn cordis_agent_wake_allowed(
        &self,
        session: SessionId,
        configuration: &SessionConfiguration,
    ) -> Result<bool, AppError> {
        if !configuration.relation_kind.is_subagent() {
            return Ok(true);
        }
        let Some(cordis) = configuration
            .agent
            .as_ref()
            .and_then(|agent| agent.cordis.as_ref())
        else {
            return Ok(true);
        };
        let limit = cordis
            .max_active_children
            .unwrap_or(DEFAULT_CORDIS_MAX_ACTIVE_CHILDREN);
        let root = self.cordis_lineage(session).await?.root_id();
        Ok(self.cordis_active_agent_count(root).await? < limit)
    }

    pub(super) async fn wake_deferred_cordis_agents(&self, root: SessionId) {
        if self.shutdown.is_cancelled() {
            return;
        }
        let Ok(page) = read_with_busy_retry(root, "cordis_deferred_catalog", || {
            self.kernel().reader.agent_catalog(root, MAX_CATALOG_ENTRIES)
        })
        .await
        else {
            return;
        };
        let mut waiting = Vec::new();
        for member in page.iter().filter(|member| !member.running) {
            let Ok(pending) = read_with_busy_retry(member.id, "cordis_deferred_inbox", || {
                self.kernel().reader.pending_deliveries(member.id, None, 64)
            })
            .await
            else {
                continue;
            };
            if let Some(earliest) = pending
                .iter()
                .filter(|delivery| delivery.target_execution.is_none() && delivery.wakes_agent())
                .map(|delivery| delivery.id)
                .min()
            {
                waiting.push((earliest, member.id));
            }
        }
        waiting.sort_by_key(|(delivery_id, session_id)| (*delivery_id, *session_id));
        for (_, child) in waiting {
            if let Err(error) = self.wake_agent_inbox(child).await {
                tracing::debug!(
                    session_id = child.get(),
                    %error,
                    "deferred Cordis agent wake skipped"
                );
            }
        }
    }

    pub(super) async fn cordis_lineage(&self, session: SessionId) -> Result<Lineage, AppError> {
        let mut cursor = session;
        let mut workspace = None;
        let mut nodes = Vec::new();
        for _ in 0..=MAX_DEPTH {
            let summary = read_with_busy_retry(cursor, "cordis_agent_summary", || {
                self.kernel().reader.summary(cursor)
            })
            .await?;
            let config = read_with_busy_retry(cursor, "cordis_agent_membership", || {
                self.kernel().reader.configuration(cursor)
            })
            .await?;
            let Some(owning_workspace) = summary.workspace_id else {
                return Err(outside_tree());
            };
            if workspace.get_or_insert(owning_workspace) != &owning_workspace
                || config.owner.is_some()
                || config.channel_authority.is_some()
            {
                return Err(outside_tree());
            }
            let parent = if config.relation_kind.is_subagent() {
                let agent = config
                    .agent
                    .as_ref()
                    .filter(|agent| agent.workflow.is_none())
                    .ok_or_else(outside_tree)?;
                nodes.push(Member {
                    id: cursor,
                    name: Some(agent.name.clone()),
                    reserved_id: agent.cordis.as_ref().and_then(|c| c.reserved_id.clone()),
                    running: summary.running_executions > 0,
                });
                config.parent_session_id.ok_or_else(outside_tree)?
            } else {
                nodes.push(Member {
                    id: cursor,
                    name: None,
                    reserved_id: None,
                    running: summary.running_executions > 0,
                });
                nodes.reverse();
                return Ok(Lineage(nodes));
            };
            // A creator existed before its child. This also fails closed on a
            // damaged cycle without reading the same sessions repeatedly.
            if parent >= cursor {
                return Err(outside_tree());
            }
            cursor = parent;
        }
        Err(AppError::Config(
            "Cordis agent ancestry exceeds its supported depth".into(),
        ))
    }

    async fn cordis_scoped_target(
        &self,
        source: &Lineage,
        address: &str,
    ) -> Result<Lineage, AppError> {
        let trimmed = address.trim();
        if let Some(target) = trimmed
            .parse::<i64>()
            .ok()
            .and_then(|value| session_id(value).ok())
            && let Ok(lineage) = self.cordis_lineage(target).await
            && lineage.root().id == source.root().id
        {
            return Ok(lineage);
        }
        let catalog = read_with_busy_retry(source.root().id, "cordis_reserved_target", || {
            self.kernel()
                .reader
                .agent_catalog(source.root().id, MAX_CATALOG_ENTRIES + 1)
        })
        .await?;
        let Some(member) = catalog
            .iter()
            .find(|member| member.reserved_id.as_deref() == Some(trimmed))
        else {
            return Err(outside_tree());
        };
        let target = self
            .cordis_lineage(member.id)
            .await
            .map_err(|_| outside_tree())?;
        if target.root().id != source.root().id {
            return Err(outside_tree());
        }
        Ok(target)
    }

    async fn cordis_interrupt_target(
        &self,
        source: &Lineage,
        address: &str,
    ) -> Result<SessionId, AppError> {
        let lineage = self.cordis_scoped_target(source, address).await?;
        let target = lineage.caller().id;
        if target == source.caller().id
            || !lineage
                .0
                .iter()
                .any(|member| member.id == source.caller().id)
        {
            return Err(AppError::Config(
                "Only an ancestor can interrupt this Cordis agent".into(),
            ));
        }
        Ok(target)
    }

    async fn cordis_resolve_name(
        &self,
        root: SessionId,
        name: &str,
        cancel: &CancellationToken,
    ) -> Result<String, AppError> {
        if name == ROOT_NAME {
            return Ok(root.get().to_string());
        }
        if name.is_empty() || name.len() > 4096 || name.split('/').count() > MAX_DEPTH {
            return Err(AppError::Config("Invalid Cordis agent name".into()));
        }
        let mut parent = root;
        for component in name.split('/') {
            if cancel.is_cancelled() {
                return Err(AppError::Cancelled);
            }
            let child = if component.is_empty() {
                None
            } else {
                *read_with_busy_retry(parent, "cordis_agent_name", || {
                    self.kernel().reader.find_agent(parent, component)
                })
                .await?
            };
            if let Some(child) = child {
                let config = read_with_busy_retry(child, "cordis_agent_name", || {
                    self.kernel().reader.configuration(child)
                })
                .await?;
                if config.owner.is_none()
                    && config.channel_authority.is_none()
                    && config
                        .agent
                        .as_ref()
                        .is_some_and(|agent| agent.workflow.is_none())
                {
                    parent = child;
                    continue;
                }
            }
            if name.contains('/') {
                return Err(AppError::Config(format!(
                    "Cordis agent path {name:?} was not found"
                )));
            }
            return Ok(name.to_owned());
        }
        let config = read_with_busy_retry(parent, "cordis_agent_visible_id", || {
            self.kernel().reader.configuration(parent)
        })
        .await?;
        let visible = config
            .agent
            .as_ref()
            .and_then(|agent| agent.cordis.as_ref())
            .and_then(|cordis| cordis.reserved_id.clone())
            .unwrap_or_else(|| parent.get().to_string());
        Ok(visible)
    }

    async fn cordis_list_agents(
        &self,
        parent: SessionId,
        cancel: &CancellationToken,
    ) -> Result<Value, AppError> {
        let page = read_with_busy_retry(parent, "cordis_agent_catalog", || {
            self.kernel()
                .reader
                .agent_catalog(parent, MAX_CATALOG_ENTRIES + 1)
        })
        .await?;
        if cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        if page.len() > MAX_CATALOG_ENTRIES {
            return Err(AppError::Config(
                "Cordis agent catalog exceeds 1024 children".into(),
            ));
        }
        let parent_config = read_with_busy_retry(parent, "cordis_parent_visible_id", || {
            self.kernel().reader.configuration(parent)
        })
        .await?;
        let mut visible_ids = std::collections::BTreeMap::new();
        visible_ids.insert(
            parent,
            parent_config
                .agent
                .as_ref()
                .and_then(|agent| agent.cordis.as_ref())
                .and_then(|cordis| cordis.reserved_id.clone())
                .unwrap_or_else(|| parent.get().to_string()),
        );
        for child in page.iter() {
            visible_ids.insert(
                child.id,
                child
                    .reserved_id
                    .clone()
                    .unwrap_or_else(|| child.id.get().to_string()),
            );
        }
        let parents = page
            .iter()
            .map(|child| child.parent)
            .collect::<std::collections::BTreeSet<_>>();
        let mut paths = std::collections::BTreeMap::new();
        paths.insert(parent, Some(String::new()));
        let mut entries = Vec::with_capacity(page.len());
        for child in page.iter() {
            let path = paths
                .get(&child.parent)
                .and_then(Option::as_ref)
                .filter(|_| child.name != ROOT_NAME)
                .map(|prefix| {
                    if prefix.is_empty() {
                        child.name.clone()
                    } else {
                        format!("{prefix}/{}", child.name)
                    }
                });
            let id = visible_ids
                .get(&child.id)
                .cloned()
                .unwrap_or_else(|| child.id.get().to_string());
            let parent_visible_id = visible_ids
                .get(&child.parent)
                .cloned()
                .unwrap_or_else(|| child.parent.get().to_string());
            let mut entry = json!({
                "kind": "child", "mode": "continuable", "id": id,
                "parentId": parent_visible_id, "depth": child.depth, "name": child.name,
                "label": child.label, "activity": activity(child.running),
                "hasChildren": parents.contains(&child.id),
            });
            if let Some(path) = &path {
                entry["path"] = json!(path);
            }
            paths.insert(child.id, path);
            entries.push(entry);
        }
        Ok(Value::Array(entries))
    }

    async fn cordis_send_message(
        &self,
        source: &Lineage,
        request: AgentMessageRequest,
        attachments: Vec<agena_domain::AttachmentItem>,
        cancel: &CancellationToken,
    ) -> Result<Value, AppError> {
        let caller = source.caller().id;
        let target = self
            .cordis_scoped_target(source, &request.target)
            .await?
            .caller()
            .id;
        if target == caller {
            return Err(AppError::Config(
                "Send Cordis messages to another member of the tree".into(),
            ));
        }
        let mut message = request.message;
        let mut inline_attachments = Vec::new();
        if !attachments.is_empty() {
            let mut summary_lines = Vec::new();
            let mut inline_bytes = 0usize;
            for item in attachments {
                let encoded_len = serde_json::to_vec(&item)
                    .map(|bytes| bytes.len())
                    .unwrap_or(usize::MAX);
                let name = item.filename.as_deref().unwrap_or("attachment");
                let size = item.size_bytes.unwrap_or(0);
                summary_lines.push(format!(
                    "[Attachment: {name} ({}, {size} bytes{})]",
                    item.mime,
                    item.width
                        .zip(item.height)
                        .map(|(w, h)| format!(", {w}x{h}"))
                        .unwrap_or_default()
                ));
                if inline_bytes.saturating_add(encoded_len) <= 44 * 1024 {
                    inline_bytes = inline_bytes.saturating_add(encoded_len);
                    inline_attachments.push(item);
                } else {
                    let mut degraded = item;
                    degraded.source = agena_domain::AttachmentSource::Unavailable {
                        reason: "attachment exceeds inline inter-agent message budget".into(),
                    };
                    inline_attachments.push(degraded);
                }
            }
            if !summary_lines.is_empty() {
                if !message.is_empty() {
                    message.push('\n');
                }
                message.push_str(&summary_lines.join("\n"));
            }
        }
        let mut payload =
            messaging::agent_message(caller, &member_name(&source.0), &message, true)?;
        if !inline_attachments.is_empty() {
            payload["attachments"] = serde_json::to_value(&inline_attachments)?;
            payload = bounded_agent_payload(payload)?;
        }
        if cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        let command = RequestId::new();
        let receipt = persist(|| {
            self.kernel().executor.deliver(
                command,
                target,
                format!("cordis-agent-message:{command}"),
                payload.clone(),
            )
        })
        .await
        .map_err(kernel_error)?;
        if let Err(error) = self.wake_agent_inbox(target).await {
            tracing::debug!(session_id = target.get(), %error, "Cordis agent inbox wake deferred");
        }
        Ok(json!(receipt.value.id.to_string()))
    }

    async fn cordis_interrupt(
        &self,
        target: SessionId,
        cancel: &CancellationToken,
    ) -> Result<(), AppError> {
        let Some(execution) = self.running_execution(target).await? else {
            return Ok(());
        };
        let view = read_with_busy_retry(target, "cordis_interrupt_attempt", || {
            self.kernel().reader.execution(target, execution)
        })
        .await?;
        if cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        if view.phase == ExecutionPhase::Running {
            self.cancel_execution_attempt_with_outcome(target.get(), execution, view.generation)
                .await?;
        }
        Ok(())
    }

    async fn prepare_cordis_control(
        &self,
        session: i64,
        call: Option<i64>,
        operation: &'static str,
        input: Value,
        cancel: &CancellationToken,
    ) -> Result<Value, AppError> {
        let (executor, context) = self.host_callback_executor(Some(session), call).await?;
        let mut executor = executor.with_cancellation_token(Some(cancel.clone()));
        if operation == "cancel"
            && context.is_some_and(|context| context.relation_kind.is_subagent())
        {
            executor = executor.for_cordis_agent_permission("cancel");
        }
        let invocation = ToolInvocation::new(
            format!("agena.agent.{operation}"),
            StructuredObject::try_from(input).map_err(AppError::Config)?,
        );
        let prepared = executor
            .prepare_invocation(&invocation, session, call.unwrap_or(-1))
            .await
            .map_err(tool_error)?;
        self.authorize_cordis_agent_tool(session, call, &executor, &prepared, None)
            .await?;
        Ok(Value::from(prepared.invocation.input))
    }

    pub(super) async fn authorize_cordis_agent_tool(
        &self,
        session: i64,
        call: Option<i64>,
        executor: &ToolExecutor,
        prepared: &agena_tool::PreparedToolInvocation,
        path_check: Option<agena_tool::ToolPermissionCheck>,
    ) -> Result<(), AppError> {
        if let Some(call) = call {
            let (tool, control) = self.live_tool(session, call)?;
            self.authorize_tool(
                &tool,
                PermissionInvocation {
                    tool: &prepared.invocation,
                    run_mod_check: true,
                    hook_ask: prepared.permission_ask.as_deref(),
                },
                executor,
                &control,
                &control.state,
            )
            .await
            .map_err(tool_error)?;
            if let Some(check) = path_check {
                self.authorize_checks(&tool, vec![check], executor, &control, &control.state)
                    .await
                    .map_err(tool_error)?;
            }
        } else {
            if let Some(reason) = &prepared.permission_ask {
                return Err(AppError::Config(format!(
                    "Cordis agent coordination requires interactive approval: {reason}"
                )));
            }
            let mut checks = executor
                .collect_permission_checks_for_invocation_in_session(
                    &prepared.invocation,
                    Some(session),
                )
                .await
                .map_err(tool_error)?;
            checks.extend(path_check);
            self.authorize_session_scoped_checks(session, executor, checks)
                .await?;
            if let Some(
                ImportedToolDecision::Deny { reason } | ImportedToolDecision::Ask { reason },
            ) = executor
                .run_imported_tool_check(
                    &prepared.invocation,
                    session,
                    -1,
                    PermissionDecision::Allow,
                )
                .await
            {
                return Err(AppError::Config(reason.unwrap_or_else(|| {
                    "A plugin blocked Cordis agent coordination".into()
                })));
            }
        }
        Ok(())
    }
}
