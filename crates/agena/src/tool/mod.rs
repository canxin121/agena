mod apply_patch;
mod ask_user;
mod bash;
mod builtins;
mod catalog;
mod cron;
mod definition;
mod glob;
mod grep;
mod mcp;
mod monitor;
mod monitor_tool;
mod orchestrator;
mod plan;
mod read;
mod result;
mod shell;
mod skill;
mod subtask;
mod task;
mod todo_write;
mod tool_search;
mod truncation;
mod view_file;
mod web_fetch;
mod web_search;
mod worktree;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use thiserror::Error;

use crate::agent::Agent;
use crate::message::{
    AskUserToolInput, BuiltinToolInput, BuiltinToolOutput, CustomToolOutput, Message, PartContent,
    StructuredObject, ToolExecutionPart, ToolInvocation, ToolOutput,
};
use crate::permission::{
    AccessKind, PermissionAction, PermissionDecision, PermissionRuleStore, PermissionRuntime,
    PermissionRuntimeDecision,
};
use crate::plugin::{
    PluginHost, ToolAfterInput as PluginToolAfterInput,
    ToolBeforeInput as PluginToolBeforeInput, ToolDefinitionInput as PluginToolDefinitionInput,
    ToolFailureInput as PluginToolFailureInput, ToolInvokeInput as PluginToolInvokeInput,
    ToolSource as SdkToolSource,
    sdk::{ShellEnvInput as PluginShellEnvInput, ToolBehavior as SdkToolBehavior},
};

pub use apply_patch::{AppliedFileChange, ApplyPatchExecution};
pub use catalog::{ModelToolProfile, ToolAvailability, ToolCatalog};
pub use definition::{ToolBehavior, ToolDefinition, ToolLoadPriority, ToolSource};
pub use plan::{PlanRegistry, registry_for_executor as plan_registry_for_executor};
pub use shell::{ExecutionPolicy, ShellError, ShellOutput, ShellRequest};
pub use worktree::{WorktreeRegistry, registry_for_executor as worktree_registry_for_executor};
pub use monitor::{
    MonitorError, MonitorRead, MonitorRegistry, MonitorService, MonitorStart, MonitorStopOutcome,
    ReadParams as MonitorReadParams, StartParams as MonitorStartParams,
};
pub use result::{BuiltinExecution, ToolExecutionView, ToolInvocationExecution};
pub use subtask::{
    InMemorySubtaskSessionManager, SubtaskSession, SubtaskSessionError, SubtaskSessionManager,
    SubtaskSessionRequest,
};
pub use truncation::{ToolOutputTruncationPolicy, ToolOutputTruncator};

/// Stable id used to register the built-in plugin with a [`PluginHost`].
pub fn builtins_plugin_id() -> &'static str {
    builtins::BUILTIN_PLUGIN_ID
}

