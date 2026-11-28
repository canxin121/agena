use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
    time::Duration,
};

use agena_plugin_sdk::{
    HostClient, PluginKey,
    host_api::{
        HookTranscriptFormat, HookTranscriptScope, HostProcessExecution, HostProcessResources,
        ImportedHookEnvironment, ImportedHookTranscripts, LogLevel,
    },
};
use agena_process::sandbox::{self, SandboxPolicy};
use serde_json::{Value, json};
use tokio::time::Instant;

use super::{
    Action, CodexShellHook, Command, Dialect, Effects, Event, Handler, HandlerMode, HookInput,
    HookPathOverrides, HookScope, HookSet, Shell,
    background::BackgroundJobs,
    codex_shell_aliases,
    http::HttpHook,
    monitor,
    scope::{OnceHook, PendingFork, ScopedHooks},
    sequential_hook_budget,
};
use crate::agent_plugin::{
    AgentPluginDescriptor, agent_plugin_data_path, agent_plugin_mcp_server_name, claude_mods, mcp,
};

const MAX_INPUT_BYTES: usize = 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_CODEX_CONTEXT_OUTPUT_BYTES: usize = 1024 * 1024;
static DATA_DIRECTORIES: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

#[derive(Debug, Clone, thiserror::Error)]
enum HookFailure {
    #[error("{0}")]
    Timeout(&'static str),
    #[error("{0}")]
    Other(String),
}

impl From<String> for HookFailure {
    fn from(message: String) -> Self {
        Self::Other(message)
    }
}

impl From<&str> for HookFailure {
    fn from(message: &str) -> Self {
        Self::Other(message.to_owned())
    }
}

fn codex_model_hook_units(content: Option<&Value>) -> Result<f64, HookFailure> {
    let Some(content) = content.filter(|content| !content.is_null()) else {
        return Ok(0.0);
    };
    let fields = content
        .as_object()
        .ok_or("Codex post-hook structuredContent must be an object or null")?;
    let Some(units) = fields.get("extra_codex_rollout_budget_units") else {
        return Ok(0.0);
    };
    units
        .as_f64()
        .filter(|units| units.is_finite() && *units >= 0.0)
        .ok_or_else(|| "Codex hook budget units must be a finite nonnegative number".into())
}

fn uses_shell_environment(handler: &Handler, event: Event) -> bool {
    if handler.dialect != Dialect::Claude
        || !matches!(
            event,
            Event::SessionStart | Event::Setup | Event::CwdChanged | Event::FileChanged
        )
    {
        return false;
    }
    match &handler.action {
        Action::Command(Command::Shell { shell, .. }) => {
            !matches!(shell, Shell::PowerShell) && !(cfg!(windows) && *shell == Shell::Default)
        }
        Action::Command(Command::Exec { program, .. }) => !Path::new(program)
            .file_stem()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.eq_ignore_ascii_case("powershell") || name.eq_ignore_ascii_case("pwsh")
            }),
        _ => false,
    }
}

pub(super) fn transcript_scope(event: Event, dialect: Dialect) -> HookTranscriptScope {
    match (event, dialect) {
        (Event::SubagentStop | Event::TeammateIdle, _) => HookTranscriptScope::ParentAndAgent,
        (Event::SubagentStart, Dialect::Claude) => HookTranscriptScope::Parent,
        _ => HookTranscriptScope::Current,
    }
}

impl From<Dialect> for HookTranscriptFormat {
    fn from(dialect: Dialect) -> Self {
        match dialect {
            Dialect::Claude => Self::Claude,
            Dialect::Codex => Self::Codex,
        }
    }
}

pub(in crate::agent_plugin) struct HookRunner {
    hooks: HookSet,
    mods: Option<std::sync::Arc<claude_mods::Mods>>,
    root: PathBuf,
    plugin_id: String,
    package_name: String,
    environment: Vec<String>,
    user_config_env: BTreeMap<String, String>,
    option_defaults: BTreeMap<String, String>,
    background: BackgroundJobs,
    scopes: ScopedHooks,
    skill_dirs: BTreeSet<PathBuf>,
    mcp_servers: BTreeSet<String>,
    monitors: Vec<monitor::Monitor>,
}

impl HookRunner {
    pub(in crate::agent_plugin) fn new(
        package: &AgentPluginDescriptor,
        key: &PluginKey,
        environment: &[String],
        user_config_env: &BTreeMap<String, String>,
    ) -> Self {
        Self {
            hooks: package.hooks.clone(),
            mods: package.mod_runtime.clone(),
            root: package.root.clone(),
            plugin_id: key.to_string(),
            package_name: package.name.clone(),
            environment: environment.to_vec(),
            user_config_env: user_config_env.clone(),
            option_defaults: package.claude_option_defaults.clone(),
            background: BackgroundJobs::default(),
            scopes: ScopedHooks::default(),
            skill_dirs: package.skill_dirs.iter().cloned().collect(),
            mcp_servers: package
                .mcp_servers
                .iter()
                .map(|server| server.name.clone())
                .collect(),
            monitors: package.monitors.clone(),
        }
    }

    pub(super) fn end_session(&self, session: i64) {
        self.background.end_session(session);
        self.scopes.end_session(session);
        if let Some(mods) = &self.mods {
            mods.close(session);
        }
    }

    pub(super) async fn instructions_loaded(
        &self,
        host: &dyn HostClient,
        input: agena_plugin_sdk::ImportedInstructionsLoadedInput,
    ) {
        let session = input.session_id;
        let files = self
            .scopes
            .new_instructions(session, input.compacted, input.files);
        for file in files {
            use agena_plugin_sdk::ImportedInstructionOrigin;

            let mut fields = json!({"file_path":file.path, "memory_type":file.scope});
            if let Some(path) = &file.trigger_file_path {
                fields["trigger_file_path"] = json!(path);
            }
            fields["load_reason"] = json!(match file.origin {
                ImportedInstructionOrigin::Include { parent_file_path } => {
                    fields["parent_file_path"] = json!(parent_file_path);
                    "include"
                }
                ImportedInstructionOrigin::PathGlobMatch { globs } => {
                    fields["globs"] = json!(globs);
                    "path_glob_match"
                }
                ImportedInstructionOrigin::Conventional if file.trigger_file_path.is_some() => {
                    "nested_traversal"
                }
                ImportedInstructionOrigin::Conventional if input.compacted => "compact",
                ImportedInstructionOrigin::Conventional => "session_start",
            });
            self.run(
                host,
                Event::InstructionsLoaded,
                session,
                fields,
                &[],
                Some(Dialect::Claude),
            )
            .await;
        }
    }

    pub(super) fn mods(&self) -> Option<&claude_mods::Mods> {
        self.mods.as_deref()
    }

    pub(super) fn stop_background(&self) {
        self.background.stop_all();
    }

    pub(super) async fn watch_files(
        &self,
        host: &dyn HostClient,
        session: i64,
        paths: Option<Vec<PathBuf>>,
    ) {
        if paths.is_none()
            && !self.hooks.0.contains_key(&Event::FileChanged)
            && !self
                .mods
                .as_ref()
                .is_some_and(|mods| mods.matches("classic.FileChanged"))
        {
            return;
        }
        if let Err(error) = host.configure_imported_plugin_files(paths, None).await {
            host.log(
                LogLevel::Warn,
                error.diagnostic_message().to_owned(),
                json!({"hook": "FileChanged", "session": session}),
            )
            .await;
        }
    }

    pub(super) fn activate_skill(&self, session: i64, document: &Path) -> bool {
        let Some(directory) = document
            .parent()
            .filter(|_| document.file_name() == Some("SKILL.md".as_ref()))
        else {
            return false;
        };
        if self.skill_dirs.contains(directory) {
            self.activate(session, HookScope::Skill(directory.to_owned()));
            true
        } else {
            false
        }
    }

