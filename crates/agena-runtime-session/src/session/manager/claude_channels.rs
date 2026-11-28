//! Operator-owned Claude MCP channel connections and approval relay.

use agena_domain::{
    PermissionReply, PermissionReplyKind,
    claude_channels::{ClaudeChannelAction, ClaudeChannelDefinition, ClaudeChannelView},
};
use agena_mcp_client::{
    ClaudeChannelEvent, ClaudeChannelPeer, ClaudeChannelStream, McpConnectionManager,
    McpSandboxContext, ServerSpec,
};
use agena_plugin_host::{
    AgentPluginFormat, PluginPackage,
    agent_plugin::agent_plugin_mcp_server_name,
    host::ImportedPromptScope,
    sdk::{
        PluginKey,
        host_api::{HostProcessResources, ImportedPluginPromptRequest, SESSION_SCOPED_CALL_ID},
    },
};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    path::PathBuf,
    sync::Weak,
};

use super::*;

const MCP_BRIDGE: &str = "agena.mcp";
const MAX_CHANNELS: usize = 8;
const RECHECK_INTERVAL: Duration = Duration::from_secs(5);
const RECONNECT_ATTEMPTS: u32 = 5;
const RECONNECT_DELAY: Duration = Duration::from_millis(500);

#[derive(Clone, Default)]
pub(super) struct ClaudeChannels {
    bindings: Arc<parking_lot::Mutex<BTreeMap<(SessionId, String), Arc<Binding>>>>,
}

struct Binding {
    session: SessionId,
    server: String,
    definition: ChannelDefinition,
    permission_relay: bool,
    cancellation: CancellationToken,
    live: parking_lot::Mutex<LiveChannel>,
}

#[derive(PartialEq, Eq)]
struct ChannelDefinition {
    target: ChannelTarget,
    spec: ServerSpec,
    authority: McpSandboxContext,
}

#[derive(PartialEq, Eq)]
struct ChannelTarget {
    plugin: PluginKey,
    server_id: String,
    package: Option<(PathBuf, Option<String>)>,
}

#[derive(Default)]
struct LiveChannel {
    connection: Option<ChannelConnection>,
    used: BTreeSet<String>,
    pending: BTreeMap<String, InteractionId>,
}

struct ChannelConnection {
    state: Weak<ExecutionState>,
    owner: Arc<McpConnectionManager>,
    manager: Arc<McpConnectionManager>,
    peer: ClaudeChannelPeer,
}

impl ChannelConnection {
    fn is_for_state(&self, state: &Arc<ExecutionState>) -> bool {
        self.state.ptr_eq(&Arc::downgrade(state))
    }
}

enum ChannelEnd {
    Disconnected,
    Stopped,
}

pub(super) struct RelayedPermissions(Vec<(Arc<Binding>, String)>);

impl Drop for RelayedPermissions {
    fn drop(&mut self) {
        for (binding, id) in &self.0 {
            binding.live.lock().pending.remove(id);
        }
    }
}

impl ClaudeChannels {
    pub(super) fn stop_session(&self, session: SessionId) {
        self.bindings.lock().retain(|(id, _), binding| {
            if *id == session {
                binding.cancellation.cancel();
                false
            } else {
                true
            }
        });
    }

    fn session(&self, session: SessionId) -> Vec<Arc<Binding>> {
        self.bindings
            .lock()
            .values()
            .filter(|binding| binding.session == session)
            .cloned()
            .collect()
    }

    fn remove(&self, binding: &Arc<Binding>) {
        let key = (binding.session, binding.server.clone());
        let mut bindings = self.bindings.lock();
        if bindings
            .get(&key)
            .is_some_and(|active| Arc::ptr_eq(active, binding))
        {
            bindings.remove(&key);
        }
        binding.cancellation.cancel();
    }
}

impl Binding {
    fn view(&self, state: &Arc<ExecutionState>) -> ClaudeChannelView {
        ClaudeChannelView {
            server: self.server.clone(),
            permission_relay: self.permission_relay,
            connected: !self.cancellation.is_cancelled()
                && self
                    .live
                    .lock()
                    .connection
                    .as_ref()
                    .is_some_and(|connection| {
                        connection.is_for_state(state) && connection.peer.connected()
                    }),
        }
    }

