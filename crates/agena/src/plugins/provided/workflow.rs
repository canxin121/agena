//! `agena.workflow` plugin: orchestration tools (task, tool_search,
//! todo_write, create_goal, get_goal, update_goal, ask_user, enter_plan_mode,
//! exit_plan_mode, enter_worktree, exit_worktree).

use std::sync::{Arc, RwLock};

use agena_macros::StaticToolSurface;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::entry::{
    ToolExecutionView, ToolPayloadExecution, ToolPayloadOutput, ask_user, tool_search,
};
use crate::message::{
    AgentRestoreToolInput, AgentSwitchToolInput, AskUserToolInput, ClearGoalToolInput,
    CreateGoalToolInput, EnterPlanModeToolInput, EnterWorktreeToolInput, ExitPlanModeToolInput,
    ExitWorktreeToolInput, GetGoalToolInput, TaskToolInput, TodoItem, TodoPriority, TodoStatus,
    TodoWriteToolInput, ToolSearchToolInput, WorkflowPromptToolInput,
};
use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::{
    AskUserOption as HostAskUserOption, AskUserQuestion as HostAskUserQuestion, AskUserRequest,
    HostAgentRestoreRequest, HostAgentRestoreResponse, HostAgentSwitchRequest,
    HostAgentSwitchResponse, HostClearGoalRequest, HostClient, HostCreateGoalRequest,
    HostEnterPlanModeRequest, HostEnterWorktreeRequest, HostExitPlanModeRequest,
    HostExitWorktreeRequest, HostGetGoalRequest, HostGetSessionRequest, HostGoal, HostGoalStatus,
    HostRenameSessionRequest, HostSession, HostTodoItem, HostTodoPriority, HostTodoStatus,
    HostTodoWriteRequest, HostUpdateGoalRequest, SpawnSubtaskRequest, ToolDescriptor,
};
use crate::plugin::sdk::{
    HookSubscription, HostCapability, InitContext, InitOutcome, PathAccessSpec, PathKind,
    PathRequest, Plugin, PluginManifest, PluginToolDecl, Result as SdkResult, ToolInvokeInput,
    ToolInvokeOutput, ToolTag,
};