    pub(super) fn prepare_fork(
        &self,
        parent: i64,
        name: &str,
        scope: &HookScope,
    ) -> Option<PendingFork<'_>> {
        self.hooks
            .0
            .values()
            .flatten()
            .any(|handler| &handler.scope == scope)
            .then(|| self.scopes.prepare_fork(parent, name, scope.clone()))
    }

    pub(super) fn activate_fork(&self, parent: i64, child: i64, name: &str) {
        self.scopes.activate_fork(parent, child, name);
    }

    pub(super) fn activate(&self, session: i64, scope: HookScope) {
        if self
            .hooks
            .0
            .values()
            .flatten()
            .any(|handler| handler.scope == scope)
        {
            self.scopes.activate(session, scope);
        }
    }

    pub(super) async fn start_monitors(
        &self,
        host: &dyn HostClient,
        session: i64,
        skill: Option<&str>,
    ) {
        let pending = self
            .monitors
            .iter()
            .filter(|monitor| {
                monitor.skill.as_deref() == skill
                    && !self.background.started(session, &monitor.name)
            })
            .collect::<Vec<_>>();
        if pending.is_empty() {
            return;
        }
        let result = tokio::time::timeout(Duration::from_secs(4), async {
            let prepared = self.prepare(host, session, true, true, false, None).await?;
            let data = prepared
                .data
                .as_deref()
                .ok_or("monitor data directory is unavailable")?;
            for monitor in pending {
                let script = Command::Shell {
                    script: monitor.command.clone(),
                    shell: Shell::Default,
                };
                let (program, args, env) = self.command(
                    Dialect::Claude,
                    &script,
                    &prepared.workspace_root,
                    data,
                    &HookPathOverrides::default(),
                    None,
                )?;
                let command = prepared.wrap(program, args, env, None, None)?;
                let binding = host
                    .bind_imported_plugin_background()
                    .await
                    .map_err(|error| error.diagnostic_message().to_owned())?;
                let monitor = monitor.clone();
                let plugin = self.plugin_id.clone();
                let name = monitor.name.clone();
                self.background.start(
                    session,
                    Some(name.clone()),
                    binding.cancellation,
                    async move {
                        if let Err(error) =
                            monitor::run(command, monitor, binding.notices.as_ref()).await
                        {
                            tracing::warn!(%plugin, %name, %error, "plugin monitor failed");
                        }
                    },
                )?;
            }
            Ok::<_, String>(())
        })
        .await
        .unwrap_or_else(|_| Err("monitor admission timed out".into()));
        if let Err(error) = result {
            host.log(LogLevel::Warn, error, json!({"component":"monitors"}))
                .await;
        }
    }

    pub(super) async fn run<'a>(
        &self,
        host: &dyn HostClient,
        event: Event,
        session: i64,
        input: impl Into<HookInput<'a>>,
        aliases: &[&str],
        dialect: Option<Dialect>,
    ) -> Effects {
        let mut input = input.into();
        let module_event = event.classic_mod_name();
        if matches!(event, Event::Stop | Event::SubagentStop | Event::TeammateIdle)
            && dialect != Some(Dialect::Codex)
            && (self.hooks.0.get(&event).is_some_and(|handlers| {
                handlers.iter().any(|handler| handler.dialect == Dialect::Claude)
            }) || self.mods.as_ref().is_some_and(|mods| mods.matches(&module_event)))
            && let Ok(Ok(Some(work))) = tokio::time::timeout(
                Duration::from_secs(1),
                host.imported_claude_stop_work(
                    matches!(event, Event::SubagentStop | Event::TeammateIdle),
                    None,
                ),
            )
            .await
        {
            input.fields["background_tasks"] = json!(work.background_tasks);
            input.fields["session_crons"] = json!(work.session_crons);
        }
        if !event.is_worktree()
            && dialect != Some(Dialect::Codex)
            && let Some(mods) = &self.mods
            && mods.matches(&module_event)
        {
            return super::mods::run_classic(
                mods,
                host,
                &module_event,
                event,
                session,
                input,
                |input| self.run_settings(host, event, session, input, aliases, dialect),
            )
            .await;
        }
        self.run_settings(host, event, session, input, aliases, dialect)
            .await
    }

    async fn run_settings<'a>(
        &self,
        host: &dyn HostClient,
        event: Event,
        session: i64,
        input: HookInput<'a>,
        aliases: &[&str],
        dialect: Option<Dialect>,
    ) -> Effects {
        let HookInput {
            mut fields,
            claude_mcp,
            codex_tool_cwd,
            codex_shell_hook,
            skip_scope,
            canonical_model,
            process_cwd,
        } = input;
        let Some(handlers) = self.hooks.0.get(&event) else {
            return Effects::default();
        };
        let mut matched: Vec<_> = handlers
            .iter()
            .enumerate()
            .filter(|(_, handler)| {
                if skip_scope.as_ref() == Some(&handler.scope) {
                    return false;
                }
                if dialect.is_some_and(|dialect| handler.dialect != dialect) {
                    return false;
                }
                if event.is_model_switch() {
                    return canonical_model.is_none_or(|model| handler.matcher.matches(model));
                }
                let (tool_name, aliases) = match (handler.dialect, codex_shell_hook) {
                    (Dialect::Codex, CodexShellHook::Skip) => return false,
                    (Dialect::Codex, CodexShellHook::Launch(launch)) => {
                        (Some("Bash"), codex_shell_aliases(launch))
                    }
                    (Dialect::Codex, CodexShellHook::Completed(result)) => {
                        (Some("Bash"), codex_shell_aliases(result.invocation.mode))
                    }
                    (Dialect::Claude, _) if claude_mcp.is_some() => {
                        (claude_mcp.map(|tool| tool.tool_name.as_str()), aliases)
                    }
                    _ => (fields["tool_name"].as_str(), aliases),
                };
                event.matcher_field().is_none_or(|field| {
                    let value = if field == "tool_name" {
                        tool_name
                    } else {
                        fields[field].as_str()
                    }
                    .unwrap_or_default();
                    let value = if event == Event::FileChanged {
                        Path::new(value)
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or_default()
                    } else {
                        value
                    };
                    handler.matcher.matches(value)
                        || (field == "tool_name"
                            && aliases.iter().any(|name| handler.matcher.matches(name)))
                }) && handler.condition.as_ref().is_none_or(|condition| {
                    tool_name.is_some_and(|name| condition.matches_tool(name, aliases))
                })
            })
            .collect();
        let profile = if matched
            .iter()
            .any(|(_, handler)| matches!(handler.scope, HookScope::Agent(_)))
        {
            match host.imported_hook_profile().await {
                Ok(profile) => profile,
                Err(error) => {
                    host.log(
                        LogLevel::Warn,
                        error.diagnostic_message().to_owned(),
                        json!({"hook": event.name()}),
                    )
                    .await;
                    None
                }
            }
        } else {
            None
        };
        let role = profile
            .as_deref()
            .and_then(|profile| profile.strip_prefix(self.plugin_id.as_str()))
            .and_then(|profile| profile.strip_prefix('.'));
        matched.retain(|(_, handler)| match &handler.scope {
            HookScope::Package => event != Event::Stop || fields.get("agent_id").is_none(),
            HookScope::Agent(name) => role == Some(name.as_str()),
            scope => self.scopes.is_active(session, scope),
        });
        if matched.is_empty() {
            return Effects::default();
        }

        // Leave time to translate a failure before the enclosing hook RPC's
        // default deadline. All matched commands share the phase's budget.
        let budget = match event {
            Event::Setup | Event::SessionStart => 600,
            Event::PreToolUse | Event::PostToolUse | Event::Stop => 28,
            Event::PreCompact
            | Event::PostCompact
            | Event::PostToolBatch
            | Event::PermissionRequest
            | Event::TaskCreated
            | Event::TaskCompleted
            | Event::Elicitation
            | Event::ElicitationResult
            | Event::PreSkillShellCommand
            | Event::SubagentStart
            | Event::SubagentStop
            | Event::PreModelRequest
            | Event::PostModelRequest => 9,
            _ => 4,
        };
        let budget = matched
            .iter()
            .filter_map(|(_, handler)| handler.agent_timeout())
            .fold(Duration::from_secs(budget), Duration::max);
        let budget = if event.is_model_switch()
            || event.is_model_attempt()
            || event.is_worktree()
            || matches!(event, Event::MessageDisplay | Event::ConfigChange | Event::DirectoryAdded)
        {
            sequential_hook_budget(matched.iter().map(|(_, handler)| *handler))
        } else {
            budget
        };
        let deadline = Instant::now() + budget;
        let prepare_data = matched
            .iter()
            .any(|(_, handler)| matches!(handler.action, Action::Command(_)));
        let require_sandbox = matched
            .iter()
            .all(|(_, handler)| matches!(handler.action, Action::Command(_) | Action::Http(_)));
        let shell_environment = matched
            .iter()
            .any(|(_, handler)| uses_shell_environment(handler, event));
        let mut prepared = tokio::time::timeout_at(
            deadline,
            self.prepare(
                host,
                session,
                prepare_data,
                require_sandbox,
                shell_environment,
                process_cwd,
            ),
        )
        .await
        .map_err(|_| HookFailure::Timeout("sandbox preparation timed out"))
        .and_then(|prepared| prepared.map_err(HookFailure::from));
        let mut effects = Effects::default();
        let mut context_budget = if event == Event::PreCompact { 64 } else { 16 } * 1024;
        let mut message_budget = 8 * 1024;
        for (index, handler) in matched {
            if agena_plugin_sdk::current_cancellation_token()
                .is_some_and(|token| token.is_cancelled())
            {
                break;
            }
            if let Some(condition) = &handler.condition {
                let cwd = prepared.as_ref().ok().map(|prepared| prepared.cwd.as_str());
                let fallback = agena_plugin_sdk::host_api::current_host_callback_context()
                    .and_then(|context| context.workspace_root);
                if !condition.matches_input(&fields["tool_input"], cwd.or(fallback.as_deref())) {
                    continue;
                }
            }
            if event == Event::PreSkillShellCommand
                && matches!(handler.action, Action::Prompt(_) | Action::Agent(_))
            {
                effects.block.get_or_insert_with(|| {
                    format!("{} does not support model hook handlers", event.name(),)
                });
                continue;
            }
            let mut once = if handler.once {
                let Some(claim) = self.scopes.claim_once(session, event, index) else {
                    continue;
                };
                Some(claim)
            } else {
                None
            };
            let codex_shell = handler.dialect == Dialect::Codex
                && codex_tool_cwd.is_some()
                && (fields["tool_name"] == "Bash"
                    || matches!(codex_shell_hook, CodexShellHook::Launch(_)))
                && fields["tool_input"]["command"].is_string();
            let result = match &mut prepared {
                Ok(prepared) => {
                    if matches!(handler.action, Action::Command(_))
                        && !prepared.policy.disabled
                        && sandbox::enabled()
                    {
                        prepared
                            .load_transcripts(host, event, handler.dialect, deadline)
                            .await;
                    }
                    let cwd = match handler.dialect {
                        Dialect::Codex => codex_tool_cwd.unwrap_or(&prepared.cwd),
                        Dialect::Claude => &prepared.cwd,
                    };
                    fields["cwd"] = json!(cwd);
                    if fields.get("session_id").is_none() {
                        fields["session_id"] = json!(session.to_string());
                    }
                    fields["hook_event_name"] = json!(event.name());
                    if let Some(role) = role
                        && !event.is_model_attempt()
                        && matches!(handler.scope, HookScope::Agent(_))
                    {
                        fields["agent_id"] = json!(session.to_string());
                        fields["agent_type"] = json!(format!("{}:{role}", self.package_name));
                    }
                    if !event.is_model_attempt()
                        && event != Event::MessageDisplay
                        && let Some(execution) = &prepared.execution
                    {
                        fields["turn_id"] = json!(execution.id);
                        fields["model"] = json!(execution.model);
                    }
                    fields["transcript_path"] = match handler.dialect {
                        Dialect::Claude => json!(""),
                        Dialect::Codex => Value::Null,
                    };
                    if fields.get("permission_mode").is_none() {
                        fields["permission_mode"] = json!("default");
                    }
                    if handler.dialect == Dialect::Codex && fields.get("model").is_none() {
                        fields["model"] = json!("");
                    }
                    let mut projected_fields;
                    let handler_fields = if handler.dialect == Dialect::Codex
                        && let CodexShellHook::Completed(result) = codex_shell_hook
                    {
                        projected_fields = fields.clone();
                        projected_fields["cwd"] = json!(result.invocation.cwd);
                        projected_fields["tool_name"] = json!("Bash");
                        projected_fields["tool_use_id"] =
                            json!(result.invocation.call.call_id.to_string());
                        projected_fields["tool_input"] =
                            json!({"command": result.invocation.command});
                        projected_fields["tool_response"] = json!(result.output);
                        &projected_fields
                    } else if codex_shell {
                        projected_fields = fields.clone();
                        projected_fields["tool_name"] = json!("Bash");
                        projected_fields["tool_input"] =
                            json!({"command":fields["tool_input"]["command"]});
                        let response = &fields["tool_response"];
                        if event == Event::PostToolUse
                            && let Some(output) = response["output"].as_str()
                        {
                            let output = response["output_resource"]["resource_id"]
                                .as_str()
                                .and_then(|id| {
                                    output.strip_suffix(&format!("\n\nOutput resource: {id}"))
                                })
                                .unwrap_or(output);
                            projected_fields["tool_response"] = json!(output);
                        }
                        &projected_fields
                    } else if handler.dialect == Dialect::Claude
                        && let Some(tool) = claude_mcp
                    {
                        projected_fields = fields.clone();
                        tool.apply(&mut projected_fields);
                        &projected_fields
                    } else {
                        &fields
                    };
                    self.invoke(
                        host,
                        handler,
                        event,
                        handler_fields,
                        prepared,
                        deadline,
                        &mut once,
                    )
                    .await
                }
                Err(error) => Err(error.clone()),
            };
            match result {
                Ok(output) => {
                    if output.successful
                        && let Some(claim) = once.take()
                    {
                        claim.complete();
                    }
                    if event.is_worktree() {
                        effects.worktree = output.effects.worktree;
                        break;
                    }
                    if event == Event::MessageDisplay {
                        if let Some(text) = output.effects.display_content {
                            effects.display_content = Some(text);
                        }
                        continue;
                    }
                    if event == Event::ConfigChange {
                        if output.effects.block.is_some() {
                            effects.block = output.effects.block;
                            break;
                        }
                        continue;
                    }
                    if event == Event::DirectoryAdded {
                        if let Some(message) = output.system_message {
                            host.log(
                                LogLevel::Debug,
                                message,
                                json!({"hook": event.name(), "source": handler.source}),
                            )
                            .await;
                        }
                        continue;
                    }
                    if event.is_model_attempt() {
                        let sum = effects.extra_codex_rollout_budget_units
                            + output.effects.extra_codex_rollout_budget_units;
                        if sum.is_finite() {
                            effects.extra_codex_rollout_budget_units = sum;
                        } else {
                            host.log(
                                LogLevel::Warn,
                                "Codex model request hook charges exceed the numeric range".into(),
                                json!({"hook":event.name(), "source":handler.source}),
                            )
                            .await;
                        }
                        continue;
                    }
                    if let Some(message) = output.system_message {
                        if event.is_model_switch() && !message.is_empty() && message_budget > 0 {
                            let end =
                                message.floor_char_boundary(message.len().min(message_budget));
                            effects
                                .model_switch
                                .system_messages
                                .push(message[..end].to_owned());
                            message_budget = message_budget.saturating_sub(end + 1);
                        }
                        host.log(
                            LogLevel::Info,
                            message,
                            json!({"hook": event.name(), "source": handler.source}),
                        )
                        .await;
                    }
                    if let Some(mut input) = output.effects.input {
                        if codex_shell {
                            let Some(command) =
                                input.get("command").filter(|value| value.is_string())
                            else {
                                effects.block = Some(
                                    "Codex Bash hook updatedInput requires a string command".into(),
                                );
                                break;
                            };
                            // Codex rewrites the command without replacing the
                            // launch settings or the declared permission effects.
                            let mut updated = fields["tool_input"].clone();
                            updated["command"] = command.clone();
                            input = updated;
                        }
                        fields["tool_input"] = input.clone();
                        effects.input = Some(input);
                    }
                    if let Some(output) = output.effects.output {
                        fields["tool_response"] = output.clone();
                        effects.output = Some(output);
                    }
                    for mut context in output.effects.context {
                        if context_budget > 0 && !context.is_empty() {
                            if let Some(limit) = handler.additional_context_limit {
                                let directory =
                                    codex_hook_output_directory(&self.plugin_id, session);
                                context =
                                    spill_codex_context(directory, context, limit, context_budget)
                                        .await;
                            }
                            context.truncate(
                                context.floor_char_boundary(context.len().min(context_budget)),
                            );
                            context_budget = context_budget.saturating_sub(context.len() + 1);
                            effects.context.push(context);
                        }
                    }
                    if let Some(mut message) = output.effects.initial_message {
                        message.truncate(message.floor_char_boundary(message.len().min(16 * 1024)));
                        effects.initial_message = Some(message);
                    }
                    if output.effects.session_title.is_some() {
                        effects.session_title = output.effects.session_title;
                    }
                    effects.reload_skills |= output.effects.reload_skills;
                    if output.effects.watch_paths.is_some() {
                        effects.watch_paths = output.effects.watch_paths;
                    }
                    if let Some(reply) = output.effects.elicitation {
                        match &mut effects.elicitation {
                            Some(previous) => previous.prefer(reply),
                            None => effects.elicitation = Some(reply),
                        }
                    }
                    if let Some(decision) = output.effects.model_switch.decision {
                        effects.model_switch.apply_decision(decision);
                    }
                    if let Some(stop) = output.effects.stop_skill {
                        effects.stop_skill = Some(stop);
                        break;
                    }
                    if let Some(mut reason) = output.effects.permission_ask {
                        reason.truncate(reason.floor_char_boundary(reason.len().min(8192)));
                        effects.permission_ask.get_or_insert(reason);
                    }
                    if let Some(block) = output.effects.block {
                        effects.block.get_or_insert(block);
                        if !matches!(event, Event::PreSkillShellCommand | Event::SessionStart) {
                            break;
                        }
                    }
                }
                Err(error) => {
                    let level = if event == Event::DirectoryAdded {
                        LogLevel::Debug
                    } else {
                        LogLevel::Warn
                    };
                    host.log(
                        level,
                        error.to_string(),
                        json!({"hook": event.name(), "source": handler.source}),
                    )
                    .await;
                    if event.is_worktree() {
                        effects.block = Some(format!(
                            "{} hook in {} failed: {error}",
                            event.name(),
                            handler.source,
                        ));
                        break;
                    }
                    if event == Event::PreModelSwitch && handler.mode == HandlerMode::Foreground {
                        let reason = if matches!(error, HookFailure::Timeout(_)) {
                            Some(format!(
                                "PreModelSwitch hook in {} timed out",
                                handler.source
                            ))
                        } else if handler.block_on_failure {
                            Some(format!("PreModelSwitch hook in {} failed", handler.source))
                        } else {
                            None
                        };
                        if let Some(reason) = reason {
                            effects.model_switch.apply_decision(
                                agena_plugin_sdk::ImportedModelSwitchDecision::Deny(reason),
                            );
                        }
                        continue;
                    }
                    if handler.mode == HandlerMode::Foreground
                        && handler.block_on_failure
                        && event.can_block_on_failure(handler.dialect)
                    {
                        effects.block.get_or_insert_with(|| {
                            if handler.dialect == Dialect::Codex {
                                format!("{} hook failed. This hook is configured to block continuation on failure.", event.name())
                            } else {
                                format!("{} hook in {} failed", event.name(), handler.source)
                            }
                        });
                        if !matches!(event, Event::PreSkillShellCommand | Event::SessionStart) {
                            break;
                        }
                    }
                }
            }
        }
        effects
    }

    async fn prepare(
        &self,
        host: &dyn HostClient,
        session: i64,
        prepare_data: bool,
        require_sandbox: bool,
        shell_environment: bool,
        cwd: Option<&Path>,
    ) -> Result<PreparedHook, String> {
        let prepared = host
            .prepare_process_sandbox(HostProcessResources {
                cwd: cwd.map(|path| path.to_string_lossy().into_owned()),
                require_sandbox,
                ..Default::default()
            })
            .await
            .map_err(|error| error.diagnostic_message().to_owned())?;
        let policy: SandboxPolicy = serde_json::from_value(prepared.policy)
            .map_err(|_| "invalid host process policy".to_owned())?;
        let data = if prepare_data && !policy.disabled && sandbox::enabled() {
            let plugin_id = self.plugin_id.clone();
            let workspace = PathBuf::from(&prepared.workspace_root);
            Some(
                DATA_DIRECTORIES
                    .run(move || {
                        let data = agent_plugin_data_path(&workspace, &plugin_id);
                        agena_storage_layout::secure_directory(&data).map(|()| data)
                    })
                    .await
                    .map_err(|_| "hook data directory worker failed")?
                    .map_err(|error| {
                        format!("cannot prepare hook data directory: {}", error.kind())
                    })?,
            )
        } else {
            None
        };
        let environment = if shell_environment && !policy.disabled && sandbox::enabled() {
            match host.imported_hook_environment(None).await {
                Ok(environment) => environment,
                Err(error) => {
                    host.log(
                        LogLevel::Warn,
                        error.diagnostic_message().to_owned(),
                        json!({"component":"CLAUDE_ENV_FILE"}),
                    )
                    .await;
                    None
                }
            }
        } else {
            None
        };
        Ok(PreparedHook {
            session,
            cwd: prepared.cwd,
            workspace_root: prepared.workspace_root,
            policy,
            data,
            execution: prepared.execution,
            environment,
            transcripts: BTreeMap::new(),
        })
    }

    async fn invoke(
        &self,
        host: &dyn HostClient,
        handler: &Handler,
        event: Event,
        fields: &Value,
        prepared: &PreparedHook,
        deadline: Instant,
        once: &mut Option<OnceHook>,
    ) -> Result<super::dispatch::HookOutput, HookFailure> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(HookFailure::Timeout("hook deadline elapsed"));
        }
        let transcripts = matches!(handler.action, Action::Command(_))
            .then(|| prepared.transcripts.get(&handler.dialect.into()))
            .flatten();
        let codex_session_start = handler.dialect == Dialect::Codex && event == Event::SessionStart;
        let mut local_fields;
        let fields = if transcripts.is_some() || codex_session_start {
            local_fields = fields.clone();
            if let Some(file) = transcripts.and_then(|files| files.transcript.as_ref()) {
                local_fields["transcript_path"] = json!(file.path());
            }
            if let Some(file) = transcripts.and_then(|files| files.agent.as_ref()) {
                local_fields["agent_transcript_path"] = json!(file.path());
            }
            if codex_session_start && let Some(object) = local_fields.as_object_mut() {
                for key in [
                    "agent_type",
                    "session_title",
                    "seconds_since_last_response",
                    "context_tokens",
                    "prompt_cache_likely_expired",
                    "estimated_cache_write_usd",
                ] {
                    object.remove(key);
                }
            }
            &local_fields
        } else {
            fields
        };
        let payload = serde_json::to_string(fields).map_err(|_| "cannot encode hook input")?;
        if payload.len() > MAX_INPUT_BYTES {
            return Err("hook input exceeds 1 MiB".into());
        }
        let timeout = remaining.min(handler.timeout);
        match &handler.action {
            Action::Prompt(prompt) | Action::Agent(prompt) => {
                let agent = matches!(handler.action, Action::Agent(_));
                return invoke_model_hook(host, event, prompt, &payload, timeout, agent).await;
            }
            Action::Mcp {
                server,
                tool,
                input,
            } => {
                let server = self.mcp_hook_server(handler.dialect, server)?;
                let mut arguments = input.clone();
                expand_mcp_input(&mut arguments, fields)?;
                let response = tokio::time::timeout(
                    timeout,
                    host.call_imported_plugin_mcp(server, tool.clone(), arguments),
                )
                .await
                .map_err(|_| HookFailure::Timeout("MCP hook timed out"))?
                .map_err(|error| error.diagnostic_message().to_owned())?;
                let payload = response.payload.ok_or("MCP hook returned no result")?;
                if payload.get("isError").and_then(Value::as_bool) == Some(true) {
                    return Err("MCP tool reported a hook error".into());
                }
                if event.is_model_attempt() {
                    let billable = event == Event::PostModelRequest
                        && (fields["status"] == "completed"
                            || (fields["status"] == "failed" && fields["token_usage"].is_object()));
                    let units = if billable {
                        codex_model_hook_units(payload.get("structured_content"))?
                    } else {
                        0.0
                    };
                    return Ok(super::dispatch::HookOutput {
                        effects: Effects {
                            extra_codex_rollout_budget_units: units,
                            ..Default::default()
                        },
                        successful: true,
                        ..Default::default()
                    });
                }
                let content = payload
                    .get("content")
                    .and_then(Value::as_array)
                    .ok_or("MCP hook returned no content")?;
                let mut stdout = String::new();
                for text in content.iter().filter_map(|item| {
                    (item.get("type").and_then(Value::as_str) == Some("text"))
                        .then(|| item.get("text").and_then(Value::as_str))
                        .flatten()
                }) {
                    if stdout.len().saturating_add(text.len()).saturating_add(1) > MAX_OUTPUT_BYTES
                    {
                        return Err("MCP hook returned too much text".into());
                    }
                    if !stdout.is_empty() {
                        stdout.push('\n');
                    }
                    stdout.push_str(text);
                }
                return Ok(super::dispatch::decode_text(
                    handler.dialect,
                    event,
                    Some(0),
                    &stdout,
                    "",
                )?);
            }
            _ => {}
        }
        if prepared.policy.disabled || !sandbox::enabled() {
            return Err("process hooks require Agena process sandboxing".into());
        }
        let shell_environment = prepared
            .environment
            .as_ref()
            .filter(|_| uses_shell_environment(handler, event));
        let (program, args, env, input) = match &handler.action {
            Action::Command(command) => {
                let data = prepared
                    .data
                    .as_deref()
                    .ok_or("hook data directory is unavailable")?;
                let (program, args, environment) = self.command(
                    handler.dialect,
                    command,
                    &prepared.workspace_root,
                    data,
                    &handler.paths,
                    shell_environment,
                )?;
                (program, args, environment, payload.into_bytes())
            }
            Action::Http(hook) => {
                let args = [
                    "-q",
                    "--silent",
                    "--show-error",
                    "--noproxy",
                    "*",
                    "--proto",
                    "=http,https",
                    "--write-out",
                    "\n%{response_code}",
                    "--config",
                    "-",
                ];
                let input = hook.request(&payload, &self.environment)?;
                (
                    "curl".into(),
                    args.map(ToOwned::to_owned).into(),
                    HashMap::new(),
                    input,
                )
            }
            _ => unreachable!("non-process hook was already handled"),
        };
        let command = prepared.wrap(program, args, env, shell_environment, transcripts)?;
        let capture_limit = if handler.additional_context_limit.is_some() {
            MAX_CODEX_CONTEXT_OUTPUT_BYTES
        } else {
            MAX_OUTPUT_BYTES
        };
        if handler.mode != HandlerMode::Foreground {
            let binding = tokio::time::timeout_at(deadline, host.bind_imported_plugin_background())
                .await
                .map_err(|_| HookFailure::Timeout("background hook admission timed out"))?
                .map_err(|error| error.diagnostic_message().to_owned())?;
            let timeout = if handler.mode == HandlerMode::Rewake {
                handler.timeout
            } else {
                Duration::MAX
            };
            let rewake = handler.mode == HandlerMode::Rewake;
            let source = handler.source.clone();
            let plugin = self.plugin_id.clone();
            let once = once.take();
            let environment = shell_environment.cloned();
            let transcripts = transcripts.cloned();
            let context_limit = handler.additional_context_limit;
            let session = prepared.session;
            self.background.start(prepared.session, None, binding.cancellation, async move {
                let _environment = environment;
                let _transcripts = transcripts;
                let output = agena_process::output_with_optional_input_cancellable(
                    command, &input, timeout, capture_limit, None,
                ).await;
                match output {
                    Ok(output) => {
                        if output.status.success() && let Some(claim) = once {
                            claim.complete();
                        }
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        let wake = rewake && output.status.code() == Some(2);
                        let message = if wake {
                            let stderr = String::from_utf8_lossy(&output.stderr);
                            let text = if stderr.trim().is_empty() { stdout.trim() } else { stderr.trim() };
                            Some(if text.is_empty() { format!("{} hook exited with code 2", event.name()) } else { text.to_owned() })
                        } else {
                            super::dispatch::background_message(event, &stdout)
                        };
                        if let Some(mut text) = message {
                            if let Some(limit) = context_limit {
                                text = spill_codex_context(
                                    codex_hook_output_directory(&plugin, session), text, limit, 16 * 1024,
                                ).await;
                            }
                            if let Err(error) = binding.notices.deliver(text, wake).await {
                                tracing::warn!(%plugin, %source, %error, "could not deliver background hook output");
                            }
                        }
                    }
                    Err(error) => tracing::warn!(%plugin, %source, %error, "background hook process failed"),
                }
            })?;
            return Ok(super::dispatch::HookOutput::default());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        let timeout = remaining.min(handler.timeout);
        let cancellation = agena_plugin_sdk::current_cancellation_token();
        let output = match handler.action {
            Action::Command(_) => {
                agena_process::output_with_optional_input_cancellable(
                    command,
                    &input,
                    timeout,
                    capture_limit,
                    cancellation,
                )
                .await
            }
            Action::Http(_) => {
                agena_process::output_with_input_cancellable(
                    command,
                    &input,
                    timeout,
                    MAX_OUTPUT_BYTES,
                    cancellation,
                )
                .await
            }
            _ => unreachable!("non-process hook was already handled"),
        }
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::TimedOut => HookFailure::Timeout("hook process timed out"),
            kind => HookFailure::Other(format!("hook process failed: {kind}")),
        })?;
        let output = if matches!(handler.action, Action::Http(_)) {
            HttpHook::response(output)?
        } else {
            output
        };
        if event.is_worktree() {
            return Ok(super::dispatch::decode_worktree(
                event,
                output,
                matches!(handler.action, Action::Http(_)),
            )?);
        }
        let status = output.status;
        let decoded = super::dispatch::decode_output(handler.dialect, event, output)?;
        if handler.dialect == Dialect::Codex
            && handler.block_on_failure
            && !decoded.successful
            && decoded.effects.block.is_none()
            && decoded.effects.output.is_none()
        {
            return Err(format!("hook exited with {status}").into());
        }
        Ok(decoded)
    }

    fn mcp_hook_server(&self, dialect: Dialect, server: &str) -> Result<String, String> {
        let Some(scoped) = server.strip_prefix("plugin:") else {
            if dialect == Dialect::Codex && self.mcp_servers.contains(server) {
                return Ok(agent_plugin_mcp_server_name(&self.plugin_id, server));
            }
            return Ok(server.to_owned());
        };
        let (plugin, name) = scoped
            .split_once(':')
            .ok_or("invalid plugin MCP server name")?;
        if plugin != self.package_name || name.is_empty() {
            return Err("scoped MCP hook servers must belong to the imported plugin".into());
        }
        Ok(agent_plugin_mcp_server_name(&self.plugin_id, name))
    }

    fn command(
        &self,
        dialect: Dialect,
        command: &Command,
        project_root: &str,
        data: &Path,
        paths: &HookPathOverrides,
        shell_environment: Option<&ImportedHookEnvironment>,
    ) -> Result<(String, Vec<String>, HashMap<String, String>), String> {
        let mut env = HashMap::new();
        for name in &self.environment {
            env.insert(name.clone(), mcp::read_var(name)?);
        }
        let mut options = BTreeMap::new();
        if dialect == Dialect::Claude {
            options.extend(
                self.option_defaults
                    .iter()
                    .map(|(key, value)| (key.as_str(), value.clone())),
            );
            for (key, name) in &self.user_config_env {
                options.remove(key.as_str());
                if let Ok(value) = mcp::read_var(name) {
                    options.insert(key.as_str(), value);
                }
            }
            for (key, value) in &options {
                if mcp::valid_env_name(key) {
                    env.insert(
                        format!("CLAUDE_PLUGIN_OPTION_{}", key.to_ascii_uppercase()),
                        value.clone(),
                    );
                }
            }
        }
        let root = self
            .root
            .to_str()
            .ok_or("hook package path must be UTF-8")?;
        let data = data.to_str().ok_or("hook data path must be UTF-8")?;
        let plugin_root = paths.plugin_root.as_ref().map(|path| self.root.join(path));
        let project_dir = paths
            .project_dir
            .as_ref()
            .map(|path| Path::new(project_root).join(path));
        let mut variables = vec![
            ("PLUGIN_ROOT", root),
            ("PLUGIN_DATA", data),
            (
                "CLAUDE_PLUGIN_ROOT",
                plugin_root
                    .as_deref()
                    .and_then(Path::to_str)
                    .unwrap_or(root),
            ),
            ("CLAUDE_PLUGIN_DATA", data),
            (
                "CLAUDE_PROJECT_DIR",
                project_dir
                    .as_deref()
                    .and_then(Path::to_str)
                    .unwrap_or(project_root),
            ),
        ];
        if let Some(environment) = shell_environment {
            let path = environment
                .path()
                .to_str()
                .ok_or("hook environment path must be UTF-8")?;
            variables.push(("CLAUDE_ENV_FILE", path));
        }
        let (program, args) = match command {
            Command::Exec { program, args } => (
                expand_exec(program, &variables, &options)?,
                args.iter()
                    .map(|arg| expand_exec(arg, &variables, &options))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            Command::Shell { script, shell } => shell_argv(dialect, shell, script, &variables),
        };
        for (key, value) in variables {
            env.insert(key.into(), value.into());
        }
        Ok((program, args, env))
    }
}