    fn is_attached_to(&self, state: &Arc<ExecutionState>) -> bool {
        self.live
            .lock()
            .connection
            .as_ref()
            .is_some_and(|connection| connection.is_for_state(state))
    }

    async fn close(&self) {
        let connection = {
            let mut live = self.live.lock();
            live.pending.clear();
            live.connection.take()
        };
        if let Some(connection) = connection {
            connection
                .owner
                .release_sandbox_server(&self.definition.target.server_id, &connection.manager)
                .await;
        }
    }

    async fn connect(
        &self,
        state: &Arc<ExecutionState>,
        owner: Arc<McpConnectionManager>,
    ) -> Result<ClaudeChannelStream, AppError> {
        if self.cancellation.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        let server_id = &self.definition.target.server_id;
        let (manager, stream) = owner
            .for_claude_channel_scope(
                server_id,
                self.definition.authority.clone(),
                &self.definition.spec,
            )
            .await
            .map_err(mcp_error)?;
        let error = if self.cancellation.is_cancelled() {
            Some(AppError::Cancelled)
        } else if self.permission_relay && !stream.peer().supports_permission_relay() {
            Some(AppError::Config(
                "The channel server does not support permission relay".into(),
            ))
        } else {
            None
        };
        if let Some(error) = error {
            owner.release_sandbox_server(server_id, &manager).await;
            return Err(error);
        }
        self.live.lock().connection = Some(ChannelConnection {
            state: Arc::downgrade(state),
            owner,
            manager,
            peer: stream.peer().clone(),
        });
        Ok(stream)
    }

    fn issue_permission(
        &self,
        interaction: InteractionId,
        state: &Arc<ExecutionState>,
    ) -> Option<(String, ClaudeChannelPeer)> {
        let mut live = self.live.lock();
        let connection = live.connection.as_ref()?;
        // Keep used IDs across a reconnect so a delayed vote from an earlier
        // peer can never approve a later interaction.
        if !self.permission_relay
            || live.used.len() >= 4096
            || self.cancellation.is_cancelled()
            || !connection.is_for_state(state)
            || !connection.peer.connected()
        {
            return None;
        }
        let peer = connection.peer.clone();
        let alphabet = b"abcdefghijkmnopqrstuvwxyz";
        loop {
            let random = uuid::Uuid::new_v4();
            let id: String = random.as_bytes()[..5]
                .iter()
                .map(|byte| alphabet[*byte as usize % alphabet.len()] as char)
                .collect();
            if live.used.insert(id.clone()) {
                live.pending.insert(id.clone(), interaction);
                return Some((id, peer));
            }
        }
    }
}

impl SessionManager {
    pub(super) async fn claude_channel_catalog(
        &self,
        id: i64,
    ) -> Result<Vec<ClaudeChannelDefinition>, AppError> {
        let session = session_id(id)?;
        let (_, executor) = self.claude_channel_executor(session).await?;
        let registered = match executor.mcp_manager() {
            Some(owner) => owner
                .server_names()
                .await
                .into_iter()
                .collect::<BTreeSet<_>>(),
            None => BTreeSet::new(),
        };
        let mut definitions = Vec::new();
        for plugin in executor.plugin_manager().plugins() {
            if !executor.plugin_is_enabled(&plugin.key()) {
                continue;
            }
            let Some(package) = plugin.agent_plugin() else {
                continue;
            };
            let PluginPackage::AgentPlugin {
                user_config_env, ..
            } = &plugin.configured_plugin.package
            else {
                continue;
            };
            let id = plugin.key().to_string();
            for server in &package.mcp_servers {
                definitions.extend(server.claude_channel_definition(
                    &id,
                    user_config_env,
                    registered.contains(&agent_plugin_mcp_server_name(&id, &server.name)),
                ));
            }
        }
        definitions.sort_by(|left, right| {
            right
                .declared
                .cmp(&left.declared)
                .then(left.server.cmp(&right.server))
        });
        Ok(definitions)
    }

    pub(super) async fn claude_channel_views(
        &self,
        id: i64,
    ) -> Result<Vec<ClaudeChannelView>, AppError> {
        let session = session_id(id)?;
        let (state, _) = self.claude_channel_executor(session).await?;
        Ok(self
            .claude_channels
            .session(session)
            .iter()
            .map(|binding| binding.view(&state))
            .collect())
    }

