mod apply_patch;
mod bash;
mod catalog;
mod definition;
mod edit;
mod glob;
mod grep;
mod orchestrator;
mod read;
mod result;
mod task;
mod truncation;
mod write;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use thiserror::Error;

use crate::agent::Agent;
use crate::message::{
    BuiltinToolInput, BuiltinToolOutput, CustomToolOutput, StructuredObject, ToolInvocation,
    ToolOutput,
};
use crate::permission::{
    AccessKind, PermissionAction, PermissionDecision, PermissionRuleStore, PermissionRuntime,
    PermissionRuntimeDecision,
};
use crate::plugin::{
    PluginAfterToolRequest, PluginBeforeToolRequest, PluginManager, PluginShellEnvRequest,
    PluginToolCallRequest,
};
use crate::session::{InMemorySubtaskSessionManager, SubtaskSessionManager};
use procwarden::{
    SandboxCommandRequest, SandboxError, SandboxExecOutput, SandboxManager, SandboxPolicy,
};

pub use apply_patch::{AppliedFileChange, ApplyPatchExecution};
pub use catalog::{ModelToolProfile, ToolAvailability, ToolCatalog};
pub use definition::{ToolBehavior, ToolDefinition, ToolLoadPriority, ToolSource};
pub use result::{BuiltinExecution, ToolExecutionView, ToolInvocationExecution};
pub use truncation::{ToolOutputTruncationPolicy, ToolOutputTruncator};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPermissionCheck {
    pub action: PermissionAction,
    pub decision: PermissionDecision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedToolInvocation {
    pub invocation: ToolInvocation,
    pub title_override: Option<String>,
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
    #[error("invalid patch: {0}")]
    InvalidPatch(String),
    #[error("invalid tool input: {0}")]
    InvalidInput(String),
    #[error("invalid glob pattern: {0}")]
    InvalidGlobPattern(#[from] globset::Error),
    #[error("invalid regex pattern: {0}")]
    InvalidRegexPattern(#[from] regex::Error),
    #[error("sandbox error: {0}")]
    Sandbox(#[from] SandboxError),
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
    truncator: ToolOutputTruncator,
    sandbox_policy: SandboxPolicy,
    sandbox_manager: SandboxManager,
    plugins: Arc<PluginManager>,
    permission_mode: PermissionExecutionMode,
}

impl ToolExecutor {
    pub fn new(workspace_root: impl Into<PathBuf>, agent: Agent) -> Self {
        Self::with_sandbox_policy(
            workspace_root,
            agent,
            SandboxPolicy::new_workspace_write_policy(),
        )
    }

    pub fn with_sandbox_policy(
        workspace_root: impl Into<PathBuf>,
        agent: Agent,
        sandbox_policy: SandboxPolicy,
    ) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            agent,
            model_id: None,
            subtask_manager: Arc::new(InMemorySubtaskSessionManager::new()),
            truncator: ToolOutputTruncator::default(),
            sandbox_policy,
            sandbox_manager: SandboxManager::new(),
            plugins: Arc::new(PluginManager::default()),
            permission_mode: PermissionExecutionMode::Enforced,
        }
    }

    pub fn with_subtask_manager(mut self, manager: Arc<dyn SubtaskSessionManager>) -> Self {
        self.subtask_manager = manager;
        self
    }

    pub fn with_model_id(mut self, model_id: impl Into<String>) -> Self {
        self.model_id = Some(model_id.into());
        self
    }

    pub fn with_plugin_manager(mut self, manager: Arc<PluginManager>) -> Self {
        self.plugins = manager;
        self
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

    pub fn sandbox_policy(&self) -> &SandboxPolicy {
        &self.sandbox_policy
    }

    pub fn subtask_manager(&self) -> &Arc<dyn SubtaskSessionManager> {
        &self.subtask_manager
    }

    pub fn plugin_manager(&self) -> &Arc<PluginManager> {
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
            BuiltinToolInput::Write(crate::message::WriteToolInput {
                file_path: String::new(),
                content: String::new(),
            }),
            BuiltinToolInput::Edit(crate::message::EditToolInput {
                file_path: String::new(),
                old_string: String::new(),
                new_string: String::new(),
                replace_all: false,
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
                subagent_type: String::new(),
                task_id: None,
                command: None,
            }),
        ]
        .into_iter()
        .map(|input| catalog.availability_for_input(&self.agent, &input))
        .collect()
    }

    pub fn available_tools(&self) -> Vec<ToolDefinition> {
        let catalog = self.tool_catalog();
        let mut definitions = self
            .plugins
            .plugins()
            .iter()
            .flat_map(|plugin| {
                plugin.tools.iter().filter_map(|descriptor| {
                    catalog.is_behavior_enabled(descriptor.behavior).then(|| {
                        ToolDefinition::plugin(
                            descriptor.name.clone(),
                            descriptor.description.clone(),
                            descriptor.input_schema.clone(),
                            descriptor.behavior,
                            plugin.metadata.name.clone(),
                        )
                    })
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
        definitions
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

        let execution = orchestrator::execute_builtin(self, input, context)?;
        Ok(self.truncator.apply(execution))
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
            BuiltinToolInput::Write(payload) => {
                let target = self.resolve_target_path(&payload.file_path);
                self.push_path_checks(&mut checks, AccessKind::Write, &target);
            }
            BuiltinToolInput::Edit(payload) => {
                let target = self.resolve_target_path(&payload.file_path);
                self.push_path_checks(&mut checks, AccessKind::Write, &target);
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
            BuiltinToolInput::Task(_) => {}
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

        let hook = self
            .plugins
            .apply_before_tool_hooks(PluginBeforeToolRequest {
                tool_name: tool_name.clone(),
                source: source.clone(),
                session_id,
                call_id,
                input_json,
            })
            .map_err(|err| ToolError::Plugin(err.message))?;

        Ok(PreparedToolInvocation {
            invocation: parse_invocation_from_json(
                tool_name.as_str(),
                hook.input_json.as_str(),
                &source,
            )?,
            title_override: hook.title_override,
        })
    }

    pub fn collect_permission_checks_for_invocation(
        &self,
        invocation: &ToolInvocation,
    ) -> Result<Vec<ToolPermissionCheck>, ToolError> {
        match invocation {
            ToolInvocation::Builtin { input } => self.collect_permission_checks(input),
            ToolInvocation::Custom { name, .. } => {
                let descriptor = self
                    .plugins
                    .custom_tool(name.as_str())
                    .ok_or_else(|| ToolError::UnknownTool(name.clone()))?
                    .1;

                if !self.tool_catalog().is_behavior_enabled(descriptor.behavior) {
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
            ToolInvocation::Mcp { server, tool, .. } => Err(ToolError::UnsupportedInvocation(
                format!("mcp:{server}:{tool}"),
            )),
        }
    }

    pub fn execute_invocation_detailed(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<ToolInvocationExecution, ToolError> {
        match invocation {
            ToolInvocation::Builtin { input } => {
                let mut execution: ToolInvocationExecution = self
                    .execute_builtin_detailed_with_context(
                        input,
                        BuiltinExecutionContext {
                            session_id: Some(session_id),
                            call_id: Some(call_id),
                        },
                    )?
                    .into();
                self.apply_after_hooks(invocation, session_id, call_id, &mut execution)?;
                Ok(execution)
            }
            ToolInvocation::Custom { name, input } => {
                let (plugin, _descriptor) = self
                    .plugins
                    .custom_tool(name.as_str())
                    .ok_or_else(|| ToolError::UnknownTool(name.clone()))?;

                let payload_json = serde_json::to_string(&serde_json::Value::from(input.clone()))
                    .map_err(|err| ToolError::InvalidInput(err.to_string()))?;
                let response = plugin
                    .invoke_tool(PluginToolCallRequest {
                        tool_name: name.clone(),
                        session_id,
                        call_id,
                        workspace_root: self.workspace_root.to_string_lossy().to_string(),
                        input_json: payload_json,
                    })
                    .map_err(|err| ToolError::Plugin(err.message))?;

                let payload = response
                    .payload_json
                    .as_deref()
                    .map(parse_custom_payload)
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
            ToolInvocation::Mcp { server, tool, .. } => Err(ToolError::UnsupportedInvocation(
                format!("mcp:{server}:{tool}"),
            )),
        }
    }

    pub fn shell_env_overrides(
        &self,
        cwd: &Path,
        session_id: Option<i64>,
        call_id: Option<i64>,
    ) -> Result<std::collections::HashMap<String, String>, ToolError> {
        Ok(self
            .plugins
            .shell_env(PluginShellEnvRequest {
                cwd: cwd.to_string_lossy().to_string(),
                session_id,
                call_id,
            })
            .map_err(|err| ToolError::Plugin(err.message))?
            .env
            .into_iter()
            .collect())
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
        match runtime.decide_or_request(session_id, action, base) {
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
        let mut payload_json = match &execution.output {
            ToolOutput::Custom { output } => Some(
                serde_json::to_string(&serde_json::Value::from(output.payload.clone()))
                    .map_err(|err| ToolError::InvalidInput(err.to_string()))?,
            ),
            _ => None,
        };

        let hook = self
            .plugins
            .apply_after_tool_hooks(PluginAfterToolRequest {
                tool_name,
                source,
                session_id,
                call_id,
                title: execution.view.title.clone(),
                output_text: execution.view.output_text.clone(),
                payload_json: payload_json.clone(),
                metadata: execution.view.metadata.clone().into_iter().collect(),
            })
            .map_err(|err| ToolError::Plugin(err.message))?;

        if let Some(title) = hook.title {
            execution.view.title = title;
        }
        if let Some(output_text) = hook.output_text {
            execution.view.output_text = output_text;
        }
        if let Some(next_payload_json) = hook.payload_json {
            payload_json = Some(next_payload_json);
        }
        execution.view.metadata.extend(hook.metadata);

        if let (Some(payload_json), ToolOutput::Custom { output }) =
            (payload_json, &mut execution.output)
        {
            output.payload = parse_custom_payload(payload_json.as_str())?;
        }

        Ok(())
    }

    pub(crate) fn resolve_target_path(&self, raw_path: &str) -> PathBuf {
        let candidate = PathBuf::from(raw_path);
        if candidate.is_absolute() {
            candidate
        } else {
            self.workspace_root.join(candidate)
        }
    }

    pub(crate) fn execute_sandboxed_command(
        &self,
        request: &SandboxCommandRequest,
    ) -> Result<SandboxExecOutput, ToolError> {
        self.sandbox_manager
            .execute(request, self.sandbox_policy(), self.workspace_root())
            .map_err(ToolError::from)
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

fn invocation_name(invocation: &ToolInvocation) -> &str {
    match invocation {
        ToolInvocation::Builtin { input } => crate::permission::builtin_name(input),
        ToolInvocation::Custom { name, .. } => name.as_str(),
        ToolInvocation::Mcp { tool, .. } => tool.as_str(),
    }
}

fn invocation_source(invocation: &ToolInvocation, plugins: &PluginManager) -> ToolSource {
    match invocation {
        ToolInvocation::Builtin { .. } => ToolSource::Builtin,
        ToolInvocation::Custom { name, .. } => plugins
            .custom_tool(name.as_str())
            .map(|(plugin, _)| ToolSource::Plugin {
                plugin_name: plugin.metadata.name.clone(),
            })
            .unwrap_or_else(|| ToolSource::Plugin {
                plugin_name: "custom".to_string(),
            }),
        ToolInvocation::Mcp { server, .. } => ToolSource::Plugin {
            plugin_name: format!("mcp:{server}"),
        },
    }
}

fn invocation_input_json(invocation: &ToolInvocation) -> Result<String, ToolError> {
    match invocation {
        ToolInvocation::Builtin { input } => match input {
            BuiltinToolInput::Bash(payload) => serde_json::to_string(payload),
            BuiltinToolInput::Read(payload) => serde_json::to_string(payload),
            BuiltinToolInput::Write(payload) => serde_json::to_string(payload),
            BuiltinToolInput::Edit(payload) => serde_json::to_string(payload),
            BuiltinToolInput::ApplyPatch(payload) => serde_json::to_string(payload),
            BuiltinToolInput::Glob(payload) => serde_json::to_string(payload),
            BuiltinToolInput::Grep(payload) => serde_json::to_string(payload),
            BuiltinToolInput::Task(payload) => serde_json::to_string(payload),
        }
        .map_err(|err| ToolError::InvalidInput(err.to_string())),
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
        ToolSource::Builtin => Ok(ToolInvocation::Builtin {
            input: parse_builtin_input(tool_name, input_json)?,
        }),
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
        "write" => Ok(BuiltinToolInput::Write(parse(input_json)?)),
        "edit" => Ok(BuiltinToolInput::Edit(parse(input_json)?)),
        "apply_patch" => Ok(BuiltinToolInput::ApplyPatch(parse(input_json)?)),
        "glob" => Ok(BuiltinToolInput::Glob(parse(input_json)?)),
        "grep" => Ok(BuiltinToolInput::Grep(parse(input_json)?)),
        "task" => Ok(BuiltinToolInput::Task(parse(input_json)?)),
        other => Err(ToolError::UnknownTool(other.to_string())),
    }
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

    use serde_json::json;
    use uuid::Uuid;

    use crate::message::{
        BashToolInput, BuiltinToolInput, BuiltinToolOutput, EditToolInput, GlobToolInput,
        GrepToolInput, ReadToolInput, StructuredObject, TaskToolInput, ToolInvocation, ToolOutput,
        WriteToolInput,
    };
    use crate::permission::PermissionPolicy;
    use crate::plugin::{
        AgenaPlugin, PluginAfterToolRequest, PluginAfterToolResponse, PluginBeforeToolRequest,
        PluginBeforeToolResponse, PluginError, PluginManager, PluginMetadata,
        PluginShellEnvRequest, PluginShellEnvResponse, PluginToolCallRequest,
        PluginToolCallResponse, PluginToolDescriptor,
    };
    use procwarden::SandboxPolicy;

    use super::{ToolBehavior, ToolExecutor, ToolSource};

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

    fn build_executor_with_policy(root: &Path, policy: SandboxPolicy) -> ToolExecutor {
        let agent = crate::agent::Agent::new("build", PermissionPolicy::allow_all());
        ToolExecutor::with_sandbox_policy(root, agent, policy)
    }

    #[derive(Debug)]
    struct FixturePlugin;

    impl AgenaPlugin for FixturePlugin {
        fn metadata(&self) -> PluginMetadata {
            PluginMetadata {
                name: "fixture".to_string(),
                version: "0.1.0".to_string(),
                description: "fixture plugin".to_string(),
            }
        }

        fn tools(&self) -> Vec<PluginToolDescriptor> {
            vec![PluginToolDescriptor {
                name: "plugin_echo".to_string(),
                description: "Echo a message from the plugin.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "message": { "type": "string" }
                    },
                    "required": ["message"]
                }),
                behavior: ToolBehavior::ReadOnly,
            }]
        }

        fn invoke_tool(
            &self,
            request: PluginToolCallRequest,
        ) -> Result<PluginToolCallResponse, PluginError> {
            let input: serde_json::Value = serde_json::from_str(request.input_json.as_str())
                .map_err(|err| PluginError::new(err.to_string()))?;
            let message = input
                .get("message")
                .and_then(|value| value.as_str())
                .ok_or_else(|| PluginError::new("missing message"))?
                .to_string();

            Ok(PluginToolCallResponse {
                title: "Plugin echo".to_string(),
                output_text: message.clone(),
                payload_json: Some(json!({ "echoed": message }).to_string()),
                metadata: BTreeMap::from([("plugin".to_string(), "fixture".to_string())]),
            })
        }

        fn before_tool(
            &self,
            request: PluginBeforeToolRequest,
        ) -> Result<PluginBeforeToolResponse, PluginError> {
            if request.tool_name != "plugin_echo" {
                return Ok(PluginBeforeToolResponse::passthrough(request.input_json));
            }

            let mut input: serde_json::Value = serde_json::from_str(request.input_json.as_str())
                .map_err(|err| PluginError::new(err.to_string()))?;
            let message = input
                .get("message")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            input["message"] = serde_json::Value::String(format!("{message} prepared"));

            Ok(PluginBeforeToolResponse {
                input_json: input.to_string(),
                title_override: Some("Prepared plugin echo".to_string()),
                metadata: BTreeMap::new(),
            })
        }

        fn after_tool(
            &self,
            request: PluginAfterToolRequest,
        ) -> Result<PluginAfterToolResponse, PluginError> {
            if request.tool_name != "plugin_echo" {
                return Ok(PluginAfterToolResponse::default());
            }

            let mut payload = request
                .payload_json
                .as_deref()
                .map(|raw| serde_json::from_str::<serde_json::Value>(raw))
                .transpose()
                .map_err(|err| PluginError::new(err.to_string()))?
                .unwrap_or_else(|| json!({}));
            payload["after"] = serde_json::Value::Bool(true);

            Ok(PluginAfterToolResponse {
                title: Some(format!("{} after", request.title)),
                output_text: Some(format!("{} after", request.output_text)),
                payload_json: Some(payload.to_string()),
                metadata: BTreeMap::from([("after_hook".to_string(), "applied".to_string())]),
            })
        }

        fn shell_env(
            &self,
            _request: PluginShellEnvRequest,
        ) -> Result<PluginShellEnvResponse, PluginError> {
            Ok(PluginShellEnvResponse {
                env: BTreeMap::from([("PLUGIN_FLAG".to_string(), "from_plugin".to_string())]),
            })
        }
    }

    fn build_plugin_manager() -> Arc<PluginManager> {
        let mut manager = PluginManager::new();
        manager
            .register_static(FixturePlugin)
            .expect("fixture plugin should register");
        Arc::new(manager)
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
    fn write_and_edit_builtins_update_file_content() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        executor
            .execute_builtin_detailed(&BuiltinToolInput::Write(WriteToolInput {
                file_path: "src/app.txt".to_string(),
                content: "hello world\n".to_string(),
            }))
            .expect("write builtin should succeed");

        executor
            .execute_builtin_detailed(&BuiltinToolInput::Edit(EditToolInput {
                file_path: "src/app.txt".to_string(),
                old_string: "world".to_string(),
                new_string: "agena".to_string(),
                replace_all: false,
            }))
            .expect("edit builtin should succeed");

        let current = fs::read_to_string(workspace.root.join("src/app.txt"))
            .expect("failed to read edited file");
        assert_eq!(current, "hello agena\n");
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
                subagent_type: "explore".to_string(),
                task_id: None,
                command: None,
            }))
            .expect("task should succeed");

        match result.output {
            BuiltinToolOutput::Task { session_id, .. } => {
                assert!(session_id.is_some());
            }
            other => panic!("expected task output, got {other:?}"),
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
            SandboxPolicy::new_read_only_policy().with_world_writable_audit(false),
        );

        let result = executor
            .execute_builtin_detailed(&BuiltinToolInput::Bash(BashToolInput {
                command: "echo hello_agena".to_string(),
                description: "smoke bash".to_string(),
                timeout_ms: Some(30_000),
                workdir: None,
            }))
            .expect("bash builtin should succeed");

        match result.output {
            BuiltinToolOutput::Bash {
                output,
                description,
            } => {
                let output = output.expect("output should exist").to_ascii_lowercase();
                assert!(output.contains("hello_agena"));
                assert!(description.is_some());
            }
            other => panic!("expected bash output, got {other:?}"),
        }
    }

    #[test]
    fn readonly_model_profile_disables_write_and_task_tools() {
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
        assert!(!find("write"));
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
            SandboxPolicy::new_read_only_policy().with_world_writable_audit(false),
        )
        .with_plugin_manager(build_plugin_manager());

        let execution = executor
            .execute_invocation_detailed(
                &ToolInvocation::Builtin {
                    input: BuiltinToolInput::Bash(BashToolInput {
                        command: "printf %s \"$PLUGIN_FLAG\"".to_string(),
                        description: "print plugin env".to_string(),
                        timeout_ms: Some(30_000),
                        workdir: None,
                    }),
                },
                10,
                11,
            )
            .expect("bash invocation should succeed");

        match execution.output {
            ToolOutput::Builtin {
                output:
                    BuiltinToolOutput::Bash {
                        output,
                        description: _,
                    },
            } => {
                assert_eq!(output.as_deref(), Some("from_plugin"));
            }
            other => panic!("expected bash output, got {other:?}"),
        }
    }
}