fn codex_hook_output_directory(plugin: &str, session: i64) -> PathBuf {
    agena_storage_layout::StorageLayout::current()
        .temp_dir()
        .join("hook_outputs")
        .join(plugin)
        .join(session.to_string())
}

#[cfg(test)]
#[tokio::test]
async fn codex_context_spill_retains_complete_output_and_honors_zero_limit() {
    let directory = tempfile::tempdir().unwrap();
    let text = format!("BEGIN {} END", "多字节内容".repeat(80));
    let unchanged = spill_codex_context(directory.path().into(), text.clone(), 0, 8_192).await;
    assert_eq!(unchanged, text);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);

    let preview = spill_codex_context(directory.path().into(), text.clone(), 90, 8_192).await;
    let (_, saved) = preview.split_once("\n\nFull hook output saved to: ").unwrap();
    assert!(preview.starts_with("BEGIN "));
    assert!(preview.contains(" END"));
    assert!(preview.len() <= 360);
    assert_eq!(std::fs::read_to_string(saved).unwrap(), text);
}

async fn spill_codex_context(
    directory: PathBuf,
    text: String,
    tokens: usize,
    budget: usize,
) -> String {
    // Codex approximates a token with four bytes. Agena also limits the
    // aggregate hook context for the phase.
    let limit = if tokens == 0 {
        budget
    } else {
        tokens.saturating_mul(4).min(budget)
    };
    if text.len() <= limit {
        return text;
    }
    match DATA_DIRECTORIES
        .run(move || {
            use std::io::Write;

            let saved = (|| -> std::io::Result<PathBuf> {
                agena_storage_layout::secure_directory(&directory)?;
                let mut output = tempfile::Builder::new()
                    .suffix(".txt")
                    .tempfile_in(&directory)?;
                output.write_all(text.as_bytes())?;
                let (_, path) = output.keep()?;
                Ok(path)
            })();
            let footer = match saved {
                Ok(path) => format!("\n\nFull hook output saved to: {}", path.display()),
                Err(error) => {
                    tracing::warn!(%error, "could not save oversized Codex hook output");
                    String::new()
                }
            };
            let budget = limit.saturating_sub(footer.len());
            let marker = "\n…\n";
            let remainder = budget.saturating_sub(marker.len());
            let preview = if remainder == 0 {
                String::new()
            } else {
                let head = text.floor_char_boundary(remainder / 2);
                let tail =
                    text.ceil_char_boundary(text.len().saturating_sub(remainder - remainder / 2));
                format!("{}{marker}{}", &text[..head], &text[tail..])
            };
            format!("{preview}{footer}")
        })
        .await
    {
        Ok(preview) => preview,
        Err(error) => {
            tracing::warn!(%error, "could not prepare Codex hook output");
            String::new()
        }
    }
}