    pub(super) async fn update_claude_channel(
        &self,
        id: i64,
        action: ClaudeChannelAction,
    ) -> Result<Vec<ClaudeChannelView>, AppError> {
        if self.shutdown.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        let manager = self.clone();
        self.control_tasks
            .spawn(async move { manager.update_claude_channel_inner(id, action).await })
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?
    }

    async fn update_claude_channel_inner(
        &self,
        id: i64,
        action: ClaudeChannelAction,
    ) -> Result<Vec<ClaudeChannelView>, AppError> {
        let session = session_id(id)?;
        let dispatch = self.session_dispatch(session);
        let _submission = dispatch.submission.lock().await;
        let (server, relay) = match action {
            ClaudeChannelAction::Enable {
                server,
                permission_relay,
            } => (server, Some(permission_relay)),
            ClaudeChannelAction::Disable { server } => (server, None),
        };
        if server.is_empty() || server.len() > 512 || server.trim() != server {
            return Err(AppError::Config("Invalid Claude channel server ID".into()));
        }
        let (state, executor) = self.claude_channel_executor(session).await?;
        let previous = self
            .claude_channels
            .bindings
            .lock()
            .get(&(session, server.clone()))
            .cloned();
        if let Some(previous) = &previous {
            if relay == Some(previous.permission_relay)
                && previous.view(&state).connected
                && self.validate_claude_channel(previous).await.is_ok()
            {
                return self.claude_channel_views(id).await;
            }
            self.claude_channels.remove(previous);
            previous.close().await;
        }
        if let Some(permission_relay) = relay {
            if self.claude_channels.session(session).len() >= MAX_CHANNELS {
                return Err(AppError::Config(
                    "Too many active Claude channels in this session".into(),
                ));
            }
            let owner = executor
                .mcp_manager()
                .cloned()
                .ok_or_else(|| AppError::Config("The MCP bridge is not enabled".into()))?;
            let definition = self
                .claude_channel_definition(session, &executor, &owner, &server)
                .await?;
            let binding = Arc::new(Binding {
                session,
                server: server.clone(),
                definition,
                permission_relay,
                cancellation: self.shutdown.child_token(),
                live: Default::default(),
            });
            let stream = binding.connect(&state, owner).await?;
            self.claude_channels
                .bindings
                .lock()
                .insert((session, server), binding.clone());
            self.track_imported_config_session(session);
            let manager = self.clone();
            self.control_tasks
                .spawn(async move { manager.run_claude_channel(binding, stream).await });
        }
        self.claude_channel_views(id).await
    }

    async fn claude_channel_executor(
        &self,
        session: SessionId,
    ) -> Result<(Arc<ExecutionState>, ToolExecutor), AppError> {
        self.require_writable_bot_history(session.get()).await?;
        let state = self.state_for_session(session).await?;
        let context = self.execution_context_with_state(session, &state).await?;
        if context.global
            || context.owner.is_some()
            || context.channel_authority.is_some()
            || context.relation_kind.is_subagent()
            || context
                .conversation
                .as_ref()
                .is_some_and(|conversation| conversation.is_read_only())
        {
            return Err(AppError::Config(
                "Claude channels require an operator-owned project session".into(),
            ));
        }
        let executor = state.executor.for_session_resource_context(&context);
        let bridge: PluginKey = MCP_BRIDGE.parse().expect("static MCP bridge identity");
        if !executor.plugin_is_enabled(&bridge) {
            return Err(AppError::Config(
                "The MCP bridge is unavailable in this session".into(),
            ));
        }
        Ok((state, executor))
    }