/// Construct the in-process plugin that exposes every built-in tool. Pass it
/// to [`agena_plugin_host::PluginHostBuilder::register_static`] before
/// calling `.build()`.
pub fn new_builtins_plugin() -> impl crate::plugin::sdk::Plugin {
    builtins::BuiltinPlugin::new()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPermissionCheck {
    pub action: PermissionAction,
    pub decision: PermissionDecision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedToolInvocation {
    pub invocation: ToolInvocation,
    pub title_override: Option<String>,
    pub metadata: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PermissionExecutionMode {
    Enforced,
    Bypassed,
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct BuiltinExecutionContext {
    pub session_id: Option<i64>,
    pub call_id: Option<i64>,
}

#[derive(Debug)]
pub enum PermissionedBuiltinExecution {
    Executed(BuiltinExecution),
    Pending(crate::permission::PendingPermission),
}

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("permission confirmation required: {0}")]
    PermissionAsk(String),
    #[error("user input required")]
    UserInputRequired(AskUserToolInput),
    #[error("invalid patch: {0}")]
    InvalidPatch(String),
    #[error("invalid tool input: {0}")]
    InvalidInput(String),
    #[error("invalid glob pattern: {0}")]
    InvalidGlobPattern(#[from] globset::Error),
    #[error("invalid regex pattern: {0}")]
    InvalidRegexPattern(#[from] regex::Error),
    #[error("shell error: {0}")]
    Shell(#[from] ShellError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plugin error: {0}")]
    Plugin(String),
    #[error("unknown tool: {0}")]
    UnknownTool(String),
    #[error("unsupported tool invocation in executor: {0}")]
    UnsupportedInvocation(String),
}

#[derive(Clone)]
pub struct ToolExecutor {
    workspace_root: PathBuf,
    agent: Agent,
    model_id: Option<String>,
    subtask_manager: Arc<dyn SubtaskSessionManager>,
    monitor_registry: Option<Arc<dyn MonitorService>>,
    truncator: ToolOutputTruncator,
    sandbox_policy: ExecutionPolicy,
    plugins: Arc<PluginHost>,
    mcp_manager: Option<Arc<agena_mcp_client::McpConnectionManager>>,
    web_search_backend: crate::config::WebSearchBackend,
    plan_registry: Option<plan::PlanRegistry>,
    skills_manager: Option<Arc<agena_skills::SkillsManager>>,
    worktree_registry: Option<worktree::WorktreeRegistry>,
    scheduler: Option<Arc<agena_scheduler::Scheduler>>,
    permission_mode: PermissionExecutionMode,
}

impl ToolExecutor {
    pub fn new(workspace_root: impl Into<PathBuf>, agent: Agent) -> Self {
        Self::with_sandbox_policy(
            workspace_root,
            agent,
            ExecutionPolicy::workspace_write(),
        )
    }

    pub fn with_sandbox_policy(
        workspace_root: impl Into<PathBuf>,
        agent: Agent,
        sandbox_policy: ExecutionPolicy,
    ) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            agent,
            model_id: None,
            subtask_manager: Arc::new(InMemorySubtaskSessionManager::new()),
            monitor_registry: monitor::default_registry(),
            truncator: ToolOutputTruncator::default(),
            sandbox_policy,
            plugins: PluginHost::new_empty(),
            mcp_manager: None,
            web_search_backend: crate::config::WebSearchBackend::DuckDuckGoHtml,
            plan_registry: None,
            skills_manager: None,
            worktree_registry: None,
            scheduler: None,
            permission_mode: PermissionExecutionMode::Enforced,
        }
    }

    pub fn with_subtask_manager(mut self, manager: Arc<dyn SubtaskSessionManager>) -> Self {
        self.subtask_manager = manager;
        self
    }

    pub fn with_monitor_registry(mut self, registry: Arc<dyn MonitorService>) -> Self {
        self.monitor_registry = Some(registry);
        self
    }

    pub fn without_monitor_registry(mut self) -> Self {
        self.monitor_registry = None;
        self
    }

    pub fn with_model_id(mut self, model_id: impl Into<String>) -> Self {
        self.model_id = Some(model_id.into());
        self
    }

    pub fn with_plugin_manager(mut self, manager: Arc<PluginHost>) -> Self {
        self.plugins = manager;
        self
    }

    pub fn with_mcp_manager(
        mut self,
        manager: Arc<agena_mcp_client::McpConnectionManager>,
    ) -> Self {
        self.mcp_manager = Some(manager);
        self
    }

    pub fn mcp_manager(&self) -> Option<&Arc<agena_mcp_client::McpConnectionManager>> {
        self.mcp_manager.as_ref()
    }

    pub fn with_web_search_backend(
        mut self,
        backend: crate::config::WebSearchBackend,
    ) -> Self {
        self.web_search_backend = backend;
        self
    }

    pub fn web_search_backend(&self) -> crate::config::WebSearchBackend {
        self.web_search_backend.clone()
    }

    pub fn with_plan_registry(mut self, reg: plan::PlanRegistry) -> Self {
        self.plan_registry = Some(reg);
        self
    }

    pub fn plan_registry(&self) -> Option<&plan::PlanRegistry> {
        self.plan_registry.as_ref()
    }

    pub fn with_skills_manager(mut self, mgr: Arc<agena_skills::SkillsManager>) -> Self {
        self.skills_manager = Some(mgr);
        self
    }

    pub fn skills_manager(&self) -> Option<&Arc<agena_skills::SkillsManager>> {
        self.skills_manager.as_ref()
    }

    pub fn with_worktree_registry(mut self, reg: worktree::WorktreeRegistry) -> Self {
        self.worktree_registry = Some(reg);
        self
    }

    pub fn worktree_registry(&self) -> Option<&worktree::WorktreeRegistry> {
        self.worktree_registry.as_ref()
    }

    pub fn with_scheduler(mut self, scheduler: Arc<agena_scheduler::Scheduler>) -> Self {
        self.scheduler = Some(scheduler);
        self
    }

    pub fn scheduler(&self) -> Option<&Arc<agena_scheduler::Scheduler>> {
        self.scheduler.as_ref()
    }

    /// Refuse mutating invocations while plan mode is active for this
    /// session.  Returns Ok when the invocation is safe (read-only) or
    /// when plan mode is off.  Returns `PermissionDenied` otherwise.
    pub fn enforce_plan_mode_for(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
    ) -> Result<(), ToolError> {
        let Some(reg) = self.plan_registry() else {
            return Ok(());
        };
        if !reg.read().contains_key(&session_id) {
            return Ok(());
        }
        // In plan mode.  Allow read-only builtins (and plan tools
        // themselves); refuse everything else.
        if let Some(builtin) = invocation.as_builtin() {
            let name = crate::permission::builtin_name(&builtin);
            let allowed = matches!(
                name,
                "read" | "view_file" | "glob" | "grep" | "tool_search" | "todo_write"
                    | "ask_user" | "monitor" | "web_fetch" | "web_search"
                    | "enter_plan_mode" | "exit_plan_mode"
            );
            if allowed {
                return Ok(());
            }
            return Err(ToolError::PermissionDenied(format!(
                "tool '{name}' is blocked in plan mode; call exit_plan_mode first"
            )));
        }
        Err(ToolError::PermissionDenied(
            "non-builtin tools are blocked in plan mode; call exit_plan_mode first".to_string(),
        ))
    }

    pub fn with_truncation_policy(mut self, policy: ToolOutputTruncationPolicy) -> Self {
        self.truncator = ToolOutputTruncator::new(policy);
        self
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn agent(&self) -> &Agent {
        &self.agent
    }

    pub fn sandbox_policy(&self) -> &ExecutionPolicy {
        &self.sandbox_policy
    }

    pub fn subtask_manager(&self) -> &Arc<dyn SubtaskSessionManager> {
        &self.subtask_manager
    }

    pub fn monitor_registry(&self) -> Option<&Arc<dyn MonitorService>> {
        self.monitor_registry.as_ref()
    }

    pub fn plugin_manager(&self) -> &Arc<PluginHost> {
        &self.plugins
    }

    pub fn tool_catalog(&self) -> ToolCatalog {
        ToolCatalog::for_model(self.model_id.as_deref())
    }

    pub fn available_builtins(&self) -> Vec<ToolAvailability> {
        let catalog = self.tool_catalog();
        vec![
            BuiltinToolInput::Bash(crate::message::BashToolInput {
                command: String::new(),
                description: String::new(),
                timeout_ms: None,
                workdir: None,
            }),
            BuiltinToolInput::Read(crate::message::ReadToolInput {
                file_path: String::new(),
                offset: None,
                limit: None,
            }),
            BuiltinToolInput::ViewFile(crate::message::ViewFileToolInput {
                path: String::new(),
            }),
            BuiltinToolInput::ApplyPatch(crate::message::ApplyPatchToolInput {
                patch: String::new(),
            }),
            BuiltinToolInput::Glob(crate::message::GlobToolInput {
                pattern: String::new(),
                path: None,
            }),
            BuiltinToolInput::Grep(crate::message::GrepToolInput {
                pattern: String::new(),
                path: None,
                include: None,
            }),
            BuiltinToolInput::Task(crate::message::TaskToolInput {
                description: String::new(),
                prompt: String::new(),
                subagent_type: crate::message::TaskSubagentType::Explore,
                task_id: None,
                command: None,
            }),
            BuiltinToolInput::ToolSearch(crate::message::ToolSearchToolInput {
                query: String::new(),
                load: Vec::new(),
                limit: None,
            }),
            BuiltinToolInput::TodoWrite(crate::message::TodoWriteToolInput { items: Vec::new() }),
            BuiltinToolInput::AskUser(crate::message::AskUserToolInput {
                questions: Vec::new(),
            }),
            BuiltinToolInput::Monitor(crate::message::MonitorToolInput::List {}),
        ]
        .into_iter()
        .map(|input| catalog.availability_for_input(&self.agent, &input))
        .collect()
    }

    fn catalogued_tools(&self) -> Vec<ToolDefinition> {
        let catalog = self.tool_catalog();
        let mut definitions = self
            .plugins
            .tool_entries()
            .filter(|entry| entry.plugin_name != builtins::BUILTIN_PLUGIN_ID)
            .filter_map(|entry| {
                let behavior = sdk_to_local_behavior(entry.decl.behavior);
                catalog.is_behavior_enabled(behavior).then(|| {
                    ToolDefinition::plugin(
                        entry.exposed_name.clone(),
                        entry.decl.description.clone().unwrap_or_default(),
                        entry.decl.input_schema.clone(),
                        behavior,
                        entry.plugin_name.clone(),
                    )
                })
            })
            .collect::<Vec<_>>();

        let plugin_names = definitions
            .iter()
            .map(|definition| definition.name.clone())
            .collect::<std::collections::HashSet<_>>();
        definitions.extend(
            catalog
                .builtin_definitions()
                .into_iter()
                .filter(|definition| !plugin_names.contains(definition.name.as_str())),
        );

        // Splice in tools advertised by connected MCP servers.  Names use
        // `mcp:<server>:<tool>` so the session-level call parser can map
        // them back into `ToolInvocation::Mcp`.
        if let Some(manager) = &self.mcp_manager {
            let manager = manager.clone();
            let entries = mcp::block_on(async move { manager.all_tools().await });
            let existing: std::collections::HashSet<String> =
                definitions.iter().map(|d| d.name.clone()).collect();
            for (server, tool) in entries {
                let name = format!("mcp:{server}:{}", tool.name);
                if existing.contains(&name) {
                    continue;
                }
                let schema = tool.input_schema.unwrap_or_else(|| {
                    serde_json::json!({"type": "object", "properties": {}})
                });
                let description = tool.description.unwrap_or_default();
                definitions.push(ToolDefinition::plugin(
                    name,
                    description,
                    schema,
                    ToolBehavior::Mutating,
                    format!("mcp:{server}"),
                ));
            }
        }

        definitions.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.description.cmp(&right.description))
        });

        // Plugin chain: tool.definition. Let plugins rewrite descriptions /
        // input schemas before the list reaches the LLM.
        if !self.plugins.is_empty() {
            definitions = definitions
                .into_iter()
                .map(|def| {
                    let input = PluginToolDefinitionInput {
                        tool_name: def.name.clone(),
                        source: local_to_sdk_source(&def.source),
                        description: def.description.clone(),
                        input_schema: def.input_schema.clone(),
                    };
                    match self.plugins.dispatch_tool_definition_blocking(input) {
                        Ok(patched) => ToolDefinition {
                            description: patched.description,
                            input_schema: patched.input_schema,
                            ..def
                        },
                        Err(err) => {
                            tracing::warn!(
                                target: "agena_plugin_host::tool_definition",
                                tool = %def.name,
                                "tool.definition hook failed (keeping original): {err}"
                            );
                            def
                        }
                    }
                })
                .collect();
        }

        definitions
    }

    pub fn searchable_tools(&self) -> Vec<ToolDefinition> {
        self.catalogued_tools()
    }

    pub fn available_tools(&self) -> Vec<ToolDefinition> {
        self.catalogued_tools()
            .into_iter()
            .filter(ToolDefinition::should_load_by_default)
            .collect()
    }

    pub fn available_tools_for_messages(&self, messages: &[Message]) -> Vec<ToolDefinition> {
        self.available_tools_for_messages_and_loaded(messages, &[])
    }

    pub fn available_tools_for_messages_and_loaded(
        &self,
        messages: &[Message],
        loaded_tools: &[String],
    ) -> Vec<ToolDefinition> {
        let loaded_tools = collect_loaded_tool_names(messages, loaded_tools);
        self.catalogued_tools()
            .into_iter()
            .filter(|definition| {
                definition.should_load_by_default()
                    || loaded_tools.contains(definition.name.as_str())
            })
            .collect()
    }

    pub fn execute_builtin_detailed(
        &self,
        input: &BuiltinToolInput,
    ) -> Result<BuiltinExecution, ToolError> {
        self.execute_builtin_detailed_with_context(input, BuiltinExecutionContext::default())
    }

    fn execute_builtin_detailed_with_context(
        &self,
        input: &BuiltinToolInput,
        context: BuiltinExecutionContext,
    ) -> Result<BuiltinExecution, ToolError> {
        self.ensure_builtin_enabled(input)?;

        if self.permission_mode == PermissionExecutionMode::Enforced {
            match self.agent.authorize_builtin_tool(input) {
                PermissionDecision::Allow => {}
                PermissionDecision::Ask { reason } => return Err(ToolError::PermissionAsk(reason)),
                PermissionDecision::Deny { reason } => {
                    return Err(ToolError::PermissionDenied(reason));
                }
            }
        }

        let execution = self.dispatch_builtin_via_plugin_host(input, context)?;
        Ok(self.truncator.apply(execution))
    }

    /// Route a built-in tool call through the plugin host (in-process plugin
    /// `agena.builtin`). Falls back to the in-crate orchestrator if the
    /// plugin host has no entry registered for the built-in (e.g. test
    /// harnesses that build a `ToolExecutor` without going through
    /// `build_plugin_host`).
    fn dispatch_builtin_via_plugin_host(
        &self,
        input: &BuiltinToolInput,
        context: BuiltinExecutionContext,
    ) -> Result<BuiltinExecution, ToolError> {
        let tool_name = crate::permission::builtin_name(input).to_string();
        let resolution = match self.plugins.lookup_tool(&tool_name) {
            Some(r) if r.handle.plugin_id == builtins::BUILTIN_PLUGIN_ID => r,
            _ => {
                // Plugin host has no built-ins registered: fall back to direct
                // orchestrator dispatch so test fixtures keep working.
                return orchestrator::execute_builtin(self, input, context);
            }
        };

        let payload_value = builtin_input_payload(input)?;
        let session_id = context.session_id.unwrap_or(-1);
        let call_id = context.call_id.unwrap_or(-1);
        let response = builtins::with_executor(self, || {
            self.plugins.invoke_tool(
                &resolution.handle,
                PluginToolInvokeInput {
                    tool_name: tool_name.clone(),
                    session_id,
                    call_id,
                    workspace_root: self.workspace_root.to_string_lossy().to_string(),
                    input: payload_value,
                },
            )
        })
        .map_err(|err| ToolError::Plugin(err.message))?;

        let output = builtins::payload_to_builtin_envelope(response.payload.as_ref())
            .map_err(|err| ToolError::Plugin(format!("decode {tool_name} output: {err}")))?;
        let view = ToolExecutionView {
            title: response.title,
            output_text: response.output_text,
            metadata: response.metadata.into_iter().collect(),
            attachments: response.attachments,
        };
        let mut execution = BuiltinExecution::new(output.output, view);
        if let Some(apply) = output.apply_patch {
            execution = execution.with_apply_patch(apply);
        }
        Ok(execution)
    }

    pub fn collect_permission_checks(
        &self,
        input: &BuiltinToolInput,
    ) -> Result<Vec<ToolPermissionCheck>, ToolError> {
        self.ensure_builtin_enabled(input)?;

        let mut checks = vec![ToolPermissionCheck {
            action: PermissionAction::BuiltinTool {
                tool_name: crate::permission::builtin_name(input).to_string(),
            },
            decision: self.agent.authorize_builtin_tool(input),
        }];

        match input {
            BuiltinToolInput::Bash(payload) => {
                let cwd = payload
                    .workdir
                    .as_deref()
                    .map(|workdir| self.resolve_target_path(workdir))
                    .unwrap_or_else(|| self.workspace_root().to_path_buf());
                self.push_path_checks(&mut checks, AccessKind::Read, &cwd);
            }
            BuiltinToolInput::Read(payload) => {
                let target = self.resolve_target_path(&payload.file_path);
                self.push_path_checks(&mut checks, AccessKind::Read, &target);
            }
            BuiltinToolInput::ViewFile(payload) => {
                let target = self.resolve_target_path(&payload.path);
                self.push_path_checks(&mut checks, AccessKind::Read, &target);
            }
            BuiltinToolInput::ApplyPatch(payload) => {
                for path in apply_patch::planned_paths(&payload.patch)? {
                    let target = self.resolve_target_path(&path);
                    self.push_path_checks(&mut checks, AccessKind::Write, &target);
                }
            }
            BuiltinToolInput::Glob(payload) => {
                let base_path = payload
                    .path
                    .as_deref()
                    .map(|path| self.resolve_target_path(path))
                    .unwrap_or_else(|| self.workspace_root().to_path_buf());
                self.push_path_checks(&mut checks, AccessKind::Read, &base_path);
            }
            BuiltinToolInput::Grep(payload) => {
                let base_path = payload
                    .path
                    .as_deref()
                    .map(|path| self.resolve_target_path(path))
                    .unwrap_or_else(|| self.workspace_root().to_path_buf());
                self.push_path_checks(&mut checks, AccessKind::Read, &base_path);
            }
            BuiltinToolInput::ToolSearch(_) => {}
            BuiltinToolInput::TodoWrite(_) => {}
            BuiltinToolInput::Task(_) => {}
            BuiltinToolInput::AskUser(_) => {}
            BuiltinToolInput::Monitor(payload) => {
                if let crate::message::MonitorToolInput::Start { workdir, .. } = payload {
                    let cwd = workdir
                        .as_deref()
                        .map(|w| self.resolve_target_path(w))
                        .unwrap_or_else(|| self.workspace_root().to_path_buf());
                    self.push_path_checks(&mut checks, AccessKind::Read, &cwd);
                }
            }
            BuiltinToolInput::WebFetch(_) => {}
            BuiltinToolInput::WebSearch(_) => {}
            BuiltinToolInput::EnterPlanMode(_) => {}
            BuiltinToolInput::ExitPlanMode(_) => {}
            BuiltinToolInput::SkillRun(_) => {}
            BuiltinToolInput::EnterWorktree(_) => {}
            BuiltinToolInput::ExitWorktree(_) => {}
            BuiltinToolInput::CronCreate(_) => {}
            BuiltinToolInput::CronList(_) => {}
            BuiltinToolInput::CronDelete(_) => {}
            BuiltinToolInput::ScheduleWakeup(_) => {}
        }

        Ok(checks)
    }

    pub fn prepare_invocation(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<PreparedToolInvocation, ToolError> {
        let tool_name = invocation_name(invocation).to_owned();
        let source = invocation_source(invocation, self.plugins.as_ref());
        let input_json = invocation_input_json(invocation)?;
        let input_value: serde_json::Value =
            serde_json::from_str(&input_json).map_err(|e| ToolError::InvalidInput(e.to_string()))?;

        let hooked = self
            .plugins
            .dispatch_tool_before(PluginToolBeforeInput {
                tool_name: tool_name.clone(),
                source: local_to_sdk_source(&source),
                session_id,
                call_id,
                workspace_root: self.workspace_root.to_string_lossy().to_string(),
                input: input_value,
                title_override: None,
                metadata: Default::default(),
            })
            .map_err(|err| ToolError::Plugin(err.message))?;

        let input_json = serde_json::to_string(&hooked.input)
            .map_err(|e| ToolError::InvalidInput(e.to_string()))?;

        Ok(PreparedToolInvocation {
            invocation: parse_invocation_from_json(
                tool_name.as_str(),
                input_json.as_str(),
                &source,
            )?,
            title_override: hooked.title_override,
            metadata: hooked.metadata.into_iter().collect(),
        })
    }

    pub fn collect_permission_checks_for_invocation(
        &self,
        invocation: &ToolInvocation,
    ) -> Result<Vec<ToolPermissionCheck>, ToolError> {
        if let Some(builtin) = invocation.as_builtin() {
            return self.collect_permission_checks(&builtin);
        }
        match invocation {
            ToolInvocation::Custom { name, .. } => {
                let resolution = self
                    .plugins
                    .lookup_tool(name.as_str())
                    .ok_or_else(|| ToolError::UnknownTool(name.clone()))?;

                let behavior = sdk_to_local_behavior(resolution.decl.behavior);
                if !self.tool_catalog().is_behavior_enabled(behavior) {
                    return Err(ToolError::PermissionDenied(format!(
                        "tool '{name}' disabled for current model profile"
                    )));
                }

                Ok(vec![ToolPermissionCheck {
                    action: PermissionAction::BuiltinTool {
                        tool_name: name.clone(),
                    },
                    decision: self.agent.authorize_tool_name(name.as_str()),
                }])
            }
            ToolInvocation::Mcp { server, tool, .. } => {
                let action_name = format!("mcp:{server}:{tool}");
                Ok(vec![ToolPermissionCheck {
                    action: PermissionAction::BuiltinTool {
                        tool_name: action_name.clone(),
                    },
                    decision: self.agent.authorize_tool_name(action_name.as_str()),
                }])
            }
        }
    }

    pub fn execute_invocation_detailed(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<ToolInvocationExecution, ToolError> {
        if let Some(builtin) = invocation.as_builtin() {
            let mut execution: ToolInvocationExecution = self
                .execute_builtin_detailed_with_context(
                    &builtin,
                    BuiltinExecutionContext {
                        session_id: Some(session_id),
                        call_id: Some(call_id),
                    },
                )?
                .into();
            self.apply_after_hooks(invocation, session_id, call_id, &mut execution)?;
            return Ok(execution);
        }
        match invocation {
            ToolInvocation::Custom { name, input } => {
                let resolution = self
                    .plugins
                    .lookup_tool(name.as_str())
                    .ok_or_else(|| ToolError::UnknownTool(name.clone()))?;

                let payload_value = serde_json::Value::from(input.clone());
                let response = self
                    .plugins
                    .invoke_tool(
                        &resolution.handle,
                        PluginToolInvokeInput {
                            tool_name: resolution.handle.original_name.clone(),
                            session_id,
                            call_id,
                            workspace_root: self.workspace_root.to_string_lossy().to_string(),
                            input: payload_value,
                        },
                    )
                    .map_err(|err| ToolError::Plugin(err.message))?;

                let payload = response
                    .payload
                    .as_ref()
                    .map(|v| parse_custom_payload(&v.to_string()))
                    .transpose()?
                    .unwrap_or_default();
                let mut execution = ToolInvocationExecution::new(
                    ToolOutput::Custom {
                        output: CustomToolOutput {
                            name: name.clone(),
                            payload,
                        },
                    },
                    ToolExecutionView {
                        title: response.title.clone(),
                        output_text: response.output_text.clone(),
                        metadata: response.metadata.into_iter().collect(),
                        attachments: Vec::new(),
                    },
                );
                self.apply_after_hooks(invocation, session_id, call_id, &mut execution)?;
                Ok(execution)
            }
            ToolInvocation::Mcp { server, tool, input } => {
                let manager = self.mcp_manager.as_ref().ok_or_else(|| {
                    ToolError::UnsupportedInvocation(format!(
                        "mcp:{server}:{tool} (no MCP manager configured)"
                    ))
                })?;
                let mut execution = mcp::invoke(manager, server, tool, input)?;
                self.apply_after_hooks(invocation, session_id, call_id, &mut execution)?;
                Ok(execution)
            }
        }
    }

    pub fn execute_invocation_detailed_bypassing_permissions(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<ToolInvocationExecution, ToolError> {
        let mut trusted = self.clone();
        trusted.permission_mode = PermissionExecutionMode::Bypassed;
        trusted.execute_invocation_detailed(invocation, session_id, call_id)
    }

    pub fn shell_env_overrides(
        &self,
        cwd: &Path,
        session_id: Option<i64>,
        call_id: Option<i64>,
    ) -> Result<std::collections::HashMap<String, String>, ToolError> {
        let patch = self
            .plugins
            .dispatch_shell_env(PluginShellEnvInput {
                cwd: cwd.to_path_buf(),
                session_id,
                call_id,
            })
            .map_err(|err| ToolError::Plugin(err.message))?;
        Ok(patch.set.into_iter().collect())
    }

    fn ensure_builtin_enabled(&self, input: &BuiltinToolInput) -> Result<(), ToolError> {
        let availability = self
            .tool_catalog()
            .availability_for_input(&self.agent, input);
        if !availability.enabled {
            return Err(ToolError::UnsupportedInvocation(
                availability.tool_name.to_string(),
            ));
        }
        Ok(())
    }

    pub fn execute_builtin_with_permission_runtime<S>(
        &self,
        session_id: Option<i64>,
        runtime: &mut PermissionRuntime<S>,
        input: &BuiltinToolInput,
    ) -> Result<PermissionedBuiltinExecution, ToolError>
    where
        S: PermissionRuleStore,
    {
        let base = self.agent.authorize_builtin_tool(input);
        let action = PermissionAction::BuiltinTool {
            tool_name: crate::permission::builtin_name(input).to_string(),
        };
        match runtime.decide_or_request_with_plugins(
            session_id,
            action,
            base,
            Some(&self.plugins),
        ) {
            Ok(PermissionRuntimeDecision::Immediate(PermissionDecision::Allow)) => Ok(
                PermissionedBuiltinExecution::Executed(self.execute_builtin_detailed(input)?),
            ),
            Ok(PermissionRuntimeDecision::Immediate(PermissionDecision::Deny { reason })) => {
                Err(ToolError::PermissionDenied(reason))
            }
            Ok(PermissionRuntimeDecision::Immediate(PermissionDecision::Ask { reason })) => {
                Err(ToolError::PermissionAsk(reason))
            }
            Ok(PermissionRuntimeDecision::Pending(request)) => {
                Ok(PermissionedBuiltinExecution::Pending(request))
            }
            Err(err) => Err(ToolError::InvalidInput(err.to_string())),
        }
    }

    pub fn execute_builtin(
        &self,
        input: &BuiltinToolInput,
    ) -> Result<(BuiltinToolOutput, Option<ApplyPatchExecution>), ToolError> {
        let execution = self.execute_builtin_detailed(input)?;
        Ok((execution.output, execution.apply_patch))
    }

    fn apply_after_hooks(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
        execution: &mut ToolInvocationExecution,
    ) -> Result<(), ToolError> {
        let tool_name = invocation_name(invocation).to_owned();
        let source = invocation_source(invocation, self.plugins.as_ref());
        let payload_value: Option<serde_json::Value> = match &execution.output {
            ToolOutput::Custom { output } => Some(serde_json::Value::from(output.payload.clone())),
            _ => None,
        };

        let after_in = PluginToolAfterInput {
            tool_name,
            source: local_to_sdk_source(&source),
            session_id,
            call_id,
            workspace_root: self.workspace_root.to_string_lossy().to_string(),
            title: execution.view.title.clone(),
            output_text: execution.view.output_text.clone(),
            payload: payload_value,
            metadata: execution.view.metadata.clone().into_iter().collect(),
        };

        let hooked = self
            .plugins
            .dispatch_tool_after(after_in)
            .map_err(|err| ToolError::Plugin(err.message))?;

        execution.view.title = hooked.title;
        execution.view.output_text = hooked.output_text;
        for (k, v) in hooked.metadata {
            execution.view.metadata.insert(k, v);
        }

        if let (Some(payload_value), ToolOutput::Custom { output }) =
            (hooked.payload, &mut execution.output)
        {
            output.payload =
                parse_custom_payload(&payload_value.to_string()).unwrap_or_default();
        }

        Ok(())
    }

    /// Fire-and-forget notification to plugins about a tool execution failure.
    pub fn broadcast_tool_failure(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
        error: &str,
    ) {
        if self.plugins.is_empty() {
            return;
        }
        let tool_name = invocation_name(invocation).to_owned();
        let source = invocation_source(invocation, &self.plugins);
        let input_value = invocation_input_json(invocation)
            .ok()
            .and_then(|j| serde_json::from_str::<serde_json::Value>(&j).ok())
            .unwrap_or(serde_json::Value::Null);
        let failure_input = PluginToolFailureInput {
            tool_name,
            source: local_to_sdk_source(&source),
            session_id,
            call_id,
            workspace_root: self.workspace_root.to_string_lossy().to_string(),
            input: input_value,
            error: error.to_owned(),
            is_interrupt: false,
        };
        let plugins = Arc::clone(&self.plugins);
        tokio::spawn(async move {
            plugins.broadcast_tool_failure(failure_input).await;
        });
    }

    pub(crate) fn resolve_target_path(&self, raw_path: &str) -> PathBuf {
        let candidate = PathBuf::from(raw_path);
        if candidate.is_absolute() {
            candidate
        } else {
            self.workspace_root.join(candidate)
        }
    }

    pub(crate) fn execute_shell_command(
        &self,
        request: &ShellRequest,
    ) -> Result<ShellOutput, ToolError> {
        shell::execute(request, self.sandbox_policy()).map_err(ToolError::from)
    }

    pub(crate) fn display_path(&self, path: &Path) -> String {
        if let Ok(relative) = path.strip_prefix(&self.workspace_root) {
            let normalized = normalize_path_for_display(relative);
            if normalized.is_empty() {
                return ".".to_string();
            }
            return normalized;
        }
        normalize_path_for_display(path)
    }

    pub(crate) fn ensure_read_permission(&self, target_path: &Path) -> Result<(), ToolError> {
        self.ensure_access_permission(AccessKind::Read, target_path)
    }

    pub(crate) fn ensure_edit_permission(&self, target_path: &Path) -> Result<(), ToolError> {
        self.ensure_access_permission(AccessKind::Write, target_path)
    }

    fn ensure_access_permission(
        &self,
        access: AccessKind,
        target_path: &Path,
    ) -> Result<(), ToolError> {
        if self.permission_mode == PermissionExecutionMode::Bypassed {
            return Ok(());
        }

        match self.agent.authorize_path_access(
            AccessKind::ExternalDirectory,
            self.workspace_root(),
            target_path,
        ) {
            PermissionDecision::Allow => {}
            PermissionDecision::Ask { reason } => return Err(ToolError::PermissionAsk(reason)),
            PermissionDecision::Deny { reason } => return Err(ToolError::PermissionDenied(reason)),
        }

        match self
            .agent
            .authorize_path_access(access, self.workspace_root(), target_path)
        {
            PermissionDecision::Allow => Ok(()),
            PermissionDecision::Ask { reason } => Err(ToolError::PermissionAsk(reason)),
            PermissionDecision::Deny { reason } => Err(ToolError::PermissionDenied(reason)),
        }
    }

    fn push_path_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        access: AccessKind,
        target_path: &Path,
    ) {
        let workspace_root = normalize_path_for_display(self.workspace_root());
        let target = normalize_path_for_display(target_path);

        checks.push(ToolPermissionCheck {
            action: PermissionAction::PathAccess {
                access_kind: access_kind_name(AccessKind::ExternalDirectory).to_string(),
                workspace_root: workspace_root.clone(),
                target_path: target.clone(),
            },
            decision: self.agent.authorize_path_access(
                AccessKind::ExternalDirectory,
                self.workspace_root(),
                target_path,
            ),
        });
        checks.push(ToolPermissionCheck {
            action: PermissionAction::PathAccess {
                access_kind: access_kind_name(access).to_string(),
                workspace_root,
                target_path: target,
            },
            decision: self
                .agent
                .authorize_path_access(access, self.workspace_root(), target_path),
        });
    }
}

pub(crate) fn normalize_path_for_display(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn access_kind_name(access: AccessKind) -> &'static str {
    match access {
        AccessKind::Read => "read",
        AccessKind::Write => "write",
        AccessKind::ExternalDirectory => "external_directory",
    }
}

fn invocation_name(invocation: &ToolInvocation) -> String {
    match invocation {
        ToolInvocation::Custom { name, .. } => name.clone(),
        ToolInvocation::Mcp { server, tool, .. } => format!("mcp:{server}:{tool}"),
    }
}

fn invocation_source(invocation: &ToolInvocation, plugins: &PluginHost) -> ToolSource {
    match invocation {
        ToolInvocation::Custom { name, .. } => {
            if crate::message::canonical_builtin_name(name.as_str()).is_some() {
                ToolSource::Builtin
            } else {
                plugins
                    .lookup_tool(name.as_str())
                    .map(|res| ToolSource::Plugin {
                        plugin_name: res.handle.plugin_id.clone(),
                    })
                    .unwrap_or_else(|| ToolSource::Plugin {
                        plugin_name: "custom".to_string(),
                    })
            }
        }
        ToolInvocation::Mcp { server, .. } => ToolSource::Plugin {
            plugin_name: format!("mcp:{server}"),
        },
    }
}

fn local_to_sdk_source(source: &ToolSource) -> SdkToolSource {
    match source {
        ToolSource::Builtin => SdkToolSource::Builtin,
        ToolSource::Plugin { plugin_name } => SdkToolSource::Plugin {
            plugin: plugin_name.clone(),
        },
    }
}

fn sdk_to_local_behavior(b: SdkToolBehavior) -> ToolBehavior {
    match b {
        SdkToolBehavior::ReadOnly => ToolBehavior::ReadOnly,
        SdkToolBehavior::WriteSandboxed | SdkToolBehavior::WriteUnsandboxed => {
            ToolBehavior::Mutating
        }
        SdkToolBehavior::Task => ToolBehavior::Task,
    }
}

/// Serialize a `BuiltinToolInput` to the JSON shape expected by the
/// in-process built-in plugin. The plugin parses by tool name (passed as
/// `ToolInvokeInput::tool_name`) and consumes the inner payload only.
fn builtin_input_payload(input: &BuiltinToolInput) -> Result<serde_json::Value, ToolError> {
    let value = serde_json::to_value(input)
        .map_err(|err| ToolError::Plugin(format!("encode built-in input: {err}")))?;
    let mut obj = match value {
        serde_json::Value::Object(o) => o,
        other => return Ok(other),
    };
    obj.remove("tool");
    Ok(serde_json::Value::Object(obj))
}

fn invocation_input_json(invocation: &ToolInvocation) -> Result<String, ToolError> {
    match invocation {
        ToolInvocation::Custom { input, .. } => {
            serde_json::to_string(&serde_json::Value::from(input.clone()))
                .map_err(|err| ToolError::InvalidInput(err.to_string()))
        }
        ToolInvocation::Mcp { input, .. } => {
            serde_json::to_string(input).map_err(|err| ToolError::InvalidInput(err.to_string()))
        }
    }
}

fn parse_invocation_from_json(
    tool_name: &str,
    input_json: &str,
    source: &ToolSource,
) -> Result<ToolInvocation, ToolError> {
    match source {
        ToolSource::Builtin => {
            let builtin = parse_builtin_input(tool_name, input_json)?;
            Ok(builtin.into_invocation())
        }
        ToolSource::Plugin { .. } => {
            let value = if input_json.trim().is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str(input_json)
                    .map_err(|err| ToolError::InvalidInput(err.to_string()))?
            };
            let input = StructuredObject::try_from(value)
                .map_err(|err| ToolError::InvalidInput(err.to_string()))?;
            Ok(ToolInvocation::Custom {
                name: tool_name.to_string(),
                input,
            })
        }
    }
}

fn parse_builtin_input(tool_name: &str, input_json: &str) -> Result<BuiltinToolInput, ToolError> {
    fn parse<T>(value: &str) -> Result<T, ToolError>
    where
        T: serde::de::DeserializeOwned,
    {
        let payload = if value.trim().is_empty() { "{}" } else { value };
        serde_json::from_str(payload).map_err(|err| ToolError::InvalidInput(err.to_string()))
    }

    match tool_name {
        "bash" => Ok(BuiltinToolInput::Bash(parse(input_json)?)),
        "read" => Ok(BuiltinToolInput::Read(parse(input_json)?)),
        "view_file" => Ok(BuiltinToolInput::ViewFile(parse(input_json)?)),
        "apply_patch" => Ok(BuiltinToolInput::ApplyPatch(parse(input_json)?)),
        "glob" => Ok(BuiltinToolInput::Glob(parse(input_json)?)),
        "grep" => Ok(BuiltinToolInput::Grep(parse(input_json)?)),
        "task" => Ok(BuiltinToolInput::Task(parse(input_json)?)),
        "tool_search" => Ok(BuiltinToolInput::ToolSearch(parse(input_json)?)),
        "todo_write" => Ok(BuiltinToolInput::TodoWrite(parse(input_json)?)),
        "ask_user" | "request_user_input" => Ok(BuiltinToolInput::AskUser(parse(input_json)?)),
        other => Err(ToolError::UnknownTool(other.to_string())),
    }
}

fn collect_loaded_tool_names(
    messages: &[Message],
    runtime_loaded_tools: &[String],
) -> std::collections::HashSet<String> {
    let mut loaded = messages
        .iter()
        .flat_map(|message| message.parts.iter())
        .filter_map(|part| match part.content.as_ref() {
            Some(PartContent::ToolExecution(ToolExecutionPart::Completed {
                details, ..
            })) => match details.as_builtin() {
                Some(BuiltinToolOutput::ToolSearch { loaded_tools, .. }) => Some(loaded_tools),
                _ => None,
            },
            _ => None,
        })
        .flatten()
        .collect::<std::collections::HashSet<_>>();

    loaded.extend(
        runtime_loaded_tools
            .iter()
            .map(String::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(ToOwned::to_owned),
    );

    loaded
}

fn parse_custom_payload(payload_json: &str) -> Result<StructuredObject, ToolError> {
    let value = if payload_json.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(payload_json)
            .map_err(|err| ToolError::InvalidInput(err.to_string()))?
    };
    StructuredObject::try_from(value).map_err(|err| ToolError::InvalidInput(err.to_string()))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use chrono::Utc;
    use serde_json::json;
    use uuid::Uuid;

    use crate::message::{
        ApplyPatchToolInput, BashToolInput, BuiltinToolInput, BuiltinToolOutput, FileChangeKind,
        GlobToolInput, GrepToolInput, Message, PartContent, ReadToolInput, StructuredObject,
        TaskSubagentType, TaskToolInput, TimeRange, TodoItem, TodoPriority, TodoStatus,
        TodoWriteToolInput, ToolExecutionPart, ToolInvocation, ToolOutput, ToolSearchToolInput,
        ViewFileToolInput,
    };
    use crate::permission::PermissionPolicy;
    use crate::plugin::sdk::prelude::*;
    use crate::plugin::{PluginEntry, PluginHost, PluginHostBuilder, PluginsConfig};
    use crate::role::Role;

    use super::{ExecutionPolicy, ToolBehavior, ToolError, ToolExecutor, ToolSource};

    #[derive(Debug)]
    struct TempWorkspace {
        root: PathBuf,
    }

    impl TempWorkspace {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("agena-tool-tests-{}", Uuid::new_v4()));
            fs::create_dir_all(&root).expect("failed to create temp workspace");
            Self { root }
        }
    }

    impl Drop for TempWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn build_executor(root: &Path) -> ToolExecutor {
        let agent = crate::agent::Agent::new("build", PermissionPolicy::allow_all());
        ToolExecutor::new(root, agent)
    }

    fn build_executor_with_policy(root: &Path, policy: ExecutionPolicy) -> ToolExecutor {
        let agent = crate::agent::Agent::new("build", PermissionPolicy::allow_all());
        ToolExecutor::with_sandbox_policy(root, agent, policy)
    }

    #[derive(Debug, Default)]
    struct FixturePlugin;

    #[async_trait::async_trait]
    impl Plugin for FixturePlugin {
        fn manifest(&self) -> PluginManifest {
            PluginManifest::builder("fixture", "0.1.0")
                .description("fixture plugin")
                .hooks(
                    HookSubscription::TOOL_BEFORE
                        | HookSubscription::TOOL_AFTER
                        | HookSubscription::TOOL_INVOKE
                        | HookSubscription::SHELL_ENV,
                )
                .tool(
                    ToolDecl::new(
                        "plugin_echo",
                        json!({
                            "type": "object",
                            "properties": { "message": { "type": "string" } },
                            "required": ["message"]
                        }),
                    )
                    .description("Echo a message from the plugin.")
                    .behavior(crate::plugin::sdk::ToolBehavior::ReadOnly),
                )
                .build()
        }

        async fn tool_invoke(&self, input: ToolInvokeInput) -> Result<ToolInvokeOutput> {
            let message = input
                .input
                .get("message")
                .and_then(|v| v.as_str())
                .ok_or_else(|| PluginError::new("missing message"))?
                .to_string();
            Ok(ToolInvokeOutput {
                title: "Plugin echo".to_string(),
                output_text: message.clone(),
                payload: Some(json!({ "echoed": message })),
                metadata: std::collections::BTreeMap::from([(
                    "plugin".to_string(),
                    "fixture".to_string(),
                )]),
                attachments: Vec::new(),
            })
        }

        async fn tool_execute_before(
            &self,
            input: ToolBeforeInput,
        ) -> Result<Option<ToolBeforePatch>> {
            if input.tool_name != "plugin_echo" {
                return Ok(None);
            }
            let mut new_input = input.input.clone();
            let message = new_input
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            new_input["message"] = serde_json::Value::String(format!("{message} prepared"));
            Ok(Some(ToolBeforePatch {
                input: Some(new_input),
                title_override: Some("Prepared plugin echo".to_string()),
                metadata: Default::default(),
            }))
        }

        async fn tool_execute_after(
            &self,
            input: ToolAfterInput,
        ) -> Result<Option<ToolAfterPatch>> {
            if input.tool_name != "plugin_echo" {
                return Ok(None);
            }
            let mut payload = input.payload.clone().unwrap_or_else(|| json!({}));
            payload["after"] = serde_json::Value::Bool(true);
            Ok(Some(ToolAfterPatch {
                title: Some(format!("{} after", input.title)),
                output_text: Some(format!("{} after", input.output_text)),
                payload: Some(payload),
                metadata: std::collections::BTreeMap::from([(
                    "after_hook".to_string(),
                    "applied".to_string(),
                )]),
            }))
        }

        async fn shell_env(&self, _input: ShellEnvInput) -> Result<Option<ShellEnvPatch>> {
            Ok(Some(ShellEnvPatch::set("PLUGIN_FLAG", "from_plugin")))
        }
    }

    fn build_plugin_manager() -> Arc<PluginHost> {
        use std::collections::BTreeMap;
        use std::sync::OnceLock;
        // Long-lived runtime so the host's `runtime_handle` stays valid for
        // the life of the test process.
        static TEST_RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        let rt = TEST_RT.get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .build()
                .expect("test plugin runtime")
        });
        let mut list = BTreeMap::new();
        list.insert(
            "fixture".to_string(),
            PluginEntry::Static {
                options: serde_json::Value::Null,
                timeouts: Default::default(),
            },
        );
        let config = PluginsConfig {
            enabled: true,
            timeouts: Default::default(),
            list,
            trusted_keys: Default::default(),
        };
        rt.block_on(async {
            PluginHostBuilder::new(std::env::current_dir().unwrap_or_default(), "test")
                .with_config(config)
                .register_static("fixture", FixturePlugin)
                .build()
                .await
                .expect("plugin host should build")
        })
    }

    fn loaded_tool_search_message(loaded_tools: &[&str]) -> Message {
        Message {
            id: 99,
            role: Role::Tool,
            state: crate::message::MessageStatus::Completed,
            parts: vec![crate::message::MessagePart::with_content(
                1,
                99,
                Utc::now(),
                crate::message::ExecutionStatus::Completed,
                PartContent::ToolExecution(ToolExecutionPart::Completed {
                    call_id: 1,
                    invocation: BuiltinToolInput::ToolSearch(ToolSearchToolInput {
                        query: "load mutating tools".to_string(),
                        load: loaded_tools.iter().map(|name| name.to_string()).collect(),
                        limit: None,
                    })
                    .into_invocation(),
                    output_text: "loaded deferred tools".to_string(),
                    blocks: Vec::new(),
                    attachments: Vec::new(),
                    details: ToolOutput::Custom {
                        output: BuiltinToolOutput::ToolSearch {
                            results: Vec::new(),
                            loaded_tools: loaded_tools
                                .iter()
                                .map(|name| name.to_string())
                                .collect(),
                        }
                        .into_custom_output(),
                    },
                    lifecycle: TimeRange::default(),
                }),
            )],
            created_at: Utc::now(),
            metadata: crate::message::MessageMetadata::default(),
            usage: None,
            finish: None,
        }
    }

    fn sample_png_bytes() -> Vec<u8> {
        STANDARD
            .decode(
                "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO9W7tYAAAAASUVORK5CYII=",
            )
            .expect("sample png should decode")
    }

    #[test]
    fn read_builtin_returns_line_numbered_preview() {
        let workspace = TempWorkspace::new();
        let file_path = workspace.root.join("notes.txt");
        fs::write(&file_path, "one\ntwo\nthree\n").expect("failed to seed file");

        let executor = build_executor(&workspace.root);
        let result = executor
            .execute_builtin_detailed(&BuiltinToolInput::Read(ReadToolInput {
                file_path: "notes.txt".to_string(),
                offset: Some(2),
                limit: Some(2),
            }))
            .expect("read builtin should succeed");

        match result.output {
            BuiltinToolOutput::Read {
                preview,
                truncated,
                loaded_paths,
            } => {
                let preview = preview.expect("preview must exist");
                assert!(preview.contains("2: two"));
                assert!(preview.contains("3: three"));
                assert_eq!(truncated, Some(false));
                assert_eq!(loaded_paths, vec!["notes.txt".to_string()]);
            }
            other => panic!("expected read output, got {other:?}"),
        }
    }

    #[test]
    fn apply_patch_builtin_reports_typed_file_changes() {
        let workspace = TempWorkspace::new();
        fs::write(workspace.root.join("keep.txt"), "before\n").expect("failed to seed keep.txt");
        fs::write(workspace.root.join("remove.txt"), "delete me\n")
            .expect("failed to seed remove.txt");
        let executor = build_executor(&workspace.root);

        let result = executor
            .execute_builtin_detailed(&BuiltinToolInput::ApplyPatch(ApplyPatchToolInput {
                patch: "\
*** Begin Patch
*** Add File: added.txt
+created
*** Update File: keep.txt
@@
-before
+after
*** Delete File: remove.txt
*** End Patch"
                    .to_string(),
            }))
            .expect("apply_patch should succeed");

        match result.output {
            BuiltinToolOutput::ApplyPatch { changes, .. } => {
                assert_eq!(changes.len(), 3);
                assert!(changes.iter().any(|change| {
                    change.path == "added.txt" && change.kind == FileChangeKind::Added
                }));
                assert!(changes.iter().any(|change| {
                    change.path == "keep.txt" && change.kind == FileChangeKind::Updated
                }));
                assert!(changes.iter().any(|change| {
                    change.path == "remove.txt" && change.kind == FileChangeKind::Deleted
                }));
            }
            other => panic!("expected apply_patch output, got {other:?}"),
        }
    }

    #[test]
    fn view_file_builtin_returns_metadata_and_attachment() {
        let workspace = TempWorkspace::new();
        let file_path = workspace.root.join("pixel.png");
        fs::write(&file_path, sample_png_bytes()).expect("failed to seed png");

        let executor = build_executor(&workspace.root);
        let result = executor
            .execute_builtin_detailed(&BuiltinToolInput::ViewFile(ViewFileToolInput {
                path: "pixel.png".to_string(),
            }))
            .expect("view_file should succeed");

        match result.output {
            BuiltinToolOutput::ViewFile {
                path,
                kind,
                mime,
                size_bytes,
                filename,
                width,
                height,
                duration_ms,
                page_count,
            } => {
                assert_eq!(path, "pixel.png");
                assert_eq!(kind, crate::message::AttachmentKind::Image);
                assert_eq!(mime, "image/png");
                assert!(size_bytes > 0);
                assert_eq!(filename.as_deref(), Some("pixel.png"));
                assert_eq!(width, Some(1));
                assert_eq!(height, Some(1));
                assert_eq!(duration_ms, None);
                assert_eq!(page_count, None);
            }
            other => panic!("expected view_file output, got {other:?}"),
        }

        assert_eq!(result.view.attachments.len(), 1);
        let attachment = &result.view.attachments[0];
        assert_eq!(attachment.filename.as_deref(), Some("pixel.png"));
        assert_eq!(attachment.kind, crate::message::AttachmentKind::Image);
        assert_eq!(attachment.mime, "image/png");
        match &attachment.source {
            crate::message::AttachmentSource::Base64 { data } => assert!(!data.is_empty()),
            other => panic!("expected base64 attachment source, got {other:?}"),
        }
    }

    #[test]
    fn view_file_builtin_attaches_generic_text_file() {
        let workspace = TempWorkspace::new();
        let file_path = workspace.root.join("notes.txt");
        fs::write(&file_path, "hello from agena\n").expect("failed to seed text file");

        let executor = build_executor(&workspace.root);
        let result = executor
            .execute_builtin_detailed(&BuiltinToolInput::ViewFile(ViewFileToolInput {
                path: "notes.txt".to_string(),
            }))
            .expect("view_file should succeed for text file");

        match result.output {
            BuiltinToolOutput::ViewFile {
                path,
                kind,
                mime,
                filename,
                width,
                height,
                ..
            } => {
                assert_eq!(path, "notes.txt");
                assert_eq!(kind, crate::message::AttachmentKind::File);
                assert_eq!(mime, "text/plain");
                assert_eq!(filename.as_deref(), Some("notes.txt"));
                assert_eq!(width, None);
                assert_eq!(height, None);
            }
            other => panic!("expected view_file output, got {other:?}"),
        }

        assert_eq!(result.view.attachments.len(), 1);
        let attachment = &result.view.attachments[0];
        assert_eq!(attachment.kind, crate::message::AttachmentKind::File);
        assert_eq!(attachment.mime, "text/plain");
        match &attachment.source {
            crate::message::AttachmentSource::Base64 { data } => assert!(!data.is_empty()),
            other => panic!("expected base64 attachment source, got {other:?}"),
        }
    }

    #[test]
    fn glob_and_grep_report_match_counts() {
        let workspace = TempWorkspace::new();
        fs::create_dir_all(workspace.root.join("src/nested")).expect("failed to create tree");
        fs::write(
            workspace.root.join("src/main.rs"),
            "fn main() { println!(\"hello\"); }\n",
        )
        .expect("failed to write main.rs");
        fs::write(
            workspace.root.join("src/nested/lib.rs"),
            "pub fn value() -> i32 { 7 }\n",
        )
        .expect("failed to write lib.rs");

        let executor = build_executor(&workspace.root);

        let glob_result = executor
            .execute_builtin_detailed(&BuiltinToolInput::Glob(GlobToolInput {
                pattern: "**/*.rs".to_string(),
                path: Some("src".to_string()),
            }))
            .expect("glob should succeed");

        match glob_result.output {
            BuiltinToolOutput::Glob { count } => {
                assert_eq!(count, Some(2));
            }
            other => panic!("expected glob output, got {other:?}"),
        }

        let grep_result = executor
            .execute_builtin_detailed(&BuiltinToolInput::Grep(GrepToolInput {
                pattern: "hello".to_string(),
                path: Some("src".to_string()),
                include: Some("**/*.rs".to_string()),
            }))
            .expect("grep should succeed");

        match grep_result.output {
            BuiltinToolOutput::Grep { matches } => {
                assert_eq!(matches, Some(1));
            }
            other => panic!("expected grep output, got {other:?}"),
        }
    }

    #[test]
    fn task_builtin_generates_session_id() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let result = executor
            .execute_builtin_detailed(&BuiltinToolInput::Task(TaskToolInput {
                description: "inspect code".to_string(),
                prompt: "find modules".to_string(),
                subagent_type: TaskSubagentType::Explore,
                task_id: None,
                command: None,
            }))
            .expect("task should succeed");

        assert_eq!(
            result
                .view
                .metadata
                .get("subagent_type")
                .map(String::as_str),
            Some("explore")
        );
        assert_eq!(
            result
                .view
                .metadata
                .get("profile_guidance")
                .map(String::as_str),
            Some(TaskSubagentType::Explore.guidance())
        );

        match result.output {
            BuiltinToolOutput::Task { session_id, .. } => {
                assert!(session_id.is_some());
            }
            other => panic!("expected task output, got {other:?}"),
        }
    }

    #[test]
    fn tool_search_builtin_discovers_and_loads_deferred_tools() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let result = executor
            .execute_builtin_detailed(&BuiltinToolInput::ToolSearch(ToolSearchToolInput {
                query: "patch files".to_string(),
                load: vec!["apply_patch".to_string()],
                limit: None,
            }))
            .expect("tool_search should succeed");

        match result.output {
            BuiltinToolOutput::ToolSearch {
                results,
                loaded_tools,
            } => {
                assert!(results.iter().any(|name| name == "apply_patch"));
                assert_eq!(loaded_tools, vec!["apply_patch".to_string()]);
            }
            other => panic!("expected tool_search output, got {other:?}"),
        }

        assert!(result.view.output_text.contains("Loaded deferred tools"));
    }

    #[test]
    fn tool_search_messages_expose_deferred_tools_in_later_turns() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let initial = executor.available_tools();
        assert!(initial.iter().any(|tool| tool.name == "tool_search"));
        assert!(initial.iter().any(|tool| tool.name == "todo_write"));
        assert!(!initial.iter().any(|tool| tool.name == "bash"));
        assert!(!initial.iter().any(|tool| tool.name == "task"));

        let messages = vec![loaded_tool_search_message(&["bash", "task"])];
        let available = executor.available_tools_for_messages(messages.as_slice());

        assert!(available.iter().any(|tool| tool.name == "bash"));
        assert!(available.iter().any(|tool| tool.name == "task"));
    }

    #[test]
    fn available_tools_are_sorted_stably_for_request_fingerprints() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let names = executor
            .available_tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        let mut expected = names.clone();
        expected.sort();

        assert_eq!(names, expected);
    }

    #[test]
    fn todo_write_builtin_returns_items_for_session_state() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let result = executor
            .execute_builtin_detailed(&BuiltinToolInput::TodoWrite(TodoWriteToolInput {
                items: vec![TodoItem {
                    content: "Implement tool_search".to_string(),
                    status: TodoStatus::InProgress,
                    priority: TodoPriority::High,
                }],
            }))
            .expect("todo_write should succeed");

        match result.output {
            BuiltinToolOutput::TodoWrite { items } => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].content, "Implement tool_search");
                assert_eq!(items[0].status, TodoStatus::InProgress);
            }
            other => panic!("expected todo_write output, got {other:?}"),
        }
    }

    #[test]
    fn bash_builtin_runs_command_with_read_only_policy() {
        if cfg!(windows) {
            // Windows host environments can include PATH entries whose ACL cannot be audited
            // in sandbox preflight, which makes this smoke test flaky/non-portable.
            return;
        }

        let workspace = TempWorkspace::new();
        let executor = build_executor_with_policy(
            &workspace.root,
            ExecutionPolicy::read_only(),
        );

        let result = executor
            .execute_builtin_detailed(&BuiltinToolInput::Bash(BashToolInput {
                command: "echo hello_agena".to_string(),
                description: "smoke bash".to_string(),
                timeout_ms: Some(30_000),
                workdir: None,
            }))
            .expect("bash builtin should succeed");

        match &result.output {
            BuiltinToolOutput::Bash {
                output,
                description,
            } => {
                let output = output
                    .as_deref()
                    .expect("output should exist")
                    .to_ascii_lowercase();
                assert!(output.contains("hello_agena"));
                assert!(description.is_some());
            }
            other => panic!("expected bash output, got {other:?}"),
        }
    }

    #[test]
    fn bash_builtin_explains_no_match_exit_codes() {
        if cfg!(windows) {
            return;
        }

        let workspace = TempWorkspace::new();
        fs::write(workspace.root.join("notes.txt"), "alpha\nbeta\n")
            .expect("failed to seed notes file");
        let executor = build_executor_with_policy(
            &workspace.root,
            ExecutionPolicy::read_only(),
        );

        let result = executor
            .execute_builtin_detailed(&BuiltinToolInput::Bash(BashToolInput {
                command: "grep missing notes.txt".to_string(),
                description: "search missing text".to_string(),
                timeout_ms: Some(30_000),
                workdir: None,
            }))
            .expect("bash builtin should succeed");

        match result.output {
            BuiltinToolOutput::Bash {
                output,
                description,
            } => {
                assert!(
                    output
                        .as_deref()
                        .is_some_and(|text| text.contains("no matches"))
                );
                assert!(
                    description
                        .as_deref()
                        .is_some_and(|text| text.contains("no matches"))
                );
            }
            other => panic!("expected bash output, got {other:?}"),
        }

        assert_eq!(
            result
                .view
                .metadata
                .get("exit_interpretation")
                .map(String::as_str),
            Some("no_matches")
        );
    }

    #[test]
    fn bash_builtin_explains_diff_exit_codes() {
        if cfg!(windows) {
            return;
        }

        let workspace = TempWorkspace::new();
        fs::write(workspace.root.join("left.txt"), "alpha\n").expect("failed to write left file");
        fs::write(workspace.root.join("right.txt"), "beta\n").expect("failed to write right file");
        let executor = build_executor_with_policy(
            &workspace.root,
            ExecutionPolicy::read_only(),
        );

        let result = executor
            .execute_builtin_detailed(&BuiltinToolInput::Bash(BashToolInput {
                command: "diff left.txt right.txt".to_string(),
                description: "compare files".to_string(),
                timeout_ms: Some(30_000),
                workdir: None,
            }))
            .expect("bash builtin should succeed");

        match &result.output {
            BuiltinToolOutput::Bash { description, .. } => {
                assert!(
                    description
                        .as_deref()
                        .is_some_and(|text| text.contains("found differences"))
                );
            }
            other => panic!("expected bash output, got {other:?}"),
        }

        assert_eq!(
            result
                .view
                .metadata
                .get("exit_interpretation")
                .map(String::as_str),
            Some("differences_found")
        );
    }

    #[test]
    fn bash_builtin_blocks_obvious_write_commands_in_read_only_policy() {
        if cfg!(windows) {
            return;
        }

        let workspace = TempWorkspace::new();
        let executor = build_executor_with_policy(
            &workspace.root,
            ExecutionPolicy::read_only(),
        );

        let err = executor
            .execute_builtin_detailed(&BuiltinToolInput::Bash(BashToolInput {
                command: "echo hi > created.txt".to_string(),
                description: "attempt write".to_string(),
                timeout_ms: Some(30_000),
                workdir: None,
            }))
            .expect_err("write command should be rejected before execution");

        match err {
            ToolError::PermissionDenied(message) => {
                assert!(message.contains("read-only sandbox"));
                assert!(message.contains("output redirection"));
            }
            other => panic!("expected permission denial, got {other:?}"),
        }
    }

    #[test]
    fn readonly_model_profile_disables_apply_patch_and_task_tools() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root).with_model_id("gpt-readonly");

        let availability = executor.available_builtins();
        let find = |tool_name: &str| {
            availability
                .iter()
                .find(|item| item.tool_name == tool_name)
                .expect("tool should exist")
                .enabled
        };

        assert!(find("read"));
        assert!(!find("apply_patch"));
        assert!(!find("task"));
    }

    #[test]
    fn plugin_custom_tool_hooks_prepare_and_mutate_execution() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root).with_plugin_manager(build_plugin_manager());

        assert!(executor.available_tools().iter().any(|tool| {
            tool.name == "plugin_echo"
                && matches!(
                    tool.source,
                    ToolSource::Plugin { ref plugin_name } if plugin_name == "fixture"
                )
        }));

        let invocation = ToolInvocation::Custom {
            name: "plugin_echo".to_string(),
            input: StructuredObject::try_from(json!({ "message": "hello" }))
                .expect("structured object should build"),
        };

        let prepared = executor
            .prepare_invocation(&invocation, 7, 9)
            .expect("prepare should succeed");
        assert_eq!(
            prepared.title_override.as_deref(),
            Some("Prepared plugin echo")
        );

        let prepared_value = match &prepared.invocation {
            ToolInvocation::Custom { input, .. } => serde_json::Value::from(input.clone()),
            other => panic!("expected custom invocation, got {other:?}"),
        };
        assert_eq!(prepared_value["message"], "hello prepared");

        let execution = executor
            .execute_invocation_detailed(&prepared.invocation, 7, 9)
            .expect("plugin execution should succeed");

        match execution.output {
            ToolOutput::Custom { output } => {
                let payload = serde_json::Value::from(output.payload);
                assert_eq!(output.name, "plugin_echo");
                assert_eq!(payload["echoed"], "hello prepared");
                assert_eq!(payload["after"], true);
            }
            other => panic!("expected custom output, got {other:?}"),
        }

        assert_eq!(execution.view.title, "Plugin echo after");
        assert_eq!(execution.view.output_text, "hello prepared after");
        assert_eq!(
            execution
                .view
                .metadata
                .get("after_hook")
                .map(String::as_str),
            Some("applied")
        );
    }

    #[test]
    fn bash_invocation_applies_plugin_shell_env_overrides() {
        if cfg!(windows) {
            return;
        }

        let workspace = TempWorkspace::new();
        let executor = build_executor_with_policy(
            &workspace.root,
            ExecutionPolicy::read_only(),
        )
        .with_plugin_manager(build_plugin_manager());

        let execution = executor
            .execute_invocation_detailed(
                &BuiltinToolInput::Bash(BashToolInput {
                    command: "printf %s \"$PLUGIN_FLAG\"".to_string(),
                    description: "print plugin env".to_string(),
                    timeout_ms: Some(30_000),
                    workdir: None,
                })
                .into_invocation(),
                10,
                11,
            )
            .expect("bash invocation should succeed");

        match execution.output.as_builtin() {
            Some(BuiltinToolOutput::Bash {
                output,
                description: _,
            }) => {
                assert_eq!(output.as_deref(), Some("from_plugin"));
            }
            other => panic!("expected bash output, got {other:?}"),
        }
    }
}