struct PreparedHook {
    session: i64,
    cwd: String,
    workspace_root: String,
    policy: SandboxPolicy,
    data: Option<PathBuf>,
    execution: Option<HostProcessExecution>,
    environment: Option<ImportedHookEnvironment>,
    transcripts: BTreeMap<HookTranscriptFormat, ImportedHookTranscripts>,
}

#[derive(serde::Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(in crate::agent_plugin) struct ModProcessOptions {
    cwd: Option<String>,
    env: HashMap<String, String>,
    stdin: String,
    timeout_ms: Option<u64>,
}

pub(in crate::agent_plugin) async fn run_mod_process(
    host: &dyn HostClient,
    argv: Vec<String>,
    options: ModProcessOptions,
    transcripts: Option<&ImportedHookTranscripts>,
    cancellation: tokio_util::sync::CancellationToken,
) -> Result<Value, String> {
    let ModProcessOptions {
        cwd,
        env,
        stdin,
        timeout_ms,
    } = options;
    let mut argv = argv.into_iter();
    let program = argv
        .next()
        .filter(|program| !program.is_empty())
        .ok_or("Claude mod process requires a program in argv[0]")?;
    if !sandbox::enabled() {
        return Err("Claude mod processes require Agena process sandboxing".into());
    }
    let timeout = Duration::from_millis(timeout_ms.unwrap_or(30_000).min(600_000));
    let deadline = Instant::now() + timeout;
    let prepared = tokio::time::timeout_at(
        deadline,
        host.prepare_process_sandbox(HostProcessResources {
            cwd,
            require_sandbox: true,
            ..Default::default()
        }),
    )
    .await
    .map_err(|_| "Claude mod process sandbox preparation timed out")?
    .map_err(|error| error.diagnostic_message().to_owned())?;
    let mut policy: SandboxPolicy = serde_json::from_value(prepared.policy)
        .map_err(|_| "invalid host process policy".to_owned())?;
    if policy.disabled {
        return Err("Claude mod processes require Agena process sandboxing".into());
    }
    add_transcript_paths_to_policy(&mut policy, transcripts)?;
    let mut args = argv.collect::<Vec<_>>();
    if Path::new(&program)
        .file_stem()
        .is_some_and(|name| name.eq_ignore_ascii_case("git"))
    {
        let hooks_path = if cfg!(windows) { "NUL" } else { "/dev/null" };
        args.splice(
            ..0,
            ["-c".to_owned(), format!("core.hooksPath={hooks_path}")],
        );
    }
    // Explicit overlays are combined with the sandbox's normal sanitized base.
    let ambient = std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .collect();
    let mut environment = sandbox::sanitize_environment(&ambient);
    environment.extend(env);
    let command = wrap_process(program, args, environment, &prepared.cwd, &policy)?;
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err("Claude mod process timed out before launch".into());
    }
    let result = agena_process::capture_output(
        command,
        stdin.as_bytes(),
        remaining,
        MAX_OUTPUT_BYTES,
        Some(cancellation),
    )
    .await
    .map_err(|error| format!("Claude mod process failed: {}", error.kind()))?;
    let stream_text = |bytes: &[u8], truncated| {
        // A cut in the middle of a UTF-8 character does not add a replacement character.
        if truncated
            && let Err(error) = std::str::from_utf8(bytes)
            && error.error_len().is_none()
        {
            return String::from_utf8_lossy(&bytes[..error.valid_up_to()]).into_owned();
        }
        String::from_utf8_lossy(bytes).into_owned()
    };
    Ok(json!({
        "exitCode": result.output.status.code().unwrap_or(1),
        "stdout": stream_text(&result.output.stdout, result.stdout_truncated),
        "stderr": stream_text(&result.output.stderr, result.stderr_truncated),
        "isStdoutTruncated": result.stdout_truncated,
        "isStderrTruncated": result.stderr_truncated,
    }))
}