    async fn claude_channel_definition(
        &self,
        session: SessionId,
        executor: &ToolExecutor,
        owner: &McpConnectionManager,
        server: &str,
    ) -> Result<ChannelDefinition, AppError> {
        let target = claude_channel_target(executor, owner, server).await?;
        let spec = owner
            .server_spec_in_workspace(&target.server_id, executor.workspace_root())
            .await
            .map_err(mcp_error)?;
        let ServerSpec::Stdio { cwd, resources, .. } = &spec else {
            return Err(AppError::Config(
                "Claude channels require an MCP stdio server".into(),
            ));
        };
        let prepared = self
            .prepare_host_process_sandbox(
                session.get(),
                SESSION_SCOPED_CALL_ID,
                HostProcessResources {
                    cwd: cwd.as_ref().map(|path| path.to_string_lossy().into_owned()),
                    reads: resources.reads.clone(),
                    writes: resources.writes.clone(),
                    network: resources.network.clone(),
                    require_sandbox: true,
                    ..Default::default()
                },
            )
            .await?;
        if prepared.call_bound {
            return Err(AppError::Config(
                "Claude channels require session-wide MCP resource permissions".into(),
            ));
        }
        Ok(ChannelDefinition {
            target,
            spec,
            authority: McpSandboxContext {
                scope: prepared.scope,
                workspace_root: prepared.workspace_root.into(),
                cwd: prepared.cwd.into(),
                additional_working_directories: prepared.additional_working_directories,
                policy: serde_json::from_value(prepared.policy)?,
            },
        })
    }

    async fn validate_claude_channel(&self, binding: &Binding) -> Result<(), ChannelEnd> {
        if binding.cancellation.is_cancelled() {
            return Err(ChannelEnd::Stopped);
        }
        let (state, executor) = self
            .claude_channel_executor(binding.session)
            .await
            .map_err(|_| ChannelEnd::Stopped)?;
        if !binding.is_attached_to(&state) {
            return Err(ChannelEnd::Disconnected);
        }
        let owner = executor.mcp_manager().ok_or(ChannelEnd::Stopped)?;
        let definition = self
            .claude_channel_definition(binding.session, &executor, owner, &binding.server)
            .await;
        if binding.cancellation.is_cancelled() {
            return Err(ChannelEnd::Stopped);
        }
        if !Arc::ptr_eq(&state, &self.state.load()) {
            return Err(ChannelEnd::Disconnected);
        }
        if definition.is_ok_and(|definition| definition == binding.definition) {
            return Ok(());
        }
        Err(ChannelEnd::Stopped)
    }

    async fn run_claude_channel(&self, binding: Arc<Binding>, mut stream: ClaudeChannelStream) {
        loop {
            let reason = self.read_claude_channel(&binding, stream).await;
            {
                // Channel replacement, scope release and reattachment share
                // the session submission gate. The old manager must finish
                // closing before a later activation can reuse its key.
                let dispatch = self.session_dispatch(binding.session);
                let _submission = dispatch.submission.lock().await;
                binding.close().await;
            }
            if !matches!(reason, ChannelEnd::Disconnected) {
                break;
            }
            let Some(next) = self.reconnect_claude_channel(&binding).await else {
                break;
            };
            stream = next;
        }
        self.claude_channels.remove(&binding);
    }

    async fn reconnect_claude_channel(&self, binding: &Binding) -> Option<ClaudeChannelStream> {
        for attempt in 0..RECONNECT_ATTEMPTS {
            tokio::select! {
                biased;
                _ = binding.cancellation.cancelled() => return None,
                _ = tokio::time::sleep(RECONNECT_DELAY * (1 << attempt)) => {}
            }
            let dispatch = self.session_dispatch(binding.session);
            let _submission = tokio::select! {
                biased;
                _ = binding.cancellation.cancelled() => return None,
                scope = dispatch.submission.lock() => scope,
            };
            let (state, executor) = self.claude_channel_executor(binding.session).await.ok()?;
            let owner = executor.mcp_manager()?.clone();
            let definition = self
                .claude_channel_definition(binding.session, &executor, &owner, &binding.server)
                .await;
            if !Arc::ptr_eq(&state, &self.state.load()) {
                continue;
            }
            // A reload may replace the manager while retaining the same
            // operator selection. Require the exact authorized launch and
            // resource scope before opening a transport in the new manager.
            if !definition.is_ok_and(|definition| definition == binding.definition) {
                return None;
            }
            match binding.connect(&state, owner).await {
                Ok(stream) => return Some(stream),
                Err(error) => {
                    tracing::debug!(session_id = binding.session.get(), server = %binding.server, attempt = attempt + 1, code = %error.failure().code, "Claude channel reconnect failed")
                }
            }
        }
        tracing::warn!(session_id = binding.session.get(), server = %binding.server, attempts = RECONNECT_ATTEMPTS, "Claude channel reconnect attempts exhausted");
        None
    }

