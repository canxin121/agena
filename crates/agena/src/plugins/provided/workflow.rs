//! `agena.workflow` plugin: orchestration tools (task, tool_search,
//! todo_write, create_goal, get_goal, update_goal, ask_user, enter_plan_mode,
//! exit_plan_mode, enter_worktree, exit_worktree).

use std::sync::{Arc, RwLock};

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
    HostExitWorktreeRequest, HostGetGoalRequest, HostGoal, HostGoalStatus, HostTodoItem,
    HostTodoPriority, HostTodoStatus, HostTodoWriteRequest, HostUpdateGoalRequest,
    SpawnSubtaskRequest, ToolDescriptor,
};
use crate::plugin::sdk::{
    HookSubscription, HostCapability, InitContext, InitOutcome, PathAccessSpec, PathKind,
    PathRequest, Plugin, PluginManifest, PluginToolDecl, Result as SdkResult, ToolInvokeInput,
    ToolInvokeOutput, ToolTag,
};

pub(crate) const WORKFLOW_PLUGIN_ID: &str = "agena.workflow";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum WorkflowUpdateGoalStatus {
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
struct WorkflowUpdateGoalToolInput {
    pub status: WorkflowUpdateGoalStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
enum WorkflowToolInput {
    Init(WorkflowPromptToolInput),
    Review(WorkflowPromptToolInput),
    SecurityReview(WorkflowPromptToolInput),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
enum ToolsToolInput {
    Search(ToolSearchToolInput),
    Help(ToolsHelpInput),
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
enum AgentToolInput {
    Switch(AgentSwitchToolInput),
    Restore(AgentRestoreToolInput),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
enum TodoToolInput {
    Write(TodoWriteToolInput),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
enum GoalToolInput {
    Get(GetGoalToolInput),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
enum GoalEditToolInput {
    Create(CreateGoalToolInput),
    Clear(ClearGoalToolInput),
    Update(WorkflowUpdateGoalToolInput),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
enum UserToolInput {
    Ask(AskUserToolInput),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
enum PlanToolInput {
    Enter(EnterPlanModeToolInput),
    Exit(ExitPlanModeToolInput),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
enum WorktreeToolInput {
    Enter(EnterWorktreeToolInput),
    Exit(ExitWorktreeToolInput),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct GoalToolResponse {
    goal: Option<HostGoal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remaining_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completion_budget_report: Option<String>,
}

#[derive(Clone, Copy)]
enum CompletionBudgetReport {
    Include,
    Omit,
}

impl GoalToolResponse {
    fn new(goal: Option<HostGoal>, report_mode: CompletionBudgetReport) -> Self {
        let remaining_tokens = goal.as_ref().and_then(|goal| {
            goal.token_budget
                .map(|budget| budget.saturating_sub(goal.tokens_used))
        });
        let completion_budget_report = match report_mode {
            CompletionBudgetReport::Include => goal
                .as_ref()
                .filter(|goal| goal.status == HostGoalStatus::Completed)
                .and_then(completion_budget_report),
            CompletionBudgetReport::Omit => None,
        };
        Self {
            goal,
            remaining_tokens,
            completion_budget_report,
        }
    }
}

fn completion_budget_report(goal: &HostGoal) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(budget) = goal.token_budget {
        parts.push(format!("tokens used: {} of {budget}", goal.tokens_used));
    }
    if goal.time_used_seconds > 0 {
        parts.push(format!("time used: {} seconds", goal.time_used_seconds));
    }
    if parts.is_empty() {
        None
    } else {
        Some(format!(
            "Goal achieved. Report final budget usage to the user: {}.",
            parts.join("; ")
        ))
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
            deferred: descriptor.deferred,
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

    fn goal_tool_payload(
        goal: Option<HostGoal>,
        report_mode: CompletionBudgetReport,
    ) -> SdkResult<serde_json::Value> {
        serde_json::to_value(GoalToolResponse::new(goal, report_mode))
            .map_err(|err| PluginError::new(err.to_string()))
    }

    fn goal_summary(goal: &HostGoal) -> String {
        let budget = goal
            .token_budget
            .map(|value| format!("{}/{}", goal.tokens_used, value))
            .unwrap_or_else(|| goal.tokens_used.to_string());
        format!(
            "Goal {} is {}. Tokens: {}.",
            goal.id,
            match goal.status {
                HostGoalStatus::Active => "active",
                HostGoalStatus::Paused => "paused",
                HostGoalStatus::BudgetLimited => "budget_limited",
                HostGoalStatus::Completed => "completed",
            },
            budget,
        )
    }

    async fn invoke_get_goal(&self, _input: &GetGoalToolInput) -> SdkResult<ToolInvokeOutput> {
        let response = self.host()?.get_goal(HostGetGoalRequest::default()).await?;
        let payload = Self::goal_tool_payload(response.goal.clone(), CompletionBudgetReport::Omit)?;
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
                token_budget: input.token_budget,
            })
            .await?;
        let payload =
            Self::goal_tool_payload(Some(response.goal.clone()), CompletionBudgetReport::Omit)?;
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

    async fn invoke_update_goal(
        &self,
        input: &WorkflowUpdateGoalToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let response = self
            .host()?
            .update_goal(HostUpdateGoalRequest {
                objective: None,
                status: Some(match input.status {
                    WorkflowUpdateGoalStatus::Complete => HostGoalStatus::Completed,
                }),
                token_budget: None,
            })
            .await?;
        let payload =
            Self::goal_tool_payload(Some(response.goal.clone()), CompletionBudgetReport::Include)?;
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
        if descriptor.deferred {
            lines.push("Load priority: deferred".to_string());
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
                self.invoke_task(&serde_json::from_value(input.input)?)
                    .await
            }
            "tools" => match serde_json::from_value::<ToolsToolInput>(input.input)? {
                ToolsToolInput::Search(args) => self.invoke_tool_search(&args).await,
                ToolsToolInput::Help(args) => self.invoke_tool_help(&args).await,
            },
            "agent" => match serde_json::from_value::<AgentToolInput>(input.input)? {
                AgentToolInput::Switch(args) => self.invoke_agent_switch(&args).await,
                AgentToolInput::Restore(args) => self.invoke_agent_restore(&args).await,
            },
            "todo" => match serde_json::from_value::<TodoToolInput>(input.input)? {
                TodoToolInput::Write(args) => {
                    self.host()?
                        .todo_write(HostTodoWriteRequest {
                            items: args.items.iter().map(Self::host_todo_item).collect(),
                        })
                        .await
                }
            },
            "goal" => match serde_json::from_value::<GoalToolInput>(input.input)? {
                GoalToolInput::Get(args) => self.invoke_get_goal(&args).await,
            },
            "goal_edit" => match serde_json::from_value::<GoalEditToolInput>(input.input)? {
                GoalEditToolInput::Create(args) => self.invoke_create_goal(&args).await,
                GoalEditToolInput::Clear(args) => self.invoke_clear_goal(&args).await,
                GoalEditToolInput::Update(args) => self.invoke_update_goal(&args).await,
            },
            "user" => match serde_json::from_value::<UserToolInput>(input.input)? {
                UserToolInput::Ask(args) => self.invoke_ask_user(&args).await,
            },
            "plan" => match serde_json::from_value::<PlanToolInput>(input.input)? {
                PlanToolInput::Enter(_) => {
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
                PlanToolInput::Exit(_) => {
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
            },
            "worktree" => match serde_json::from_value::<WorktreeToolInput>(input.input)? {
                WorktreeToolInput::Enter(args) => {
                    self.host()?
                        .enter_worktree(HostEnterWorktreeRequest {
                            name: args.name,
                            path: args.path,
                        })
                        .await
                }
                WorktreeToolInput::Exit(args) => {
                    self.host()?
                        .exit_worktree(HostExitWorktreeRequest {
                            action: args.action,
                            discard_changes: args.discard_changes,
                        })
                        .await
                }
            },
            "workflow" => match serde_json::from_value::<WorkflowToolInput>(input.input)? {
                WorkflowToolInput::Init(args) => self.invoke_provided_workflow("init", &args).await,
                WorkflowToolInput::Review(args) => {
                    self.invoke_agent_workflow("review", "reviewer", &args)
                        .await
                }
                WorkflowToolInput::SecurityReview(args) => {
                    self.invoke_agent_workflow("security-review", "reviewer", &args)
                        .await
                }
            },
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
                let WorktreeToolInput::Enter(input) = serde_json::from_value(input.clone())? else {
                    return Ok(Vec::new());
                };
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
        PluginToolDecl::new(
            "workflow",
            crate::entry::definition::json_schema_for::<WorkflowToolInput>(),
        )
        .description(
            "Workflow prompt command. Set command to init, review, or security_review; pass that command's payload in args.",
        )
        .tag(ToolTag::ReadOnly)
        .always_load()
        .concurrency_safe(true)
        .host_capability(HostCapability::AgentRegistry),
        PluginToolDecl::new(
            "tools",
            crate::entry::definition::json_schema_for::<ToolsToolInput>(),
        )
        .description("Tool catalog command. Set command to search to find/load deferred tools, or help to fetch detailed usage for a tool.")
        .summary("Search tools or fetch detailed tool help.")
        .help("Use command `search` with query/load/limit to discover tools and load deferred tools. Use command `help` with `tool` to retrieve the full registered help text and input schema for any model-visible tool.")
        .tags([ToolTag::ReadOnly, ToolTag::Discovery])
        .always_load()
        .concurrency_safe(true)
        .host_capability(HostCapability::ListTools),
        PluginToolDecl::new(
            "task",
            crate::entry::definition::json_schema_for::<TaskToolInput>(),
        )
        .description(
            "Create or resume a typed subagent task session for explore, implement, or verify delegated work.",
        )
        .tags([ToolTag::Task, ToolTag::Subtask])
        .concurrency_safe(false)
        .deferred_load()
        .host_capability(HostCapability::SpawnSubtask),
        PluginToolDecl::new(
            "agent",
            crate::entry::definition::json_schema_for::<AgentToolInput>(),
        )
        .description(
            "Runtime agent command. Set command to switch or restore; pass that command's payload in args.",
        )
        .always_load()
        .concurrency_safe(false)
        .host_capability(HostCapability::AgentRegistry),
        PluginToolDecl::new(
            "todo",
            crate::entry::definition::json_schema_for::<TodoToolInput>(),
        )
        .description("Todo command. Set command to write to replace the session todo list.")
        .tags([ToolTag::Mutating, ToolTag::Planning])
        .always_load(),
        PluginToolDecl::new(
            "goal",
            crate::entry::definition::json_schema_for::<GoalToolInput>(),
        )
        .description("Goal read command. Set command to get to inspect the current runtime goal.")
        .tags([ToolTag::ReadOnly, ToolTag::Goal])
        .always_load()
        .concurrency_safe(true)
        .host_capability(HostCapability::GoalRegistry),
        PluginToolDecl::new(
            "goal_edit",
            crate::entry::definition::json_schema_for::<GoalEditToolInput>(),
        )
        .description(
            "Goal mutation command. Set command to create, clear, or update; use update only to mark a goal complete once the objective is actually finished.",
        )
        .tags([ToolTag::Mutating, ToolTag::Goal])
        .always_load()
        .concurrency_safe(false)
        .host_capability(HostCapability::GoalRegistry),
        PluginToolDecl::new(
            "user",
            crate::entry::definition::json_schema_for::<UserToolInput>(),
        )
        .description("User interaction command. Set command to ask to request short answers.")
        .tags([ToolTag::ReadOnly, ToolTag::Interactive])
        .always_load()
        .concurrency_safe(false)
        .host_capability(HostCapability::AskUser),
        PluginToolDecl::new(
            "plan",
            crate::entry::definition::json_schema_for::<PlanToolInput>(),
        )
        .description(
            "Plan mode command. Set command to enter or exit; enter allocates a plan markdown file under .agena/plans/ and exit asks the user to approve the plan.",
        )
        .tags([ToolTag::ReadOnly, ToolTag::Planning])
        .tag(ToolTag::FilesystemWrite)
        .path_access(PathAccessSpec {
            path: ".agena/plans".to_string(),
            kind: PathKind::Write,
        })
        .always_load()
        .concurrency_safe(true)
        .host_capabilities([HostCapability::PlanRegistry, HostCapability::AgentRegistry]),
        PluginToolDecl::new(
            "worktree",
            crate::entry::definition::json_schema_for::<WorktreeToolInput>(),
        )
        .description(
            "Worktree command. Set command to enter or exit; enter creates or attaches to a git worktree and exit leaves it.",
        )
        .tags([ToolTag::Mutating, ToolTag::FilesystemWrite, ToolTag::Worktree])
        .concurrency_safe(false)
        .deferred_load()
        .host_capability(HostCapability::WorktreeRegistry),
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use super::*;
    use crate::plugin::sdk::host_api::{
        EventSubscription, HostAgentRestoreRequest, HostAgentRestoreResponse,
        HostAgentSwitchRequest, HostAgentSwitchResponse, HostClearGoalRequest,
        HostClearGoalResponse, HostCreateGoalRequest, HostCreateGoalResponse,
        HostEnterPlanModeRequest, HostEnterWorktreeRequest, HostExitPlanModeRequest,
        HostExitWorktreeRequest, HostGetGoalRequest, HostGetGoalResponse, HostGoal, HostGoalStatus,
        HostUpdateGoalRequest, HostUpdateGoalResponse, LogLevel,
    };
    use crate::plugin::sdk::{EventEnvelope, EventFilter, PermissionAskInput, PermissionDecision};

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct RecordedUpdateGoalRequest {
        objective: Option<String>,
        status: Option<HostGoalStatus>,
        token_budget: Option<Option<u64>>,
    }

    struct TestHost {
        clear_goal_request_count: Mutex<u32>,
        update_goal_request: Mutex<Option<RecordedUpdateGoalRequest>>,
        agent_switch_requests: Mutex<Vec<HostAgentSwitchRequest>>,
        agent_restore_requests: Mutex<Vec<HostAgentRestoreRequest>>,
    }

    impl TestHost {
        fn new() -> Self {
            Self {
                clear_goal_request_count: Mutex::new(0),
                update_goal_request: Mutex::new(None),
                agent_switch_requests: Mutex::new(Vec::new()),
                agent_restore_requests: Mutex::new(Vec::new()),
            }
        }

        fn clear_goal_request_count(&self) -> u32 {
            *self
                .clear_goal_request_count
                .lock()
                .expect("clear goal request lock")
        }

        fn recorded_update_goal_request(&self) -> Option<RecordedUpdateGoalRequest> {
            self.update_goal_request
                .lock()
                .expect("update goal request lock")
                .clone()
        }

        fn recorded_agent_switches(&self) -> Vec<HostAgentSwitchRequest> {
            self.agent_switch_requests
                .lock()
                .expect("agent switch request lock")
                .clone()
        }

        fn recorded_agent_restores(&self) -> Vec<HostAgentRestoreRequest> {
            self.agent_restore_requests
                .lock()
                .expect("agent restore request lock")
                .clone()
        }
    }

    #[async_trait]
    impl HostClient for TestHost {
        async fn log(&self, _level: LogLevel, _message: String, _fields: serde_json::Value) {}

        async fn publish_event(&self, _env: EventEnvelope) -> SdkResult<()> {
            Ok(())
        }

        async fn subscribe_events(&self, _filter: EventFilter) -> SdkResult<EventSubscription> {
            Ok(EventSubscription { id: "sub".into() })
        }

        async fn ask_permission(&self, _req: PermissionAskInput) -> SdkResult<PermissionDecision> {
            Ok(PermissionDecision::Prompt)
        }

        async fn read_config(&self, _path: Option<String>) -> SdkResult<serde_json::Value> {
            Ok(serde_json::Value::Null)
        }

        async fn invoke_tool(
            &self,
            tool: String,
            _input: serde_json::Value,
        ) -> SdkResult<ToolInvokeOutput> {
            Err(PluginError::new(format!(
                "unexpected invoke_tool for {tool}"
            )))
        }

        async fn ask_user(
            &self,
            req: AskUserRequest,
        ) -> SdkResult<crate::plugin::sdk::host_api::AskUserResponse> {
            let question_id = req
                .questions
                .first()
                .map(|question| question.id.clone())
                .unwrap_or_else(|| "reply".to_string());
            Ok(crate::plugin::sdk::host_api::AskUserResponse {
                reply: String::new(),
                cancelled: false,
                answers: BTreeMap::from([(question_id, vec!["blue".to_string()])]),
            })
        }

        async fn spawn_subtask(
            &self,
            req: SpawnSubtaskRequest,
        ) -> SdkResult<crate::plugin::sdk::host_api::SpawnSubtaskResponse> {
            Ok(crate::plugin::sdk::host_api::SpawnSubtaskResponse {
                final_text: format!("spawned {}", req.description),
                metadata: BTreeMap::from([
                    ("session_id".to_string(), "child-1".to_string()),
                    ("model_provider_id".to_string(), "provider".to_string()),
                    ("model_id".to_string(), "model".to_string()),
                ]),
            })
        }

        async fn list_tools(&self) -> SdkResult<Vec<ToolDescriptor>> {
            Ok(vec![ToolDescriptor {
                name: "shell".to_string(),
                description: Some("Execute shell commands".to_string()),
                summary: Some("Shell commands".to_string()),
                help: Some("Execute shell commands.".to_string()),
                input_schema: Some(serde_json::json!({"type": "object"})),
                description_mode: None,
                tags: vec![
                    crate::plugin::sdk::ToolTag::Mutating,
                    crate::plugin::sdk::ToolTag::Shell,
                ],
                deferred: true,
                plugin_id: None,
            }])
        }

        async fn agent_switch(
            &self,
            req: HostAgentSwitchRequest,
        ) -> SdkResult<HostAgentSwitchResponse> {
            self.agent_switch_requests
                .lock()
                .expect("agent switch request lock")
                .push(req.clone());
            Ok(HostAgentSwitchResponse {
                session_id: req.session_id.unwrap_or(1),
                previous_agent: Some("build".to_string()),
                current_agent: req.agent.clone().filter(|agent| !agent.trim().is_empty()),
                stack_depth: usize::from(req.push_previous),
            })
        }

        async fn agent_restore(
            &self,
            req: HostAgentRestoreRequest,
        ) -> SdkResult<HostAgentRestoreResponse> {
            self.agent_restore_requests
                .lock()
                .expect("agent restore request lock")
                .push(req.clone());
            Ok(HostAgentRestoreResponse {
                session_id: req.session_id.unwrap_or(1),
                restored: true,
                previous_agent: Some("reviewer".to_string()),
                current_agent: Some("build".to_string()),
                stack_depth: 0,
            })
        }

        async fn todo_write(&self, req: HostTodoWriteRequest) -> SdkResult<ToolInvokeOutput> {
            assert_eq!(req.items.len(), 1);
            assert_eq!(req.items[0].content, "ship it");
            Ok(
                crate::plugins::provided::router::tool_execution_to_invoke_output(
                    ToolPayloadExecution::new(
                        ToolPayloadOutput::TodoWrite {
                            items: vec![TodoItem {
                                content: "ship it".to_string(),
                                status: TodoStatus::InProgress,
                                priority: TodoPriority::High,
                            }],
                        },
                        ToolExecutionView::simple(
                            "Todo write",
                            "Updated todo list with 1 item(s):",
                        ),
                    ),
                ),
            )
        }

        async fn get_goal(&self, _req: HostGetGoalRequest) -> SdkResult<HostGetGoalResponse> {
            Ok(HostGetGoalResponse {
                goal: Some(HostGoal {
                    id: 7,
                    objective: "ship it".to_string(),
                    status: HostGoalStatus::Active,
                    token_budget: Some(128),
                    tokens_used: 32,
                    time_used_seconds: 5,
                    completed_at_ms: None,
                }),
            })
        }

        async fn create_goal(
            &self,
            req: HostCreateGoalRequest,
        ) -> SdkResult<HostCreateGoalResponse> {
            assert_eq!(req.objective, "ship it");
            assert_eq!(req.token_budget, Some(128));
            Ok(HostCreateGoalResponse {
                goal: HostGoal {
                    id: 7,
                    objective: req.objective,
                    status: HostGoalStatus::Active,
                    token_budget: req.token_budget,
                    tokens_used: 0,
                    time_used_seconds: 0,
                    completed_at_ms: None,
                },
            })
        }

        async fn update_goal(
            &self,
            req: HostUpdateGoalRequest,
        ) -> SdkResult<HostUpdateGoalResponse> {
            *self
                .update_goal_request
                .lock()
                .expect("update goal request lock") = Some(RecordedUpdateGoalRequest {
                objective: req.objective.clone(),
                status: req.status,
                token_budget: req.token_budget,
            });
            let status = req.status.unwrap_or(HostGoalStatus::Active);
            Ok(HostUpdateGoalResponse {
                goal: HostGoal {
                    id: 7,
                    objective: req.objective.unwrap_or_else(|| "ship it".to_string()),
                    status,
                    token_budget: req.token_budget.unwrap_or(Some(128)),
                    tokens_used: 48,
                    time_used_seconds: 8,
                    completed_at_ms: (status == HostGoalStatus::Completed).then_some(123),
                },
            })
        }

        async fn clear_goal(&self, _req: HostClearGoalRequest) -> SdkResult<HostClearGoalResponse> {
            let mut count = self
                .clear_goal_request_count
                .lock()
                .expect("clear goal request lock");
            *count += 1;
            Ok(HostClearGoalResponse { cleared: true })
        }

        async fn enter_plan_mode(
            &self,
            _req: HostEnterPlanModeRequest,
        ) -> SdkResult<ToolInvokeOutput> {
            Ok(
                crate::plugins::provided::router::tool_execution_to_invoke_output(
                    ToolPayloadExecution::new(
                        ToolPayloadOutput::EnterPlanMode {
                            plan_path: "/tmp/plan.md".to_string(),
                            slug: "demo".to_string(),
                        },
                        ToolExecutionView::simple("Plan mode entered", "plan on"),
                    ),
                ),
            )
        }

        async fn exit_plan_mode(
            &self,
            _req: HostExitPlanModeRequest,
        ) -> SdkResult<ToolInvokeOutput> {
            Ok(
                crate::plugins::provided::router::tool_execution_to_invoke_output(
                    ToolPayloadExecution::new(
                        ToolPayloadOutput::ExitPlanMode {
                            approved: true,
                            plan_path: "/tmp/plan.md".to_string(),
                        },
                        ToolExecutionView::simple("Plan mode exited", "plan off"),
                    ),
                ),
            )
        }

        async fn enter_worktree(
            &self,
            req: HostEnterWorktreeRequest,
        ) -> SdkResult<ToolInvokeOutput> {
            assert_eq!(req.name.as_deref(), Some("demo"));
            assert_eq!(req.path, None);
            Ok(
                crate::plugins::provided::router::tool_execution_to_invoke_output(
                    ToolPayloadExecution::new(
                        ToolPayloadOutput::EnterWorktree {
                            path: "/tmp/wt".to_string(),
                            branch: "agena/demo".to_string(),
                        },
                        ToolExecutionView::simple("Worktree", "entered worktree"),
                    ),
                ),
            )
        }

        async fn exit_worktree(&self, req: HostExitWorktreeRequest) -> SdkResult<ToolInvokeOutput> {
            assert_eq!(req.action, "keep");
            assert!(!req.discard_changes);
            Ok(
                crate::plugins::provided::router::tool_execution_to_invoke_output(
                    ToolPayloadExecution::new(
                        ToolPayloadOutput::ExitWorktree {
                            action: "keep".to_string(),
                            path: "/tmp/wt".to_string(),
                        },
                        ToolExecutionView::simple("Worktree", "exited worktree"),
                    ),
                ),
            )
        }
    }

    async fn initialized_plugin() -> (WorkflowPlugin, Arc<TestHost>) {
        let plugin = WorkflowPlugin::new();
        let host = Arc::new(TestHost::new());
        plugin
            .init(
                InitContext {
                    agena_version: "test".to_string(),
                    workspace_root: "/tmp".into(),
                    plugin_id: WORKFLOW_PLUGIN_ID.to_string(),
                    host_callback_url: None,
                    host_callback_token: None,
                    options: serde_json::Value::Null,
                    protocol_version: crate::plugin::sdk::rpc::PROTOCOL_VERSION,
                },
                host.clone(),
            )
            .await
            .expect("workflow plugin init");
        (plugin, host)
    }

    fn invoke_input<T: serde::Serialize>(tool_name: &str, input: T) -> ToolInvokeInput {
        ToolInvokeInput {
            tool_name: tool_name.to_string(),
            session_id: 1,
            call_id: 2,
            workspace_root: "/tmp".to_string(),
            input: serde_json::to_value(input).expect("serialize tool input"),
        }
    }

    fn invoke_command<T: serde::Serialize>(
        tool_name: &str,
        command: &str,
        args: T,
    ) -> ToolInvokeInput {
        invoke_input(
            tool_name,
            serde_json::json!({
                "command": command,
                "args": serde_json::to_value(args).expect("serialize command args"),
            }),
        )
    }

    #[tokio::test]
    async fn ask_user_invokes_host_without_executor_context() {
        let (plugin, _) = initialized_plugin().await;
        let output = plugin
            .tool_invoke(invoke_command(
                "user",
                "ask",
                AskUserToolInput {
                    questions: vec![crate::message::UserInputQuestion {
                        id: "color".to_string(),
                        header: "Color".to_string(),
                        question: "Which color?".to_string(),
                        options: vec![crate::message::UserInputOption {
                            label: "blue".to_string(),
                            description: String::new(),
                        }],
                        multiple: false,
                        allow_custom: false,
                    }],
                },
            ))
            .await
            .expect("ask_user host invoke");
        let output = crate::plugins::provided::router::payload_to_tool_output(
            "ask_user",
            output.payload.as_ref(),
        )
        .unwrap();
        match output {
            ToolPayloadOutput::AskUser { answers } => {
                assert_eq!(answers["color"], vec!["blue".to_string()]);
            }
            other => panic!("unexpected output: {other:?}"),
        }
    }

    #[tokio::test]
    async fn get_goal_invokes_host_without_executor_context() {
        let (plugin, _) = initialized_plugin().await;
        let output = plugin
            .tool_invoke(invoke_command("goal", "get", GetGoalToolInput::default()))
            .await
            .expect("get_goal host invoke");

        assert!(output.output_text.contains("Goal 7 is active"));
        assert_eq!(
            output
                .payload
                .as_ref()
                .and_then(|payload| payload["goal"]["objective"].as_str()),
            Some("ship it")
        );
        assert_eq!(
            output
                .payload
                .as_ref()
                .and_then(|payload| payload["remaining_tokens"].as_u64()),
            Some(96)
        );
        assert_eq!(
            output
                .payload
                .as_ref()
                .map(|payload| payload["completion_budget_report"].is_null()),
            Some(true)
        );
    }

    #[test]
    fn goal_summary_renders_paused_status() {
        let summary = WorkflowPlugin::goal_summary(&HostGoal {
            id: 7,
            objective: "ship it".to_string(),
            status: HostGoalStatus::Paused,
            token_budget: Some(128),
            tokens_used: 32,
            time_used_seconds: 5,
            completed_at_ms: None,
        });

        assert_eq!(summary, "Goal 7 is paused. Tokens: 32/128.");
    }

    #[tokio::test]
    async fn create_goal_invokes_host_without_executor_context() {
        let (plugin, _) = initialized_plugin().await;
        let output = plugin
            .tool_invoke(invoke_command(
                "goal_edit",
                "create",
                CreateGoalToolInput {
                    objective: "ship it".to_string(),
                    token_budget: Some(128),
                },
            ))
            .await
            .expect("create_goal host invoke");

        assert!(output.output_text.contains("Goal 7 is active"));
        assert_eq!(
            output
                .payload
                .as_ref()
                .and_then(|payload| payload["goal"]["objective"].as_str()),
            Some("ship it")
        );
        assert_eq!(
            output
                .payload
                .as_ref()
                .and_then(|payload| payload["goal"]["token_budget"].as_u64()),
            Some(128)
        );
        assert_eq!(
            output
                .payload
                .as_ref()
                .and_then(|payload| payload["remaining_tokens"].as_u64()),
            Some(128)
        );
    }

    #[tokio::test]
    async fn clear_goal_invokes_host_without_executor_context() {
        let (plugin, host) = initialized_plugin().await;
        let output = plugin
            .tool_invoke(invoke_command(
                "goal_edit",
                "clear",
                ClearGoalToolInput::default(),
            ))
            .await
            .expect("clear_goal host invoke");

        assert_eq!(host.clear_goal_request_count(), 1);
        assert!(output.output_text.contains("Cleared the current goal."));
        assert_eq!(
            output
                .payload
                .as_ref()
                .and_then(|payload| payload["cleared"].as_bool()),
            Some(true)
        );
    }

    #[tokio::test]
    async fn update_goal_invokes_host_without_executor_context() {
        let (plugin, host) = initialized_plugin().await;
        let output = plugin
            .tool_invoke(invoke_command(
                "goal_edit",
                "update",
                WorkflowUpdateGoalToolInput {
                    status: WorkflowUpdateGoalStatus::Complete,
                },
            ))
            .await
            .expect("update_goal host invoke");

        assert!(output.output_text.contains("Goal 7 is completed"));
        assert_eq!(
            host.recorded_update_goal_request(),
            Some(RecordedUpdateGoalRequest {
                objective: None,
                status: Some(HostGoalStatus::Completed),
                token_budget: None,
            })
        );
        assert_eq!(
            output
                .payload
                .as_ref()
                .and_then(|payload| payload["goal"]["status"].as_str()),
            Some("completed")
        );
        assert_eq!(
            output
                .payload
                .as_ref()
                .and_then(|payload| payload["remaining_tokens"].as_u64()),
            Some(80)
        );
        assert_eq!(
            output
                .payload
                .as_ref()
                .and_then(|payload| payload["completion_budget_report"].as_str()),
            Some(
                "Goal achieved. Report final budget usage to the user: tokens used: 48 of 128; time used: 8 seconds."
            )
        );
    }

    #[test]
    fn update_goal_command_schema_only_exposes_complete_status() {
        let schema = crate::entry::definition::json_schema_for::<WorkflowUpdateGoalToolInput>();
        let properties = schema
            .get("properties")
            .and_then(|value| value.as_object())
            .expect("update_goal properties should be an object");

        assert!(properties.contains_key("status"));
        assert!(!properties.contains_key("objective"));
        assert!(!properties.contains_key("token_budget"));
        let status_schema = properties
            .get("status")
            .expect("status schema should exist");
        let status_schema = if status_schema.get("enum").is_some()
            || status_schema.get("const").is_some()
        {
            status_schema
        } else if let Some(reference) = status_schema.get("$ref").and_then(|value| value.as_str()) {
            let definition_name = reference
                .rsplit('/')
                .next()
                .expect("status schema ref should have a definition name");
            schema
                .get("$defs")
                .and_then(|defs| defs.get(definition_name))
                .or_else(|| {
                    schema
                        .get("definitions")
                        .and_then(|defs| defs.get(definition_name))
                })
                .expect("status schema ref should resolve")
        } else {
            panic!("status schema should be inline or a local ref: {status_schema:?}");
        };
        let status_values = status_schema
            .get("enum")
            .and_then(|value| value.as_array())
            .map(|values| {
                values
                    .iter()
                    .map(|value| {
                        value
                            .as_str()
                            .expect("enum values should be strings")
                            .to_string()
                    })
                    .collect::<Vec<_>>()
            })
            .or_else(|| {
                status_schema
                    .get("const")
                    .and_then(|value| value.as_str())
                    .map(|value| vec![value.to_string()])
            })
            .expect("status schema should enumerate a string value");
        assert_eq!(status_values, vec!["complete".to_string()]);
    }

    #[tokio::test]
    async fn task_invokes_host_without_executor_context() {
        let (plugin, _) = initialized_plugin().await;
        let output = plugin
            .tool_invoke(invoke_input(
                "task",
                TaskToolInput {
                    description: "inspect".to_string(),
                    prompt: "look around".to_string(),
                    subagent_type: crate::message::TaskSubagentType::Explore,
                    task_id: None,
                    command: None,
                },
            ))
            .await
            .expect("task host invoke");
        let output = crate::plugins::provided::router::payload_to_tool_output(
            "task",
            output.payload.as_ref(),
        )
        .unwrap();
        match output {
            ToolPayloadOutput::Task {
                session_id,
                model_provider_id,
                model_id,
            } => {
                assert_eq!(session_id.as_deref(), Some("child-1"));
                assert_eq!(model_provider_id.as_deref(), Some("provider"));
                assert_eq!(model_id.as_deref(), Some("model"));
            }
            other => panic!("unexpected output: {other:?}"),
        }
    }

    #[tokio::test]
    async fn todo_write_invokes_explicit_host_api() {
        let (plugin, _) = initialized_plugin().await;
        let output = plugin
            .tool_invoke(invoke_command(
                "todo",
                "write",
                TodoWriteToolInput {
                    items: vec![TodoItem {
                        content: "ship it".to_string(),
                        status: TodoStatus::InProgress,
                        priority: TodoPriority::High,
                    }],
                },
            ))
            .await
            .expect("todo_write host invoke");
        let output = crate::plugins::provided::router::payload_to_tool_output(
            "todo_write",
            output.payload.as_ref(),
        )
        .unwrap();
        match output {
            ToolPayloadOutput::TodoWrite { items } => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].content, "ship it");
                assert_eq!(items[0].status, TodoStatus::InProgress);
                assert_eq!(items[0].priority, TodoPriority::High);
            }
            other => panic!("unexpected output: {other:?}"),
        }
    }

    #[tokio::test]
    async fn plan_and_worktree_entries_invoke_explicit_host_apis() {
        let (plugin, host) = initialized_plugin().await;

        let enter_plan = plugin
            .tool_invoke(invoke_command(
                "plan",
                "enter",
                EnterPlanModeToolInput::default(),
            ))
            .await
            .expect("enter_plan_mode host invoke");
        let enter_plan = crate::plugins::provided::router::payload_to_tool_output(
            "enter_plan_mode",
            enter_plan.payload.as_ref(),
        )
        .unwrap();
        match enter_plan {
            ToolPayloadOutput::EnterPlanMode { plan_path, slug } => {
                assert_eq!(plan_path, "/tmp/plan.md");
                assert_eq!(slug, "demo");
            }
            other => panic!("unexpected output: {other:?}"),
        }
        let switches = host.recorded_agent_switches();
        assert_eq!(switches.len(), 1);
        assert_eq!(switches[0].agent.as_deref(), Some("planner"));
        assert!(switches[0].push_previous);

        let enter_worktree = plugin
            .tool_invoke(invoke_command(
                "worktree",
                "enter",
                EnterWorktreeToolInput {
                    name: Some("demo".to_string()),
                    path: None,
                },
            ))
            .await
            .expect("enter_worktree host invoke");
        let enter_worktree = crate::plugins::provided::router::payload_to_tool_output(
            "enter_worktree",
            enter_worktree.payload.as_ref(),
        )
        .unwrap();
        match enter_worktree {
            ToolPayloadOutput::EnterWorktree { path, branch } => {
                assert_eq!(path, "/tmp/wt");
                assert_eq!(branch, "agena/demo");
            }
            other => panic!("unexpected output: {other:?}"),
        }

        let exit_worktree = plugin
            .tool_invoke(invoke_command(
                "worktree",
                "exit",
                ExitWorktreeToolInput {
                    action: "keep".to_string(),
                    discard_changes: false,
                },
            ))
            .await
            .expect("exit_worktree host invoke");
        let exit_worktree = crate::plugins::provided::router::payload_to_tool_output(
            "exit_worktree",
            exit_worktree.payload.as_ref(),
        )
        .unwrap();
        match exit_worktree {
            ToolPayloadOutput::ExitWorktree { action, path } => {
                assert_eq!(action, "keep");
                assert_eq!(path, "/tmp/wt");
            }
            other => panic!("unexpected output: {other:?}"),
        }

        let exit_plan = plugin
            .tool_invoke(invoke_command(
                "plan",
                "exit",
                ExitPlanModeToolInput::default(),
            ))
            .await
            .expect("exit_plan_mode host invoke");
        let exit_plan = crate::plugins::provided::router::payload_to_tool_output(
            "exit_plan_mode",
            exit_plan.payload.as_ref(),
        )
        .unwrap();
        match exit_plan {
            ToolPayloadOutput::ExitPlanMode {
                approved,
                plan_path,
            } => {
                assert!(approved);
                assert_eq!(plan_path, "/tmp/plan.md");
            }
            other => panic!("unexpected output: {other:?}"),
        }
        let restores = host.recorded_agent_restores();
        assert_eq!(restores.len(), 1);
    }

    #[tokio::test]
    async fn tool_search_invokes_host_catalog_without_executor_context() {
        let (plugin, _) = initialized_plugin().await;
        let output = plugin
            .tool_invoke(invoke_command(
                "tools",
                "search",
                ToolSearchToolInput {
                    query: "shell".to_string(),
                    load: Vec::new(),
                    limit: None,
                },
            ))
            .await
            .expect("tool_search host invoke");
        let output = crate::plugins::provided::router::payload_to_tool_output(
            "tool_search",
            output.payload.as_ref(),
        )
        .unwrap();
        match output {
            ToolPayloadOutput::ToolSearch { results, .. } => {
                assert_eq!(results, vec!["shell".to_string()]);
            }
            other => panic!("unexpected output: {other:?}"),
        }
    }

    #[tokio::test]
    async fn tools_help_returns_registered_help_and_schema() {
        let (plugin, _) = initialized_plugin().await;
        let output = plugin
            .tool_invoke(invoke_command(
                "tools",
                "help",
                ToolsHelpInput {
                    tool: "shell".to_string(),
                    include_schema: true,
                },
            ))
            .await
            .expect("tools help invoke");

        assert!(output.output_text.contains("Tool: shell"));
        assert!(output.output_text.contains("Help:"));
        assert!(output.output_text.contains("Execute shell commands."));
        assert!(output.output_text.contains("Input schema:"));
        assert!(output.output_text.contains("\"type\": \"object\""));
    }

    #[test]
    fn agent_tool_schema_has_top_level_object_type() {
        let agent_decl = entries()
            .into_iter()
            .find(|decl| decl.name == "agent")
            .expect("agent tool should be registered");
        let schema = agent_decl.sanitized_input_schema();

        assert_eq!(
            schema.get("type").and_then(serde_json::Value::as_str),
            Some("object")
        );
        assert!(
            schema.get("oneOf").is_some() || schema.get("anyOf").is_some(),
            "agent schema should remain a command union: {schema:?}"
        );
    }

    #[tokio::test]
    async fn provided_workflow_entries_render_prompt_text() {
        let (plugin, host) = initialized_plugin().await;

        let review = plugin
            .tool_invoke(invoke_command(
                "workflow",
                "review",
                WorkflowPromptToolInput {
                    args: Some("auth flow".to_string()),
                },
            ))
            .await
            .expect("workflow_review invoke");
        assert_eq!(review.title, "review workflow");
        assert!(
            review
                .output_text
                .contains("You are reviewing the changes on the current branch")
        );
        assert!(review.output_text.contains("User arguments:\nauth flow"));
        assert_eq!(review.metadata.get("workflow"), Some(&"review".to_string()));
        assert_eq!(
            review.metadata.get("workflow_agent"),
            Some(&"reviewer".to_string())
        );

        let init = plugin
            .tool_invoke(invoke_command(
                "workflow",
                "init",
                WorkflowPromptToolInput::default(),
            ))
            .await
            .expect("workflow_init invoke");
        assert_eq!(init.title, "init workflow");
        assert!(init.output_text.contains("Save the result to AGENA.md"));

        let security = plugin
            .tool_invoke(invoke_command(
                "workflow",
                "security_review",
                WorkflowPromptToolInput::default(),
            ))
            .await
            .expect("workflow_security_review invoke");
        assert_eq!(security.title, "security-review workflow");
        assert!(
            security
                .output_text
                .contains("Audit the changes on this branch")
        );
        let switches = host.recorded_agent_switches();
        assert_eq!(switches.len(), 2);
        assert!(
            switches
                .iter()
                .all(|req| req.agent.as_deref() == Some("reviewer"))
        );
        assert!(switches.iter().all(|req| req.push_previous));
    }

    #[tokio::test]
    async fn agent_stop_restores_pushed_workflow_agent() {
        let (plugin, host) = initialized_plugin().await;

        let patch = plugin
            .agent_stop(crate::plugin::AgentStopInput {
                session_id: 42,
                stop_hook_active: false,
                last_assistant_message: Some("done".to_string()),
            })
            .await
            .expect("agent_stop restore");

        assert!(patch.is_none());
        let restores = host.recorded_agent_restores();
        assert_eq!(restores.len(), 1);
        assert_eq!(restores[0].session_id, Some(42));
    }
}