fn add_transcript_paths_to_policy(
    policy: &mut SandboxPolicy,
    transcripts: Option<&ImportedHookTranscripts>,
) -> Result<(), String> {
    for file in transcripts
        .into_iter()
        .flat_map(ImportedHookTranscripts::files)
    {
        let path = file.path();
        policy.private_paths.push(
            path.parent()
                .ok_or("private hook file has no parent directory")?
                .to_path_buf(),
        );
        policy.readonly_paths.push(path.to_path_buf());
    }
    Ok(())
}

impl PreparedHook {
    async fn load_transcripts(
        &mut self,
        host: &dyn HostClient,
        event: Event,
        dialect: Dialect,
        deadline: Instant,
    ) {
        let format = dialect.into();
        let std::collections::btree_map::Entry::Vacant(entry) = self.transcripts.entry(format)
        else {
            return;
        };
        let result = tokio::time::timeout_at(
            deadline,
            host.imported_hook_transcripts(format, transcript_scope(event, dialect), None),
        )
        .await;
        let error = match result {
            Ok(Ok(files)) => {
                entry.insert(files);
                return;
            }
            Ok(Err(error)) => error.diagnostic_message().to_owned(),
            Err(_) => "hook transcript preparation timed out".into(),
        };
        entry.insert(ImportedHookTranscripts::default());
        host.log(
            LogLevel::Warn,
            error,
            json!({"hook":event.name(),"component":"transcript_path"}),
        )
        .await;
    }

