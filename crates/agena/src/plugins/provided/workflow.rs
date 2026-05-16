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
    AskUserToolInput, ClearGoalToolInput, CreateGoalToolInput, EnterPlanModeToolInput,
    EnterWorktreeToolInput, ExitPlanModeToolInput, ExitWorktreeToolInput, GetGoalToolInput,
    TaskToolInput, TodoItem, TodoPriority, TodoStatus, TodoWriteToolInput, ToolSearchToolInput,
    WorkflowPromptToolInput,
};
use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::{
    AskUserOption as HostAskUserOption, AskUserQuestion as HostAskUserQuestion, AskUserRequest,
    HostClearGoalRequest, HostClient, HostCreateGoalRequest, HostEnterPlanModeRequest,
    HostEnterWorktreeRequest, HostExitPlanModeRequest, HostExitWorktreeRequest, HostGetGoalRequest,
    HostGoal, HostGoalStatus, HostTodoItem, HostTodoPriority, HostTodoStatus, HostTodoWriteRequest,
    HostUpdateGoalRequest, SpawnSubtaskRequest, ToolDescriptor,
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
            description: descriptor.description.unwrap_or_default(),
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
}

pub(crate) fn new_plugin() -> WorkflowPlugin {
    WorkflowPlugin::new()
}