    async fn read_claude_channel(
        &self,
        binding: &Binding,
        mut stream: ClaudeChannelStream,
    ) -> ChannelEnd {
        let mut recheck = tokio::time::interval_at(
            tokio::time::Instant::now() + RECHECK_INTERVAL,
            RECHECK_INTERVAL,
        );
        recheck.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut instructions = stream
            .peer()
            .instructions()
            .map(|text| truncate(text, 8192).to_owned());
        loop {
            let event = tokio::select! {
                biased;
                _ = binding.cancellation.cancelled() => return ChannelEnd::Stopped,
                _ = recheck.tick() => {
                    if let Err(reason) = self.validate_claude_channel(binding).await {
                        return reason;
                    }
                    continue;
                }
                event = stream.next() => match event {
                    Some(event) => event,
                    None => return ChannelEnd::Disconnected,
                }
            };
            if let Err(reason) = self.validate_claude_channel(binding).await {
                return reason;
            }
            match event {
                ClaudeChannelEvent::Message { content, meta } => {
                    let dispatch = self.session_dispatch(binding.session);
                    let imported = ImportedPromptScope::current();
                    let admission = tokio::select! {
                        biased;
                        _ = binding.cancellation.cancelled() => return ChannelEnd::Stopped,
                        scope = imported.acquire(&dispatch.submission) => scope,
                    };
                    if !binding.is_attached_to(&self.state.load()) {
                        return ChannelEnd::Disconnected;
                    }
                    let text =
                        channel_prompt(&binding.server, instructions.as_deref(), &content, &meta);
                    let result = admission
                        .run(async {
                            if binding.cancellation.is_cancelled() {
                                return Err(AppError::Cancelled);
                            }
                            self.submit_imported_plugin_prompt(
                                binding.session.get(),
                                binding.definition.target.plugin.clone(),
                                binding.server.clone(),
                                ImportedPluginPromptRequest {
                                    text,
                                    wait: false,
                                    as_user: false,
                                },
                            )
                            .await
                        })
                        .await;
                    if result.is_ok() {
                        instructions = None;
                    } else {
                        tracing::debug!(session_id = binding.session.get(), server = %binding.server, "Claude channel prompt was not accepted");
                    }
                }
                ClaudeChannelEvent::Permission {
                    request_id,
                    allowed,
                } if binding.permission_relay => {
                    let interaction = binding.live.lock().pending.remove(&request_id);
                    if let Some(interaction) = interaction {
                        let command = RequestId::new();
                        let decision = InteractionDecision::Permission {
                            reply: PermissionReply {
                                request_id: interaction.get().to_string(),
                                kind: if allowed {
                                    PermissionReplyKind::AllowOnce
                                } else {
                                    PermissionReplyKind::DenyOnce
                                },
                                reason: None,
                                scope: None,
                            },
                            allowed,
                        };
                        // The local and remote reply compete for the same
                        // durable one-time interaction transaction.
                        let _ = persist(|| {
                            self.kernel().sessions.reply(
                                command,
                                binding.session,
                                interaction,
                                decision.clone(),
                                None,
                            )
                        })
                        .await;
                    }
                }
                ClaudeChannelEvent::Permission { .. } => {}
            }
        }
    }

    pub(super) fn relay_claude_channel_permission(
        &self,
        control: &RunControl,
        interaction: InteractionId,
        tool_name: &str,
        description: &str,
        input: Option<&agena_domain::StructuredObject>,
    ) -> RelayedPermissions {
        let mut relays = Vec::new();
        for binding in self.claude_channels.session(control.session) {
            let Some((id, peer)) = binding.issue_permission(interaction, &control.state) else {
                continue;
            };
            relays.push((binding.clone(), id.clone()));
            let manager = self.clone();
            let tool_name = relay_tool_name(tool_name).to_owned();
            let description = relay_text(description);
            let input_preview = relay_preview(input);
            let cancelled = control.cancel.clone();
            self.control_tasks.spawn(async move {
                tokio::select! {
                    biased;
                    _ = binding.cancellation.cancelled() => {}
                    _ = cancelled.cancelled() => {}
                    _ = tokio::time::timeout(Duration::from_secs(5), async {
                        if manager.validate_claude_channel(&binding).await.is_ok()
                            && binding.live.lock().pending.contains_key(&id)
                        {
                            let _ = peer.request_permission(&id, &tool_name, &description, &input_preview).await;
                        }
                    }) => {}
                }
            });
        }
        RelayedPermissions(relays)
    }
}