    fn wrap(
        &self,
        program: String,
        args: Vec<String>,
        env: HashMap<String, String>,
        environment: Option<&ImportedHookEnvironment>,
        transcripts: Option<&ImportedHookTranscripts>,
    ) -> Result<tokio::process::Command, String> {
        let policy = self.process_policy(environment, transcripts)?;
        wrap_process(program, args, env, &self.cwd, &policy)
    }

    fn process_policy(
        &self,
        environment: Option<&ImportedHookEnvironment>,
        transcripts: Option<&ImportedHookTranscripts>,
    ) -> Result<Cow<'_, SandboxPolicy>, String> {
        let mut policy = Cow::Borrowed(&self.policy);
        for (file, writable) in environment.into_iter().map(|file| (file, true)).chain(
            transcripts
                .into_iter()
                .flat_map(ImportedHookTranscripts::files)
                .map(|file| (file, false)),
        ) {
            let path = file.path();
            let policy = policy.to_mut();
            policy.private_paths.push(
                path.parent()
                    .ok_or("private hook file has no parent directory")?
                    .to_path_buf(),
            );
            if writable {
                policy.readwrite_paths.push(path.to_path_buf());
            } else {
                policy.readonly_paths.push(path.to_path_buf());
            }
        }
        Ok(policy)
    }
}