#[async_trait]
impl Plugin for WorkflowPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder("agena-workflow", env!("CARGO_PKG_VERSION"))
            .description("Workflow orchestration tools.")
            .hooks(HookSubscription::TOOL_INVOKE)
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
            "tool_search" => {
                self.invoke_tool_search(&serde_json::from_value(input.input)?)
                    .await
            }
            "todo_write" => {
                let input: TodoWriteToolInput = serde_json::from_value(input.input)?;
                self.host()?
                    .todo_write(HostTodoWriteRequest {
                        items: input.items.iter().map(Self::host_todo_item).collect(),
                    })
                    .await
            }
            "create_goal" => {
                self.invoke_create_goal(&serde_json::from_value(input.input)?)
                    .await
            }
            "get_goal" => {
                self.invoke_get_goal(&serde_json::from_value(input.input)?)
                    .await
            }
            "clear_goal" => {
                self.invoke_clear_goal(&serde_json::from_value(input.input)?)
                    .await
            }
            "update_goal" => {
                self.invoke_update_goal(&serde_json::from_value(input.input)?)
                    .await
            }
            "ask_user" => {
                self.invoke_ask_user(&serde_json::from_value(input.input)?)
                    .await
            }
            "enter_plan_mode" => {
                let _: EnterPlanModeToolInput = serde_json::from_value(input.input)?;
                self.host()?
                    .enter_plan_mode(HostEnterPlanModeRequest::default())
                    .await
            }
            "exit_plan_mode" => {
                let _: ExitPlanModeToolInput = serde_json::from_value(input.input)?;
                self.host()?
                    .exit_plan_mode(HostExitPlanModeRequest::default())
                    .await
            }
            "enter_worktree" => {
                let input: EnterWorktreeToolInput = serde_json::from_value(input.input)?;
                self.host()?
                    .enter_worktree(HostEnterWorktreeRequest {
                        name: input.name,
                        path: input.path,
                    })
                    .await
            }
            "exit_worktree" => {
                let input: ExitWorktreeToolInput = serde_json::from_value(input.input)?;
                self.host()?
                    .exit_worktree(HostExitWorktreeRequest {
                        action: input.action,
                        discard_changes: input.discard_changes,
                    })
                    .await
            }
            "workflow_init" => {
                self.invoke_provided_workflow("init", &serde_json::from_value(input.input)?)
                    .await
            }
            "workflow_review" => {
                self.invoke_provided_workflow("review", &serde_json::from_value(input.input)?)
                    .await
            }
            "workflow_security_review" => {
                self.invoke_provided_workflow(
                    "security-review",
                    &serde_json::from_value(input.input)?,
                )
                .await
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
            "enter_worktree" => {
                let input: EnterWorktreeToolInput = serde_json::from_value(input.clone())?;
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
}

fn entries() -> Vec<PluginToolDecl> {
    vec![
        PluginToolDecl::new(
            "workflow_init",
            crate::entry::definition::json_schema_for::<WorkflowPromptToolInput>(),
        )
        .description("Generate the init workflow prompt so it can be submitted as a normal turn.")
        .tag(ToolTag::ReadOnly)
        .always_load()
        .concurrency_safe(true),
        PluginToolDecl::new(
            "workflow_review",
            crate::entry::definition::json_schema_for::<WorkflowPromptToolInput>(),
        )
        .description("Generate the review workflow prompt so it can be submitted as a normal turn.")
        .tag(ToolTag::ReadOnly)
        .always_load()
        .concurrency_safe(true),
        PluginToolDecl::new(
            "workflow_security_review",
            crate::entry::definition::json_schema_for::<WorkflowPromptToolInput>(),
        )
        .description("Generate the security review workflow prompt so it can be submitted as a normal turn.")
        .tag(ToolTag::ReadOnly)
        .always_load()
        .concurrency_safe(true),
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
            "tool_search",
            crate::entry::definition::json_schema_for::<ToolSearchToolInput>(),
        )
        .description("Search the tool catalog and optionally load deferred tools for later turns.")
        .tags([ToolTag::ReadOnly, ToolTag::Discovery])
        .always_load()
        .concurrency_safe(true)
        .host_capability(HostCapability::ListTools),
        PluginToolDecl::new(
            "todo_write",
            crate::entry::definition::json_schema_for::<TodoWriteToolInput>(),
        )
        .description("Replace the session todo list with a short execution plan and updated statuses.")
        .tags([ToolTag::Mutating, ToolTag::Planning])
        .always_load(),
        PluginToolDecl::new(
            "create_goal",
            crate::entry::definition::json_schema_for::<CreateGoalToolInput>(),
        )
        .description(
            "Create a goal only when explicitly requested by the user or system instructions. Starts a new active goal for this session and fails if one already exists. Set `token_budget` only when a budget was explicitly requested.",
        )
        .tags([ToolTag::Mutating, ToolTag::Goal])
        .always_load()
        .concurrency_safe(false)
        .host_capability(HostCapability::GoalRegistry),
        PluginToolDecl::new(
            "get_goal",
            crate::entry::definition::json_schema_for::<GetGoalToolInput>(),
        )
        .description("Get the current runtime goal for this session, including status, budgets, token and elapsed-time usage, and remaining token budget.")
        .tags([ToolTag::ReadOnly, ToolTag::Goal])
        .always_load()
        .concurrency_safe(true)
        .host_capability(HostCapability::GoalRegistry),
        PluginToolDecl::new(
            "clear_goal",
            crate::entry::definition::json_schema_for::<ClearGoalToolInput>(),
        )
        .description("Clear the current runtime goal for this session, if one exists.")
        .tags([ToolTag::Mutating, ToolTag::Goal])
        .always_load()
        .concurrency_safe(false)
        .host_capability(HostCapability::GoalRegistry),
        PluginToolDecl::new(
            "update_goal",
            crate::entry::definition::json_schema_for::<WorkflowUpdateGoalToolInput>(),
        )
        .description(
            "Update the existing runtime goal. Use this only to mark the goal achieved with `status = complete` once the objective is actually finished. Pause, resume, and budget-limit transitions are controlled by the user or system. When a budgeted goal completes, report the final usage guidance returned by the tool output to the user.",
        )
        .tags([ToolTag::Mutating, ToolTag::Goal])
        .always_load()
        .concurrency_safe(false)
        .host_capability(HostCapability::GoalRegistry),
        PluginToolDecl::new(
            "ask_user",
            crate::entry::definition::json_schema_for::<AskUserToolInput>(),
        )
        .description("Ask short questions and wait for answers.")
        .tags([ToolTag::ReadOnly, ToolTag::Interactive])
        .always_load()
        .concurrency_safe(false)
        .host_capability(HostCapability::AskUser),
        PluginToolDecl::new(
            "enter_plan_mode",
            crate::entry::definition::json_schema_for::<EnterPlanModeToolInput>(),
        )
        .description(
            "Enter plan mode. Allocates a fresh plan markdown file under .agena/plans/, blocks mutating tools, and asks the LLM to draft a plan. Pair with `exit_plan_mode` once the plan is complete.",
        )
        .tags([ToolTag::ReadOnly, ToolTag::Planning])
        .tag(ToolTag::FilesystemWrite)
        .path_access(PathAccessSpec {
            path: ".agena/plans".to_string(),
            kind: PathKind::Write,
        })
        .always_load()
        .concurrency_safe(true)
        .host_capability(HostCapability::PlanRegistry),
        PluginToolDecl::new(
            "exit_plan_mode",
            crate::entry::definition::json_schema_for::<ExitPlanModeToolInput>(),
        )
        .description(
            "Leave plan mode and return to normal tool execution. Surfaces a permission ask so the human can review the plan before approving the unblock.",
        )
        .tags([ToolTag::ReadOnly, ToolTag::Planning])
        .always_load()
        .concurrency_safe(true)
        .host_capability(HostCapability::PlanRegistry),
        PluginToolDecl::new(
            "enter_worktree",
            crate::entry::definition::json_schema_for::<EnterWorktreeToolInput>(),
        )
        .description(
            "Create or attach to a git worktree under .agena/worktrees and switch the session into it.",
        )
        .tags([ToolTag::Mutating, ToolTag::FilesystemWrite, ToolTag::Worktree])
        .concurrency_safe(false)
        .deferred_load()
        .host_capability(HostCapability::WorktreeRegistry),
        PluginToolDecl::new(
            "exit_worktree",
            crate::entry::definition::json_schema_for::<ExitWorktreeToolInput>(),
        )
        .description(
            "Leave the current worktree. action=keep preserves the worktree, action=remove deletes it (refuses unless discard_changes=true when there are uncommitted changes).",
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
        EventSubscription, HostClearGoalRequest, HostClearGoalResponse, HostCreateGoalRequest,
        HostCreateGoalResponse, HostEnterPlanModeRequest, HostEnterWorktreeRequest,
        HostExitPlanModeRequest, HostExitWorktreeRequest, HostGetGoalRequest, HostGetGoalResponse,
        HostGoal, HostGoalStatus, HostUpdateGoalRequest, HostUpdateGoalResponse, LogLevel,
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
    }

    impl TestHost {
        fn new() -> Self {
            Self {
                clear_goal_request_count: Mutex::new(0),
                update_goal_request: Mutex::new(None),
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
                name: "bash".to_string(),
                description: Some("Execute shell commands".to_string()),
                tags: vec![
                    crate::plugin::sdk::ToolTag::Mutating,
                    crate::plugin::sdk::ToolTag::Shell,
                ],
                deferred: true,
                plugin_id: None,
            }])
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

    #[tokio::test]
    async fn ask_user_invokes_host_without_executor_context() {
        let (plugin, _) = initialized_plugin().await;
        let output = plugin
            .tool_invoke(invoke_input(
                "ask_user",
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
            .tool_invoke(invoke_input("get_goal", GetGoalToolInput::default()))
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
            .tool_invoke(invoke_input(
                "create_goal",
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
            .tool_invoke(invoke_input("clear_goal", ClearGoalToolInput::default()))
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
            .tool_invoke(invoke_input(
                "update_goal",
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
    fn update_goal_entry_schema_only_exposes_complete_status() {
        let entry = entries()
            .into_iter()
            .find(|entry| entry.name == "update_goal")
            .expect("update_goal entry should exist");
        let properties = entry
            .input_schema
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
            entry
                .input_schema
                .get("$defs")
                .and_then(|defs| defs.get(definition_name))
                .or_else(|| {
                    entry
                        .input_schema
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
            .tool_invoke(invoke_input(
                "todo_write",
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
        let (plugin, _) = initialized_plugin().await;

        let enter_plan = plugin
            .tool_invoke(invoke_input(
                "enter_plan_mode",
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

        let enter_worktree = plugin
            .tool_invoke(invoke_input(
                "enter_worktree",
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
            .tool_invoke(invoke_input(
                "exit_worktree",
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
            .tool_invoke(invoke_input(
                "exit_plan_mode",
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
    }

    #[tokio::test]
    async fn tool_search_invokes_host_catalog_without_executor_context() {
        let (plugin, _) = initialized_plugin().await;
        let output = plugin
            .tool_invoke(invoke_input(
                "tool_search",
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
                assert_eq!(results, vec!["bash".to_string()]);
            }
            other => panic!("unexpected output: {other:?}"),
        }
    }

    #[tokio::test]
    async fn provided_workflow_entries_render_prompt_text() {
        let (plugin, _) = initialized_plugin().await;

        let review = plugin
            .tool_invoke(invoke_input(
                "workflow_review",
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

        let init = plugin
            .tool_invoke(invoke_input(
                "workflow_init",
                WorkflowPromptToolInput::default(),
            ))
            .await
            .expect("workflow_init invoke");
        assert_eq!(init.title, "init workflow");
        assert!(init.output_text.contains("Save the result to AGENA.md"));

        let security = plugin
            .tool_invoke(invoke_input(
                "workflow_security_review",
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
    }
}