async fn claude_channel_target(
    executor: &ToolExecutor,
    owner: &McpConnectionManager,
    server: &str,
) -> Result<ChannelTarget, AppError> {
    let unavailable = || {
        AppError::Config(
            "The selected server must belong to an enabled Claude plugin in this session".into(),
        )
    };
    let Some(qualified) = server.strip_prefix("plugin:") else {
        if owner.is_plugin_server(server).await {
            return Err(unavailable());
        }
        return Ok(ChannelTarget {
            plugin: MCP_BRIDGE.parse().expect("static MCP bridge identity"),
            server_id: server.to_owned(),
            package: None,
        });
    };
    let (id, name) = qualified.rsplit_once(':').ok_or_else(unavailable)?;
    let key: PluginKey = id.parse().map_err(|_| unavailable())?;
    if !executor.plugin_is_enabled(&key) {
        return Err(unavailable());
    }
    let package = executor
        .plugin_manager()
        .plugins()
        .iter()
        .find(|plugin| plugin.key() == key)
        .and_then(|plugin| plugin.agent_plugin())
        .filter(|package| {
            package.format == AgentPluginFormat::ClaudeCode
                && package.mcp_servers.iter().any(|server| server.name == name)
        })
        .ok_or_else(unavailable)?;
    Ok(ChannelTarget {
        plugin: key,
        server_id: agent_plugin_mcp_server_name(id, name),
        package: Some((package.root.clone(), package.version.clone())),
    })
}

fn channel_prompt(
    server: &str,
    instructions: Option<&str>,
    content: &str,
    meta: &BTreeMap<String, String>,
) -> String {
    let source = escape_xml(server, 3072);
    let mut text = String::new();
    if let Some(instructions) = instructions {
        let _ = writeln!(
            text,
            "<channel-instructions source=\"{source}\">\n{}\n</channel-instructions>",
            escape_xml(instructions, 16 * 1024)
        );
    }
    let _ = write!(text, "<channel source=\"{source}\"");
    for (key, value) in meta {
        let _ = write!(text, " {key}=\"{}\"", escape_xml(value, 1024));
    }
    let _ = write!(text, ">\n{}\n</channel>", escape_xml(content, 128 * 1024));
    text
}

fn escape_xml(text: &str, limit: usize) -> String {
    let mut escaped = String::with_capacity(text.len().min(limit));
    for ch in text.chars() {
        let entity = match ch {
            '&' => "&amp;",
            '<' => "&lt;",
            '>' => "&gt;",
            '"' => "&quot;",
            '\'' => "&apos;",
            ch if ch.is_control() && !matches!(ch, '\n' | '\r' | '\t') => "\u{fffd}",
            _ => {
                if escaped.len() + ch.len_utf8() > limit {
                    break;
                }
                escaped.push(ch);
                continue;
            }
        };
        if escaped.len() + entity.len() > limit {
            break;
        }
        escaped.push_str(entity);
    }
    escaped
}

fn truncate(text: &str, limit: usize) -> &str {
    &text[..text.floor_char_boundary(text.len().min(limit))]
}

fn relay_text(text: &str) -> String {
    let folded = text
        .chars()
        .map(|ch| match ch {
            '\u{061c}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}'
            | '\u{feff}' => ' ',
            '\u{2018}' | '\u{2019}' | '\u{ff07}' => '\'',
            '\u{201c}' | '\u{201d}' | '\u{ff02}' => '"',
            '\u{2039}' | '\u{ff1c}' => '<',
            '\u{203a}' | '\u{ff1e}' => '>',
            ch if ch.is_control() => ' ',
            ch => ch,
        })
        .collect::<String>();
    let folded = folded.split_whitespace().collect::<Vec<_>>().join(" ");
    let delimiter = |ch| {
        matches!(
            ch,
            ' ' | '='
                | ':'
                | '"'
                | '\''
                | '`'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | ','
                | ';'
                | '<'
                | '>'
        )
    };
    let mut masked = String::with_capacity(folded.len());
    let mut redact_next = false;
    for part in folded.split_inclusive(delimiter) {
        let value = part.trim_end_matches(delimiter);
        let suffix = &part[value.len()..];
        if !value.is_empty() {
            if redact_next || provider_credential(value) {
                masked.push_str("[REDACTED]");
            } else {
                masked.push_str(value);
            }
            redact_next =
                value.eq_ignore_ascii_case("bearer") || (suffix == "=" && sensitive_key(value));
        }
        masked.push_str(suffix);
    }
    let length = masked.chars().count();
    if length <= 3500 {
        return masked;
    }
    let head: String = masked.chars().take(1700).collect();
    let tail: String = masked.chars().skip(length - 1700).collect();
    format!("{head} ⋯ {} code points elided ⋯ {tail}", length - 3400)
}