fn wrap_process(
    program: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    cwd: &str,
    policy: &SandboxPolicy,
) -> Result<tokio::process::Command, String> {
    let path = env
        .get("PATH")
        .map(std::ffi::OsString::from)
        .or_else(|| std::env::var_os("PATH"));
    let executable =
        which::which_in(program, path, cwd).map_err(|_| "hook executable was not found")?;
    let mut command = tokio::process::Command::new(executable);
    command.current_dir(cwd).args(args).env_clear().envs(env);
    sandbox::wrap_tokio_command(&mut command, policy)
        .map_err(|error| format!("cannot prepare the hook launcher: {}", error.kind()))?;
    Ok(command)
}

fn expand_mcp_input(input: &mut Value, fields: &Value) -> Result<(), String> {
    match input {
        Value::Object(object) => {
            for value in object.values_mut() {
                expand_mcp_input(value, fields)?;
            }
        }
        Value::Array(array) => {
            for value in array {
                expand_mcp_input(value, fields)?;
            }
        }
        Value::String(value) => {
            let mut output = String::new();
            let mut rest = value.as_str();
            while let Some((before, tail)) = rest.split_once("${") {
                let (path, after) = tail
                    .split_once('}')
                    .ok_or("unterminated MCP hook substitution")?;
                let value = path
                    .split('.')
                    .try_fold(fields, |node, field| {
                        node.get(field).or_else(|| {
                            field
                                .parse::<usize>()
                                .ok()
                                .and_then(|index| node.get(index))
                        })
                    })
                    .ok_or_else(|| format!("unknown MCP hook field {path:?}"))?;
                output.push_str(before);
                if let Some(value) = value.as_str() {
                    output.push_str(value);
                } else {
                    output.push_str(&value.to_string());
                }
                if output.len() > MAX_INPUT_BYTES {
                    return Err("MCP hook arguments are too large".into());
                }
                rest = after;
            }
            output.push_str(rest);
            *value = output;
        }
        _ => {}
    }
    Ok(())
}

async fn invoke_model_hook(
    host: &dyn HostClient,
    event: Event,
    prompt: &str,
    payload: &str,
    timeout: Duration,
    agent: bool,
) -> Result<super::dispatch::HookOutput, HookFailure> {
    let request = if prompt.contains("$ARGUMENTS") {
        prompt.replace("$ARGUMENTS", payload)
    } else {
        format!("{prompt}\n\n{payload}")
    };
    if request.len() > 120 * 1024 {
        return Err("prompt hook request is too large".into());
    }
    let evaluate = async {
        if agent {
            host.evaluate_imported_plugin_agent(request).await
        } else {
            host.evaluate_imported_plugin_prompt(request).await
        }
    };
    let answer = tokio::time::timeout(timeout, evaluate)
        .await
        .map_err(|_| HookFailure::Timeout("model hook timed out"))?
        .map_err(|error| error.diagnostic_message().to_owned())?;
    Ok(prompt_decision(event, &answer, agent)?)
}

fn prompt_decision(
    event: Event,
    answer: &str,
    agent: bool,
) -> Result<super::dispatch::HookOutput, String> {
    #[derive(serde::Deserialize)]
    struct Decision {
        ok: bool,
        reason: Option<String>,
        #[serde(default)]
        impossible: bool,
    }
    let decision: Decision =
        serde_json::from_str(answer.trim()).map_err(|_| "invalid prompt hook JSON decision")?;
    let mut effects = Effects::default();
    if !decision.ok {
        let mut reason = decision
            .reason
            .filter(|reason| !reason.trim().is_empty())
            .ok_or("prompt hook decision is missing its reason")?;
        reason.truncate(reason.floor_char_boundary(reason.len().min(16 * 1024)));
        match event {
            Event::PreModelSwitch => {
                effects.model_switch.decision =
                    Some(agena_plugin_sdk::ImportedModelSwitchDecision::Deny(reason));
            }
            Event::PreToolUse
            | Event::UserPromptSubmit
            | Event::UserPromptExpansion
            | Event::TaskCreated
            | Event::TaskCompleted => {
                effects.block = Some(reason);
            }
            Event::Stop | Event::SubagentStop if agent || !decision.impossible => {
                effects.block = Some(reason);
            }
            Event::PostToolUse | Event::PostToolUseFailure => effects.context.push(reason),
            _ => {}
        }
    }
    Ok(super::dispatch::HookOutput {
        system_message: None,
        effects,
        successful: decision.ok,
    })
}

fn expand_exec(
    value: &str,
    variables: &[(&str, &str)],
    options: &BTreeMap<&str, String>,
) -> Result<String, String> {
    let mut output = String::new();
    let mut remaining = value;
    while let Some((prefix, rest)) = remaining.split_once("${") {
        let Some((name, suffix)) = rest.split_once('}') else {
            break;
        };
        output.push_str(prefix);
        if let Some((_, replacement)) = variables.iter().find(|(key, _)| *key == name) {
            output.push_str(replacement);
        } else if let Some(key) = name.strip_prefix("user_config.") {
            output.push_str(
                options
                    .get(key)
                    .ok_or_else(|| format!("hook user_config option {key:?} is unavailable"))?,
            );
        } else {
            output.push_str("${");
            output.push_str(name);
            output.push('}');
        }
        remaining = suffix;
    }
    output.push_str(remaining);
    Ok(output)
}

fn shell_argv(
    dialect: Dialect,
    shell: &Shell,
    script: &str,
    variables: &[(&str, &str)],
) -> (String, Vec<String>) {
    if *shell == Shell::PowerShell
        || (cfg!(windows) && *shell == Shell::Default && dialect == Dialect::Claude)
    {
        let script = variables
            .iter()
            .fold(script.to_owned(), |script, (key, _)| {
                script.replace(&format!("${{{key}}}"), &format!("${{env:{key}}}"))
            });
        let script = if dialect == Dialect::Codex {
            preserve_powershell_hook_exit_code(script)
        } else {
            script
        };
        let program = if cfg!(windows) {
            "powershell.exe"
        } else {
            "pwsh"
        };
        return (
            program.into(),
            vec![
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                script,
            ],
        );
    }
    if cfg!(windows) && *shell == Shell::Default {
        let script = variables
            .iter()
            .fold(script.to_owned(), |script, (key, _)| {
                script.replace(&format!("${{{key}}}"), &format!("%{key}%"))
            });
        return (
            "cmd.exe".into(),
            vec!["/D".into(), "/S".into(), "/C".into(), script],
        );
    }
    let program = if *shell == Shell::Bash || dialect == Dialect::Claude {
        "bash"
    } else {
        "sh"
    };
    (program.into(), vec!["-c".into(), script.into()])
}