pub(crate) const WORKFLOW_PLUGIN_ID: &str = "agena.workflow";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
struct CompleteGoalToolInput {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    entry = "workflow",
    description = "Workflow scaffold command. Use action `init`, `review`, or `security_review` to generate reusable workflow instructions; this entry does not execute shell or filesystem actions by itself.",
    tags(ToolTag::ReadOnly),
    host_capabilities(HostCapability::AgentRegistry),
    concurrency_safe = true
)]
#[serde(tag = "action", rename_all = "snake_case")]
enum WorkflowToolInput {
    #[tool(exec = "init")]
    Init {
        #[serde(flatten)]
        args: WorkflowPromptToolInput,
    },
    #[tool(exec = "review")]
    Review {
        #[serde(flatten)]
        args: WorkflowPromptToolInput,
    },
    #[tool(exec = "security_review")]
    SecurityReview {
        #[serde(flatten)]
        args: WorkflowPromptToolInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    entry = "tools",
    description = "Tool catalog command. Use action `search` to find tools or `help` to fetch detailed usage for a tool. This entry does not execute the target tool for you.",
    summary = "Search tools or fetch detailed tool help.",
    help = "Use action `search` with `query` and optional `limit` to discover tools. Use action `help` with `tool` to retrieve the full registered help text and input schema for any model-visible tool. To actually run a tool, call that tool directly after reading its help.",
    tags(ToolTag::ReadOnly, ToolTag::Discovery),
    host_capabilities(HostCapability::ListTools),
    concurrency_safe = true
)]
#[serde(tag = "action", rename_all = "snake_case")]
enum ToolsToolInput {
    #[tool(exec = "search")]
    Search {
        #[serde(flatten)]
        args: ToolSearchToolInput,
    },
    #[tool(exec = "help")]
    Help {
        #[serde(flatten)]
        args: ToolsHelpInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
struct ToolsHelpInput {
    pub tool: String,
    #[serde(default = "default_include_schema")]
    pub include_schema: bool,
}

fn default_include_schema() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    entry = "agent",
    description = "Runtime agent profile command. Use action `switch` to change the current session's active agent profile or `restore` to bring back a saved profile. This entry does not spawn delegated subagent work; use `task` for that.",
    host_capabilities(HostCapability::AgentRegistry),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case")]
enum AgentToolInput {
    #[tool(exec = "switch")]
    Switch {
        #[serde(flatten)]
        args: AgentSwitchToolInput,
    },
    #[tool(exec = "restore")]
    Restore {
        #[serde(flatten)]
        args: AgentRestoreToolInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    entry = "todo",
    description = "Todo command. Use action `write` to replace the session todo list.",
    tags(ToolTag::Mutating, ToolTag::Planning),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case")]
enum TodoToolInput {
    #[tool(exec = "write")]
    Write {
        #[serde(flatten)]
        args: TodoWriteToolInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    entry = "task",
    description = "Delegated subagent task command. Use action `run` to create or resume a typed child task session for explore, implement, or verify work. This entry launches/resumes a separate task session; it does not switch the current runtime agent profile.",
    tags(ToolTag::Task, ToolTag::Subtask),
    host_capabilities(HostCapability::SpawnSubtask),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case")]
enum TaskEntryToolInput {
    #[tool(exec = "run")]
    Run {
        #[serde(flatten)]
        args: TaskToolInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
struct SessionRenameToolInput {
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    entry = "session",
    description = "Session metadata command. Use action `get` to inspect the current session metadata or `rename` to update the session title. This entry does not read chat history or execute workflow actions.",
    tags(ToolTag::ReadOnly, ToolTag::Mutating),
    host_capabilities(HostCapability::SessionRegistry),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case")]
enum SessionToolInput {
    #[tool(exec = "get")]
    Get,
    #[tool(exec = "rename")]
    Rename {
        #[serde(flatten)]
        args: SessionRenameToolInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    entry = "goal",
    description = "Goal command. Use action `get`, `create`, `clear`, or `complete`. Use `complete` only once the objective is actually finished.",
    tags(ToolTag::ReadOnly, ToolTag::Mutating, ToolTag::Goal),
    host_capabilities(HostCapability::GoalRegistry),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case")]
enum GoalToolInput {
    #[tool(exec = "get")]
    Get {
        #[serde(flatten)]
        args: GetGoalToolInput,
    },
    #[tool(exec = "create")]
    Create {
        #[serde(flatten)]
        args: CreateGoalToolInput,
    },
    #[tool(exec = "clear")]
    Clear {
        #[serde(flatten)]
        args: ClearGoalToolInput,
    },
    #[tool(exec = "complete")]
    Complete {
        #[serde(flatten)]
        args: CompleteGoalToolInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    entry = "user",
    description = "User interaction command. Use action `request_input` to request structured short answers. Legacy action alias `ask` still works but is not the preferred name.",
    tags(ToolTag::ReadOnly, ToolTag::Interactive),
    host_capabilities(HostCapability::AskUser),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case")]
enum UserToolInput {
    #[serde(alias = "ask")]
    #[tool(exec = "request_input")]
    RequestInput(AskUserToolInput),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    entry = "plan",
    description = "Plan mode command. Use action `enter` or `exit`; `enter` allocates a plan markdown file under .agena/plans/ and `exit` asks the user to approve the plan.",
    tags(ToolTag::ReadOnly, ToolTag::Planning, ToolTag::FilesystemWrite),
    host_capabilities(HostCapability::PlanRegistry, HostCapability::AgentRegistry),
    concurrency_safe = true
)]
#[serde(tag = "action", rename_all = "snake_case")]
enum PlanToolInput {
    #[tool(exec = "enter")]
    Enter {
        #[serde(flatten)]
        args: EnterPlanModeToolInput,
    },
    #[tool(exec = "exit")]
    Exit {
        #[serde(flatten)]
        args: ExitPlanModeToolInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, StaticToolSurface)]
#[tool_surface(
    entry = "worktree",
    description = "Worktree command. Use action `enter` or `exit`; `enter` uses `target = new|existing` to create or attach to a git worktree and `exit` uses enum `exit_action = keep|remove`.",
    tags(ToolTag::Mutating, ToolTag::FilesystemWrite, ToolTag::Worktree),
    host_capabilities(HostCapability::WorktreeRegistry),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case")]
enum WorktreeToolInput {
    #[tool(exec = "enter")]
    Enter {
        #[serde(flatten)]
        args: EnterWorktreeCommandInput,
    },
    #[tool(exec = "exit")]
    Exit {
        #[serde(flatten)]
        args: ExitWorktreeCommandInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(tag = "target", rename_all = "snake_case")]
enum EnterWorktreeCommandInput {
    New {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Existing {
        path: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ExitWorktreeAction {
    Keep,
    Remove,
}

impl ExitWorktreeAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Remove => "remove",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
struct ExitWorktreeCommandInput {
    #[serde(rename = "exit_action", alias = "action")]
    exit_action: ExitWorktreeAction,
    #[serde(default)]
    discard_changes: bool,
}

fn parse_worktree_enter_input(input: serde_json::Value) -> SdkResult<EnterWorktreeToolInput> {
    match serde_json::from_value::<EnterWorktreeCommandInput>(input.clone()) {
        Ok(EnterWorktreeCommandInput::New { name }) => {
            Ok(EnterWorktreeToolInput { name, path: None })
        }
        Ok(EnterWorktreeCommandInput::Existing { path }) => Ok(EnterWorktreeToolInput {
            name: None,
            path: Some(path),
        }),
        Err(primary) => {
            let primary = PluginError::invalid_params(primary.to_string());
            serde_json::from_value::<EnterWorktreeToolInput>(input).map_err(|_| primary)
        }
    }
}

fn parse_worktree_exit_input(input: serde_json::Value) -> SdkResult<ExitWorktreeToolInput> {
    match serde_json::from_value::<ExitWorktreeCommandInput>(input.clone()) {
        Ok(parsed) => Ok(ExitWorktreeToolInput {
            action: parsed.exit_action.as_str().to_string(),
            discard_changes: parsed.discard_changes,
        }),
        Err(primary) => {
            let primary = PluginError::invalid_params(primary.to_string());
            serde_json::from_value::<ExitWorktreeToolInput>(input).map_err(|_| primary)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct GoalToolResponse {
    goal: Option<HostGoal>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct SessionToolResponse {
    session: HostSession,
}

impl GoalToolResponse {
    fn new(goal: Option<HostGoal>) -> Self {
        Self { goal }
    }
}

fn goal_status_label(status: HostGoalStatus) -> &'static str {
    match status {
        HostGoalStatus::Active => "active",
        HostGoalStatus::Paused => "paused",
        HostGoalStatus::Completed => "completed",
    }
}

pub(crate) struct WorkflowPlugin {
    host: RwLock<Option<Arc<dyn HostClient>>>,
}

impl WorkflowPlugin {
    pub(crate) fn new() -> Self {
        Self {
            host: RwLock::new(None),
        }
    }

    fn provided_workflow(name: &str) -> SdkResult<agena_skills::Skill> {
        agena_skills::bundled::all()
            .map_err(|err| PluginError::new(err.to_string()))?
            .into_iter()
            .find(|skill| skill.frontmatter.name == name)
            .ok_or_else(|| PluginError::new(format!("missing default workflow '{name}'")))
    }

    fn render_workflow_prompt(body: &str, args: &str) -> String {
        let body = body.trim();
        let args = args.trim();
        if body.contains("$ARGUMENTS") {
            return body.replace("$ARGUMENTS", args);
        }
        if args.is_empty() {
            body.to_string()
        } else {
            format!("{body}\n\nUser arguments:\n{args}")
        }
    }

    async fn invoke_provided_workflow(
        &self,
        workflow_name: &str,
        input: &WorkflowPromptToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let workflow = Self::provided_workflow(workflow_name)?;
        let prompt = Self::render_workflow_prompt(
            workflow.body.as_str(),
            input.args.as_deref().unwrap_or_default(),
        );
        Ok(ToolInvokeOutput::text(prompt)
            .with_title(format!("{} workflow", workflow.frontmatter.name))
            .with_metadata("workflow", workflow.frontmatter.name))
    }

    async fn invoke_agent_workflow(
        &self,
        workflow_name: &str,
        agent_name: &str,
        input: &WorkflowPromptToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let switch = self
            .host()?
            .agent_switch(HostAgentSwitchRequest {
                agent: Some(agent_name.to_string()),
                session_id: None,
                push_previous: true,
            })
            .await?;
        let mut output = self.invoke_provided_workflow(workflow_name, input).await?;
        output = output
            .with_metadata("workflow_agent", agent_name)
            .with_metadata("agent_stack_depth", switch.stack_depth.to_string());
        if let Some(previous) = switch.previous_agent {
            output = output.with_metadata("previous_agent", previous);
        }
        Ok(output)
    }

    async fn switch_agent_for_tool(
        &self,
        agent: Option<String>,
        push_previous: bool,
    ) -> SdkResult<HostAgentSwitchResponse> {
        self.host()?
            .agent_switch(HostAgentSwitchRequest {
                agent,
                session_id: None,
                push_previous,
            })
            .await
    }

    async fn restore_agent_for_tool(&self) -> SdkResult<HostAgentRestoreResponse> {
        self.host()?
            .agent_restore(HostAgentRestoreRequest { session_id: None })
            .await
    }

    fn host(&self) -> SdkResult<Arc<dyn HostClient>> {
        self.host
            .read()
            .map_err(|_| PluginError::new("workflow plugin host lock poisoned"))?
            .clone()
            .ok_or_else(|| PluginError::new("workflow plugin invoked before init"))
    }

    fn host_ask_user_questions(input: &AskUserToolInput) -> Vec<HostAskUserQuestion> {
        input
            .questions
            .iter()
            .map(|question| HostAskUserQuestion {
                id: question.id.clone(),
                header: question.header.clone(),
                question: question.question.clone(),
                options: question
                    .options
                    .iter()
                    .map(|option| HostAskUserOption {
                        label: option.label.clone(),
                        description: option.description.clone(),
                    })
                    .collect(),
                multiple: question.multiple,
                allow_custom: question.allow_custom,
            })
            .collect()
    }

    fn searchable_tool_from_descriptor(descriptor: ToolDescriptor) -> tool_search::SearchableTool {
        tool_search::SearchableTool {
            name: descriptor.name,
            description: descriptor
                .summary
                .or(descriptor.description)
                .unwrap_or_default(),
            tags: descriptor.tags,
        }
    }

    fn host_todo_item(item: &TodoItem) -> HostTodoItem {
        HostTodoItem {
            content: item.content.clone(),
            status: match item.status {
                TodoStatus::Pending => HostTodoStatus::Pending,
                TodoStatus::InProgress => HostTodoStatus::InProgress,
                TodoStatus::Completed => HostTodoStatus::Completed,
                TodoStatus::Cancelled => HostTodoStatus::Cancelled,
            },
            priority: match item.priority {
                TodoPriority::High => HostTodoPriority::High,
                TodoPriority::Medium => HostTodoPriority::Medium,
                TodoPriority::Low => HostTodoPriority::Low,
            },
        }
    }

    fn goal_payload_text(payload: &serde_json::Value) -> String {
        serde_json::to_string_pretty(payload).unwrap_or_else(|_| payload.to_string())
    }

    fn goal_tool_payload(goal: Option<HostGoal>) -> SdkResult<serde_json::Value> {
        serde_json::to_value(GoalToolResponse::new(goal))
            .map_err(|err| PluginError::new(err.to_string()))
    }

    fn goal_summary(goal: &HostGoal) -> String {
        format!("Goal {} is {}.", goal.id, goal_status_label(goal.status),)
    }

    fn session_tool_payload(session: HostSession) -> SdkResult<serde_json::Value> {
        serde_json::to_value(SessionToolResponse { session })
            .map_err(|err| PluginError::new(err.to_string()))
    }

    fn session_summary(session: &HostSession) -> String {
        let mut parts = vec![format!("Session #{} title: {}", session.id, session.title)];
        if let Some(parent_id) = session.parent_id {
            parts.push(format!("parent #{parent_id}"));
        }
        if session.root_id != session.id {
            parts.push(format!("root #{}", session.root_id));
        }
        if session.is_subagent {
            parts.push("subagent".to_string());
        }
        parts.join(" | ")
    }

    async fn invoke_get_session(&self) -> SdkResult<ToolInvokeOutput> {
        let response = self
            .host()?
            .get_session(HostGetSessionRequest::default())
            .await?;
        let payload = Self::session_tool_payload(response.session.clone())?;
        Ok(ToolInvokeOutput::text(format!(
            "{}\n\n{}",
            Self::session_summary(&response.session),
            Self::goal_payload_text(&payload)
        ))
        .with_title("session")
        .with_payload(payload))
    }

    async fn invoke_rename_session(
        &self,
        input: &SessionRenameToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let title = input.title.trim();
        if title.is_empty() {
            return Err(PluginError::invalid_params(
                "session rename requires a non-empty title",
            ));
        }
        let response = self
            .host()?
            .rename_session(HostRenameSessionRequest {
                session_id: None,
                title: title.to_string(),
            })
            .await?;
        let payload = Self::session_tool_payload(response.session.clone())?;
        Ok(ToolInvokeOutput::text(format!(
            "Renamed session #{} to {}.\n\n{}",
            response.session.id,
            response.session.title,
            Self::goal_payload_text(&payload)
        ))
        .with_title("session")
        .with_payload(payload))
    }

    async fn invoke_get_goal(&self, _input: &GetGoalToolInput) -> SdkResult<ToolInvokeOutput> {
        let response = self.host()?.get_goal(HostGetGoalRequest::default()).await?;
        let payload = Self::goal_tool_payload(response.goal.clone())?;
        let text = match response.goal.as_ref() {
            Some(goal) => format!(
                "{}\n\n{}",
                Self::goal_summary(goal),
                Self::goal_payload_text(&payload)
            ),
            None => Self::goal_payload_text(&payload),
        };
        Ok(ToolInvokeOutput::text(text)
            .with_title("goal")
            .with_payload(payload))
    }

    async fn invoke_create_goal(&self, input: &CreateGoalToolInput) -> SdkResult<ToolInvokeOutput> {
        let response = self
            .host()?
            .create_goal(HostCreateGoalRequest {
                objective: input.objective.clone(),
            })
            .await?;
        let payload = Self::goal_tool_payload(Some(response.goal.clone()))?;
        Ok(ToolInvokeOutput::text(format!(
            "{}\n\n{}",
            Self::goal_summary(&response.goal),
            Self::goal_payload_text(&payload)
        ))
        .with_title("goal")
        .with_payload(payload))
    }

    async fn invoke_clear_goal(&self, _input: &ClearGoalToolInput) -> SdkResult<ToolInvokeOutput> {
        let response = self
            .host()?
            .clear_goal(HostClearGoalRequest::default())
            .await?;
        let payload =
            serde_json::to_value(&response).map_err(|err| PluginError::new(err.to_string()))?;
        let text = if response.cleared {
            format!(
                "Cleared the current goal.\n\n{}",
                Self::goal_payload_text(&payload)
            )
        } else {
            format!(
                "No current goal to clear.\n\n{}",
                Self::goal_payload_text(&payload)
            )
        };
        Ok(ToolInvokeOutput::text(text)
            .with_title("goal")
            .with_payload(payload))
    }

    async fn invoke_complete_goal(
        &self,
        _input: &CompleteGoalToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let response = self
            .host()?
            .update_goal(HostUpdateGoalRequest {
                objective: None,
                status: Some(HostGoalStatus::Completed),
            })
            .await?;
        let payload = Self::goal_tool_payload(Some(response.goal.clone()))?;
        Ok(ToolInvokeOutput::text(format!(
            "{}\n\n{}",
            Self::goal_summary(&response.goal),
            Self::goal_payload_text(&payload)
        ))
        .with_title("goal")
        .with_payload(payload))
    }

    async fn invoke_ask_user(&self, input: &AskUserToolInput) -> SdkResult<ToolInvokeOutput> {
        ask_user::validate(input).map_err(|err| PluginError::invalid_params(err.to_string()))?;
        let host = self.host()?;
        let response = host
            .ask_user(AskUserRequest {
                questions: Self::host_ask_user_questions(input),
                prompt: String::new(),
                options: Vec::new(),
                allow_free_text: false,
            })
            .await?;
        if response.cancelled {
            let reason = if response.reply.trim().is_empty() {
                "user declined to answer requested questions".to_string()
            } else {
                response.reply
            };
            return Err(PluginError::new(reason));
        }

        let mut answers = response.answers;
        if answers.is_empty()
            && let Some(question) = input.questions.first()
            && !response.reply.trim().is_empty()
        {
            answers.insert(question.id.clone(), vec![response.reply]);
        }

        let execution = ask_user::execution_from_answers(input, answers);
        Ok(crate::plugins::provided::router::tool_execution_to_invoke_output(execution))
    }

    async fn invoke_task(&self, input: &TaskToolInput) -> SdkResult<ToolInvokeOutput> {
        let host = self.host()?;
        let response = host
            .spawn_subtask(SpawnSubtaskRequest {
                subagent_type: input.subagent_type.to_string(),
                description: input.description.clone(),
                prompt: input.prompt.clone(),
                task_id: input.task_id.clone(),
                command: input.command.clone(),
                model: None,
            })
            .await?;

        let session_id = response.metadata.get("session_id").cloned();
        let model_provider_id = response.metadata.get("model_provider_id").cloned();
        let model_id = response.metadata.get("model_id").cloned();
        let output_text = if response.final_text.trim().is_empty() {
            format!(
                "Created/resumed subtask session {} for profile '{}' in workspace {}.",
                session_id.as_deref().unwrap_or("unknown"),
                input.subagent_type,
                "<unknown>"
            )
        } else {
            response.final_text
        };
        let mut view = ToolExecutionView::simple(
            format!("Task {} ({})", input.description, input.subagent_type),
            output_text,
        );
        view.metadata
            .insert("description".to_string(), input.description.clone());
        view.metadata
            .insert("subagent_type".to_string(), input.subagent_type.to_string());
        view.metadata.insert(
            "profile_guidance".to_string(),
            input.subagent_type.guidance().to_string(),
        );
        if let Some(session_id_value) = session_id.clone() {
            view.metadata
                .insert("session_id".to_string(), session_id_value);
        }
        if let Some(command) = input.command.clone() {
            view.metadata.insert("command".to_string(), command);
        }
        if let Some(model_provider_id) = model_provider_id.clone() {
            view.metadata
                .insert("model_provider_id".to_string(), model_provider_id);
        }
        if let Some(model_id) = model_id.clone() {
            view.metadata.insert("model_id".to_string(), model_id);
        }
        for (key, value) in response.metadata {
            view.metadata.entry(key).or_insert(value);
        }

        let output = ToolPayloadOutput::Task {
            session_id,
            model_provider_id,
            model_id,
        };
        Ok(
            crate::plugins::provided::router::tool_execution_to_invoke_output(
                ToolPayloadExecution::new(output, view),
            ),
        )
    }

    async fn invoke_agent_switch(
        &self,
        input: &AgentSwitchToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let response = self
            .switch_agent_for_tool(input.agent.clone(), input.push_previous)
            .await?;
        let payload =
            serde_json::to_value(&response).map_err(|err| PluginError::new(err.to_string()))?;
        let current = response
            .current_agent
            .as_deref()
            .unwrap_or("default runtime context");
        let previous = response.previous_agent.as_deref().unwrap_or("none");
        Ok(ToolInvokeOutput::text(format!(
            "Switched session {} agent to {current}. Previous agent: {previous}.",
            response.session_id
        ))
        .with_title("agent switch")
        .with_payload(payload))
    }

    async fn invoke_agent_restore(
        &self,
        _input: &AgentRestoreToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let response = self.restore_agent_for_tool().await?;
        let payload =
            serde_json::to_value(&response).map_err(|err| PluginError::new(err.to_string()))?;
        let text = if response.restored {
            let current = response
                .current_agent
                .as_deref()
                .unwrap_or("default runtime context");
            let previous = response.previous_agent.as_deref().unwrap_or("none");
            format!(
                "Restored session {} agent to {current}. Previous agent: {previous}.",
                response.session_id
            )
        } else {
            format!(
                "No agent restore point is available for session {}.",
                response.session_id
            )
        };
        Ok(ToolInvokeOutput::text(text)
            .with_title("agent restore")
            .with_payload(payload))
    }

    async fn invoke_tool_search(&self, input: &ToolSearchToolInput) -> SdkResult<ToolInvokeOutput> {
        let host = self.host()?;
        let catalog = host
            .list_tools()
            .await?
            .into_iter()
            .map(Self::searchable_tool_from_descriptor)
            .collect::<Vec<_>>();
        let execution = tool_search::execute_with_tools(&catalog, input)
            .map_err(|err| PluginError::invalid_params(err.to_string()))?;
        Ok(crate::plugins::provided::router::tool_execution_to_invoke_output(execution))
    }

    async fn invoke_tool_help(&self, input: &ToolsHelpInput) -> SdkResult<ToolInvokeOutput> {
        let requested = input.tool.trim();
        if requested.is_empty() {
            return Err(PluginError::invalid_params(
                "tools help requires a non-empty tool name",
            ));
        }
        let tools = self.host()?.list_tools().await?;
        let mut exact = None;
        let mut case_insensitive = None;
        for tool in tools {
            if tool.name == requested {
                exact = Some(tool);
                break;
            }
            if case_insensitive.is_none() && tool.name.eq_ignore_ascii_case(requested) {
                case_insensitive = Some(tool);
            }
        }
        let Some(descriptor) = exact.or(case_insensitive) else {
            return Err(PluginError::invalid_params(format!(
                "unknown tool '{requested}'"
            )));
        };

        let mut lines = vec![format!("Tool: {}", descriptor.name)];
        if let Some(plugin_id) = descriptor.plugin_id.as_deref() {
            lines.push(format!("Plugin: {plugin_id}"));
        }
        if !descriptor.tags.is_empty() {
            let tags = descriptor
                .tags
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!("Tags: {tags}"));
        }
        if let Some(summary) = descriptor
            .summary
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            lines.push(format!("Summary: {summary}"));
        }
        if let Some(description) = descriptor
            .description
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            lines.push(format!("Description: {description}"));
        }
        if let Some(help) = descriptor.help.as_deref().filter(|value| !value.is_empty()) {
            lines.push("Help:".to_string());
            lines.push(help.to_string());
        }
        if input.include_schema
            && let Some(schema) = descriptor.input_schema.as_ref()
        {
            lines.push("Input schema:".to_string());
            lines.push(
                serde_json::to_string_pretty(schema)
                    .map_err(|err| PluginError::new(err.to_string()))?,
            );
        }

        Ok(
            ToolInvokeOutput::text(lines.join("\n"))
                .with_title(format!("{} help", descriptor.name)),
        )
    }
}

pub(crate) fn new_plugin() -> WorkflowPlugin {
    WorkflowPlugin::new()
}

#[async_trait]
impl Plugin for WorkflowPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder("agena-workflow", env!("CARGO_PKG_VERSION"))
            .description("Workflow orchestration tools.")
            .hooks(HookSubscription::TOOL_INVOKE | HookSubscription::AGENT_STOP)
            .plugin_capability(HostCapability::AgentRegistry)
            .tools(entries())
            .build()
    }

    async fn init(&self, _ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        *self
            .host
            .write()
            .map_err(|_| PluginError::new("workflow plugin host lock poisoned"))? = Some(host);
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        match input.tool_name.as_str() {
            "task" => {
                let (action, action_input) =
                    TaskEntryToolInput::resolve_entry("task", input.input)?;
                match action.as_str() {
                    "run" => {
                        self.invoke_task(
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    other => Err(PluginError::invalid_params(format!(
                        "unknown task action '{other}'"
                    ))),
                }
            }
            "tools" => {
                let (action, action_input) = ToolsToolInput::resolve_entry("tools", input.input)?;
                match action.as_str() {
                    "search" => {
                        self.invoke_tool_search(
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    "help" => {
                        self.invoke_tool_help(
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    other => Err(PluginError::invalid_params(format!(
                        "unknown tools action '{other}'"
                    ))),
                }
            }
            "agent" => {
                let (action, action_input) = AgentToolInput::resolve_entry("agent", input.input)?;
                match action.as_str() {
                    "switch" => {
                        self.invoke_agent_switch(
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    "restore" => {
                        self.invoke_agent_restore(
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    other => Err(PluginError::invalid_params(format!(
                        "unknown agent action '{other}'"
                    ))),
                }
            }
            "todo" => {
                let (action, action_input) = TodoToolInput::resolve_entry("todo", input.input)?;
                match action.as_str() {
                    "write" => {
                        let args: TodoWriteToolInput = serde_json::from_value(action_input)
                            .map_err(|err| PluginError::invalid_params(err.to_string()))?;
                        self.host()?
                            .todo_write(HostTodoWriteRequest {
                                items: args.items.iter().map(Self::host_todo_item).collect(),
                            })
                            .await
                    }
                    other => Err(PluginError::invalid_params(format!(
                        "unknown todo action '{other}'"
                    ))),
                }
            }
            "session" => {
                let (action, action_input) =
                    SessionToolInput::resolve_entry(input.tool_name.as_str(), input.input)?;
                match action.as_str() {
                    "get" => self.invoke_get_session().await,
                    "rename" => {
                        self.invoke_rename_session(
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    other => Err(PluginError::invalid_params(format!(
                        "unknown session action '{other}'"
                    ))),
                }
            }
            "goal" => {
                let (action, action_input) =
                    GoalToolInput::resolve_entry(input.tool_name.as_str(), input.input)?;
                match action.as_str() {
                    "get" => {
                        self.invoke_get_goal(
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    "create" => {
                        self.invoke_create_goal(
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    "clear" => {
                        self.invoke_clear_goal(
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    "complete" => {
                        self.invoke_complete_goal(
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    other => Err(PluginError::invalid_params(format!(
                        "unknown goal action '{other}'"
                    ))),
                }
            }
            "user" => {
                let (action, action_input) = UserToolInput::resolve_entry("user", input.input)?;
                match action.as_str() {
                    "request_input" => {
                        self.invoke_ask_user(
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    other => Err(PluginError::invalid_params(format!(
                        "unknown user action '{other}'"
                    ))),
                }
            }
            "plan" => {
                let (action, _action_input) = PlanToolInput::resolve_entry("plan", input.input)?;
                match action.as_str() {
                    "enter" => {
                        let switch = self
                            .switch_agent_for_tool(Some("planner".to_string()), true)
                            .await?;
                        let mut output = self
                            .host()?
                            .enter_plan_mode(HostEnterPlanModeRequest::default())
                            .await?;
                        output = output
                            .with_metadata("workflow_agent", "planner")
                            .with_metadata("agent_stack_depth", switch.stack_depth.to_string());
                        if let Some(previous) = switch.previous_agent {
                            output = output.with_metadata("previous_agent", previous);
                        }
                        Ok(output)
                    }
                    "exit" => {
                        let mut output = self
                            .host()?
                            .exit_plan_mode(HostExitPlanModeRequest::default())
                            .await?;
                        let restore = self.restore_agent_for_tool().await?;
                        output = output
                            .with_metadata("agent_restored", restore.restored.to_string())
                            .with_metadata("agent_stack_depth", restore.stack_depth.to_string());
                        if let Some(current) = restore.current_agent {
                            output = output.with_metadata("current_agent", current);
                        }
                        Ok(output)
                    }
                    other => Err(PluginError::invalid_params(format!(
                        "unknown plan action '{other}'"
                    ))),
                }
            }
            "worktree" => {
                let (action, action_input) =
                    WorktreeToolInput::resolve_entry("worktree", input.input)?;
                match action.as_str() {
                    "enter" => {
                        let args = parse_worktree_enter_input(action_input)?;
                        self.host()?
                            .enter_worktree(HostEnterWorktreeRequest {
                                name: args.name,
                                path: args.path,
                            })
                            .await
                    }
                    "exit" => {
                        let args = parse_worktree_exit_input(action_input)?;
                        self.host()?
                            .exit_worktree(HostExitWorktreeRequest {
                                action: args.action,
                                discard_changes: args.discard_changes,
                            })
                            .await
                    }
                    other => Err(PluginError::invalid_params(format!(
                        "unknown worktree action '{other}'"
                    ))),
                }
            }
            "workflow" => {
                let (action, action_input) =
                    WorkflowToolInput::resolve_entry("workflow", input.input)?;
                match action.as_str() {
                    "init" => {
                        self.invoke_provided_workflow(
                            "init",
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    "review" => {
                        self.invoke_agent_workflow(
                            "review",
                            "reviewer",
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    "security_review" => {
                        self.invoke_agent_workflow(
                            "security_review",
                            "reviewer",
                            &serde_json::from_value(action_input)
                                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
                        )
                        .await
                    }
                    other => Err(PluginError::invalid_params(format!(
                        "unknown workflow action '{other}'"
                    ))),
                }
            }
            other => Err(PluginError::invalid_params(format!(
                "unknown workflow plugin tool '{other}'"
            ))),
        }
    }

    async fn permission_paths(
        &self,
        tool_name: &str,
        input: &serde_json::Value,
    ) -> SdkResult<Vec<PathRequest>> {
        match tool_name {
            "worktree" => {
                let (action, action_input) =
                    WorktreeToolInput::resolve_entry("worktree", input.clone())?;
                if action != "enter" {
                    return Ok(Vec::new());
                }
                let input = parse_worktree_enter_input(action_input)?;
                if let Some(path) = input.path.filter(|path| !path.trim().is_empty()) {
                    return Ok(vec![
                        PathRequest::read(path.clone()),
                        PathRequest::write(path),
                    ]);
                }
                let path = input
                    .name
                    .as_deref()
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(|name| format!(".agena/worktrees/{name}"))
                    .unwrap_or_else(|| ".agena/worktrees".to_string());
                Ok(vec![PathRequest::write(path)])
            }
            _ => Ok(Vec::new()),
        }
    }

    async fn agent_stop(
        &self,
        input: crate::plugin::AgentStopInput,
    ) -> SdkResult<Option<crate::plugin::AgentStopPatch>> {
        if input.stop_hook_active {
            return Ok(None);
        }
        match self
            .host()?
            .agent_restore(HostAgentRestoreRequest {
                session_id: Some(input.session_id),
            })
            .await
        {
            Ok(_) => Ok(None),
            Err(err)
                if err.code == crate::plugin::sdk::PluginErrorCode::HostUnavailable
                    || err.message.contains("No agent restore point") =>
            {
                Ok(None)
            }
            Err(err) => Err(err),
        }
    }
}

fn entries() -> Vec<PluginToolDecl> {
    vec![
        WorkflowToolInput::tool_decl(),
        ToolsToolInput::tool_decl(),
        TaskEntryToolInput::tool_decl(),
        AgentToolInput::tool_decl(),
        TodoToolInput::tool_decl(),
        SessionToolInput::tool_decl(),
        GoalToolInput::tool_decl(),
        UserToolInput::tool_decl(),
        PlanToolInput::tool_decl().path_access(PathAccessSpec {
            path: ".agena/plans".to_string(),
            kind: PathKind::Write,
        }),
        WorktreeToolInput::tool_decl(),
    ]
}