fn provider_credential(token: &str) -> bool {
    token.len() >= 12
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        && [
            "sk-",
            "ghp_",
            "gho_",
            "ghu_",
            "ghs_",
            "ghr_",
            "github_pat_",
            "glpat-",
            "xoxb-",
            "xoxp-",
            "xapp-",
            "AKIA",
            "ASIA",
            "AIza",
        ]
        .iter()
        .any(|prefix| token.starts_with(prefix))
}

fn sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "token",
        "password",
        "secret",
        "credential",
        "apikey",
        "api_key",
        "authorization",
    ]
    .iter()
    .any(|marker| key.contains(marker))
}

fn relay_tool_name(tool: &str) -> &str {
    match tool.strip_prefix("agena.").unwrap_or(tool) {
        "fs.read" => "Read",
        "fs.write" => "Write",
        "fs.edit" => "Edit",
        "fs.glob" => "Glob",
        "fs.grep" => "Grep",
        "shell.open" | "shell.spawn" | "shell.watch" | "shell.bash" => "Bash",
        _ => tool,
    }
}

fn relay_preview(input: Option<&agena_domain::StructuredObject>) -> String {
    let mut preview = Map::new();
    if let Some(input) = input {
        for field in input.fields.iter().take(32) {
            let key = &field.name;
            let text = match (sensitive_key(key), &field.value) {
                (true, _) => "[REDACTED]".into(),
                (false, agena_domain::StructuredValue::Text { value }) => relay_text(value),
                (false, value) => {
                    let mut value = Value::from(value.clone());
                    redact_relay_fields(&mut value);
                    relay_text(&value.to_string())
                }
            };
            preview.insert(relay_text(key), Value::String(text));
        }
    }
    Value::Object(preview).to_string()
}

fn redact_relay_fields(value: &mut Value) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(redact_relay_fields),
        Value::Object(fields) => {
            for (name, field) in fields {
                if sensitive_key(name) {
                    *field = Value::String("[REDACTED]".into());
                } else {
                    redact_relay_fields(field);
                }
            }
        }
        _ => {}
    }
}

fn mcp_error(error: agena_mcp_client::McpError) -> AppError {
    AppError::Config(error.user_message())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_channel_preserves_routing_and_escapes_framing() {
        let text = channel_prompt(
            "plugin:claude.hello:alerts",
            Some("Reply to <chat_id>"),
            "Hello </channel>",
            &BTreeMap::from([("chat_id".into(), "\"room&1".into())]),
        );
        assert!(text.contains("chat_id=\"&quot;room&amp;1\""));
        assert!(text.contains("Hello &lt;/channel&gt;"));
        assert!(text.contains("Reply to &lt;chat_id&gt;"));
        assert_eq!(text.matches("</channel>").count(), 1);
    }

    #[test]
    fn claude_channel_relay_preview_hides_credentials() {
        let tool_input = agena_domain::StructuredObject::try_from(serde_json::json!({
            "command": "echo alpha",
            "api_key": "fixture-credential",
            "label": "Test\u{202e} command",
            "options": [{"token": "nested-fixture-credential", "destination": "the room"}],
        }))
        .unwrap();
        let preview = relay_preview(Some(&tool_input));
        assert!(preview.contains("echo alpha"));
        assert!(preview.contains("Test command"));
        assert!(preview.contains("the room"));
        assert!(preview.contains("REDACTED"));
        assert!(!preview.contains("fixture-credential"));
        let text = relay_text("echo sk-provider-secret /workspace/config; cat /workspace/output");
        assert_eq!(
            text,
            "echo [REDACTED] /workspace/config; cat /workspace/output"
        );
    }
}