fn preserve_powershell_hook_exit_code(script: String) -> String {
    let mut body = script.trim_start();
    while let Some(rest) = if body.starts_with('#') {
        body.split_once('\n').map(|(_, rest)| rest)
    } else if body.starts_with("<#") {
        body.split_once("#>").map(|(_, rest)| rest)
    } else {
        None
    } {
        body = rest.trim_start();
    }
    let mut words = body.split(|character: char| {
        character.is_whitespace() || matches!(character, '{' | '(' | ')' | ';' | '<' | '>')
    });
    let first = words.next().unwrap_or_default();
    let named_block = |word: &str| {
        ["begin", "process", "end", "clean", "dynamicparam"]
            .iter()
            .any(|name| word.eq_ignore_ascii_case(name))
    };
    // Named blocks must remain the outermost syntax of the command script.
    if body.contains('{')
        && (named_block(first)
            || ((first.eq_ignore_ascii_case("param")
                || first.eq_ignore_ascii_case("using")
                || body.starts_with('['))
                && words.any(named_block)))
    {
        return script;
    }
    format!("{script}\nif ($?) {{ exit 0 }}\nif ($LASTEXITCODE) {{ exit $LASTEXITCODE }}\nexit 1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_powershell_preserves_native_exit_codes_and_named_blocks() {
        let simple = "cmd.exe /c 'exit /b 2'";
        let (_, arguments) = shell_argv(Dialect::Codex, &Shell::PowerShell, simple, &[]);
        let script = arguments.last().unwrap();
        assert!(script.starts_with(simple));
        assert!(script.contains("if ($?) { exit 0 }"));
        assert!(script.contains("if ($LASTEXITCODE) { exit $LASTEXITCODE }"));
        for named in [
            "end { exit 2 }",
            "<# comment #>\n[CmdletBinding()]\nparam()\nEND { exit 2 }",
        ] {
            let (_, arguments) = shell_argv(Dialect::Codex, &Shell::PowerShell, named, &[]);
            assert_eq!(arguments.last().unwrap(), named);
        }
    }

    #[test]
    fn claude_setup_matches_the_trigger_and_ignores_decisions() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().canonicalize().unwrap();
        std::fs::create_dir(root.join(".claude-plugin")).unwrap();
        std::fs::write(
            root.join(".claude-plugin/plugin.json"),
            r#"{"name":"setup-test"}"#,
        )
        .unwrap();
        std::fs::create_dir(root.join("hooks")).unwrap();
        std::fs::write(root.join("hooks/hooks.json"), json!({
            "hooks": {"Setup":[
                {"matcher":"init","hooks":[{"type":"command","command":"probe","shell":"bash","async":true}]},
                {"matcher":"maintenance","hooks":[{"type":"command","command":"check","args":[]}]},
                {"hooks":[{"type":"mcp_tool","server":"test","tool":"unsupported"}]},
            ]},
        }).to_string()).unwrap();
        let package = crate::agent_plugin::load_agent_plugin(&root).unwrap();
        let handlers = &package.hooks.0[&Event::Setup];
        assert_eq!(handlers.len(), 2);
        assert_eq!(package.diagnostics.len(), 1);
        assert!(handlers[0].matcher.matches("init"));
        assert!(!handlers[0].matcher.matches("maintenance"));
        assert!(handlers[1].matcher.matches("maintenance"));
        assert_eq!(handlers[0].mode, HandlerMode::Foreground);
        assert!(uses_shell_environment(&handlers[0], Event::Setup));
        let output = super::super::dispatch::decode_text(
            Dialect::Claude,
            Event::Setup,
            Some(2),
            r#"{"continue":false,"decision":"block","systemMessage":"ignored"}"#,
            "also ignored",
        )
        .unwrap();
        assert!(output.effects.block.is_none());
        assert!(output.effects.context.is_empty());
        assert!(output.system_message.is_none());
    }

    #[test]
    fn transcript_files_are_read_only_and_bound_to_the_event() {
        use agena_plugin_sdk::host_api::ImportedHookFile;
        use std::sync::Arc;

        let file = |name| {
            let directory = Arc::new(tempfile::tempdir().unwrap());
            ImportedHookFile::new(directory.path().join(name), directory)
        };
        let parent = file("parent.jsonl");
        let child = file("child.jsonl");
        let files = ImportedHookTranscripts {
            transcript: Some(parent.clone()),
            agent: Some(child.clone()),
        };
        let prepared = PreparedHook {
            session: 1,
            cwd: String::new(),
            workspace_root: String::new(),
            policy: SandboxPolicy::default(),
            data: None,
            execution: None,
            environment: None,
            transcripts: BTreeMap::new(),
        };
        let policy = prepared.process_policy(None, Some(&files)).unwrap();
        assert_eq!(policy.readonly_paths, [parent.path(), child.path()]);
        assert_eq!(
            policy.private_paths,
            [
                parent.path().parent().unwrap(),
                child.path().parent().unwrap()
            ]
        );
        assert!(policy.readwrite_paths.is_empty());
        assert!(prepared.policy.private_paths.is_empty());
        let mut mod_policy = SandboxPolicy::default();
        add_transcript_paths_to_policy(&mut mod_policy, Some(&files)).unwrap();
        assert_eq!(mod_policy.readonly_paths, [parent.path(), child.path()]);
        assert_eq!(
            mod_policy.private_paths,
            [
                parent.path().parent().unwrap(),
                child.path().parent().unwrap()
            ]
        );
        assert!(mod_policy.readwrite_paths.is_empty());
        assert_eq!(
            transcript_scope(Event::SubagentStop, Dialect::Codex),
            HookTranscriptScope::ParentAndAgent
        );
        assert_eq!(
            transcript_scope(Event::SubagentStop, Dialect::Claude),
            HookTranscriptScope::ParentAndAgent
        );
        assert_eq!(
            transcript_scope(Event::SubagentStart, Dialect::Codex),
            HookTranscriptScope::Current
        );
        assert_eq!(
            transcript_scope(Event::SubagentStart, Dialect::Claude),
            HookTranscriptScope::Parent
        );
    }

    #[test]
    fn deepseek_command_defaults_and_claude_paths_are_projected() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().canonicalize().unwrap();
        std::fs::write(
            root.join("package.json"),
            json!({
                "name": "@fixture/hooks", "dsh": {"bundle": {"patch": "bundle.yml"}},
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            root.join("bundle.yml"),
            r#"
- insert:
  - id: hooks
    name: '@deepseek-ai/dsh-hooks-claude-code'
    config:
      configPath: hooks.json
      pluginRoot: extension
      projectDir: nested project
"#,
        )
        .unwrap();
        std::fs::write(root.join("hooks.json"), json!({
            "hooks": {"SessionStart": [{"hooks": [
                {"command":"probe", "args":["${CLAUDE_PLUGIN_ROOT}/check.sh", "${CLAUDE_PROJECT_DIR}"]}
            ]}]},
        }).to_string()).unwrap();
        let package = crate::agent_plugin::load_agent_plugin(&root).unwrap();
        assert!(package.diagnostics.is_empty(), "{:?}", package.diagnostics);
        let runner = HookRunner::new(
            &package,
            &"deepseek.fixture".parse().unwrap(),
            &[],
            &BTreeMap::new(),
        );
        let [handler] = runner.hooks.0[&Event::SessionStart].as_slice() else {
            panic!("expected one handler")
        };
        let Action::Command(command) = &handler.action else {
            panic!("expected a command handler")
        };
        let workspace = root.join("workspace");
        let (program, args, env) = runner
            .command(
                handler.dialect,
                command,
                workspace.to_str().unwrap(),
                &root,
                &handler.paths,
                None,
            )
            .unwrap();
        assert_eq!(program, "probe");
        assert_eq!(
            args[0],
            format!("{}/check.sh", root.join("extension").display())
        );
        assert_eq!(
            args[1],
            workspace.join("nested project").display().to_string()
        );
        assert_eq!(env["CLAUDE_PROJECT_DIR"], args[1]);
    }
}
