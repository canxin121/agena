//! First-party `agena.workflow` plugin: orchestration tools (task, tool_search,
//! todo_write, create_goal, get_goal, update_goal, ask_user, enter_plan_mode,
//! exit_plan_mode, enter_worktree, exit_worktree).

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use crate::entry::{FirstPartyExecution, ToolExecutionView, ask_user, tool_search};
use crate::message::{
    AskUserToolInput, ClearGoalToolInput, CreateGoalToolInput, EnterPlanModeToolInput,
    EnterWorktreeToolInput, ExitPlanModeToolInput, ExitWorktreeToolInput, FirstPartyToolOutput,
    GetGoalToolInput, TaskToolInput, TodoItem, TodoPriority, TodoStatus, TodoWriteToolInput,
    ToolSearchToolInput, UpdateGoalStatus, WorkflowPromptToolInput,
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
    EntryBehavior as SdkEntryBehavior, HookSubscription, HostCapability, InitContext, InitOutcome,
    PathRequest, PlanModePolicy, Plugin, PluginEntryDecl, PluginManifest, Result as SdkResult,
    ToolInvokeInput, ToolInvokeOutput,
};

pub(crate) const WORKFLOW_PLUGIN_ID: &str = "agena.workflow";

fn deserialize_update_goal_token_budget<'de, D>(
    deserializer: D,
) -> Result<Option<Option<u64>>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Some(Option::<u64>::deserialize(deserializer)?))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
struct WorkflowUpdateGoalToolInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    pub status: UpdateGoalStatus,
    /// Omit to preserve the existing budget, pass `null` to clear it, or an
    /// integer to set a new budget.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_update_goal_token_budget"
    )]
    pub token_budget: Option<Option<u64>>,
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

    fn bundled_workflow(name: &str) -> SdkResult<agena_skills::Skill> {
        agena_skills::bundled::all()
            .map_err(|err| PluginError::new(err.to_string()))?
            .into_iter()
            .find(|skill| skill.frontmatter.name == name)
            .ok_or_else(|| PluginError::new(format!("missing bundled workflow '{name}'")))
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

    async fn invoke_bundled_workflow(
        &self,
        workflow_name: &str,
        input: &WorkflowPromptToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let workflow = Self::bundled_workflow(workflow_name)?;
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
        let behavior_label = descriptor.behavior.unwrap_or_else(|| {
            if descriptor.read_only {
                "read_only".to_string()
            } else {
                "mutating".to_string()
            }
        });
        tool_search::SearchableTool {
            name: descriptor.name,
            description: descriptor.description.unwrap_or_default(),
            search_terms: descriptor.search_terms,
            behavior_label,
            read_only: descriptor.read_only,
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
        let payload =
            serde_json::to_value(&response).map_err(|err| PluginError::new(err.to_string()))?;
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
            serde_json::to_value(&response).map_err(|err| PluginError::new(err.to_string()))?;
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
                objective: input.objective.clone(),
                status: Some(match input.status {
                    UpdateGoalStatus::Active => HostGoalStatus::Active,
                    UpdateGoalStatus::Paused => HostGoalStatus::Paused,
                    UpdateGoalStatus::Complete => HostGoalStatus::Completed,
                }),
                token_budget: input.token_budget,
            })
            .await?;
        let payload =
            serde_json::to_value(&response).map_err(|err| PluginError::new(err.to_string()))?;
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
        Ok(crate::plugins::bundled::router::first_party_to_invoke_output(execution))
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

        let output = FirstPartyToolOutput::Task {
            session_id,
            model_provider_id,
            model_id,
        };
        Ok(
            crate::plugins::bundled::router::first_party_to_invoke_output(
                FirstPartyExecution::new(output, view),
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
        Ok(crate::plugins::bundled::router::first_party_to_invoke_output(execution))
    }
}

pub(crate) fn new_plugin() -> WorkflowPlugin {
    WorkflowPlugin::new()
}

#[async_trait]
impl Plugin for WorkflowPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder("agena-workflow", env!("CARGO_PKG_VERSION"))
            .description("Workflow orchestration tools exposed as a first-party plugin.")
            .hooks(HookSubscription::TOOL_INVOKE)
            .entries(entries())
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
                self.invoke_bundled_workflow("init", &serde_json::from_value(input.input)?)
                    .await
            }
            "workflow_review" => {
                self.invoke_bundled_workflow("review", &serde_json::from_value(input.input)?)
                    .await
            }
            "workflow_security_review" => {
                self.invoke_bundled_workflow(
                    "security-review",
                    &serde_json::from_value(input.input)?,
                )
                .await
            }
            other => Err(PluginError::invalid_params(format!(
                "unknown workflow plugin entry '{other}'"
            ))),
        }
    }

    async fn permission_paths(
        &self,
        tool_name: &str,
        input: &serde_json::Value,
    ) -> SdkResult<Vec<PathRequest>> {
        match tool_name {
            "enter_plan_mode" => Ok(vec![PathRequest::write(".agena/plans")]),
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

fn entries() -> Vec<PluginEntryDecl> {
    vec![
        PluginEntryDecl::new(
            "workflow_init",
            crate::entry::definition::json_schema_for::<WorkflowPromptToolInput>(),
        )
        .description("Generate the bundled init workflow prompt so it can be submitted as a normal turn.")
        .behavior(SdkEntryBehavior::ReadOnly)
        .search_terms(["bootstrap", "agents", "claude", "init workflow"])
        .always_load()
        .expose_as("init"),
        PluginEntryDecl::new(
            "workflow_review",
            crate::entry::definition::json_schema_for::<WorkflowPromptToolInput>(),
        )
        .description("Generate the bundled review workflow prompt so it can be submitted as a normal turn.")
        .behavior(SdkEntryBehavior::ReadOnly)
        .search_terms(["review", "code review", "audit branch"])
        .always_load()
        .expose_as("review"),
        PluginEntryDecl::new(
            "workflow_security_review",
            crate::entry::definition::json_schema_for::<WorkflowPromptToolInput>(),
        )
        .description("Generate the bundled security review workflow prompt so it can be submitted as a normal turn.")
        .behavior(SdkEntryBehavior::ReadOnly)
        .search_terms(["security review", "security audit", "audit branch"])
        .always_load()
        .expose_as("security-review"),
        PluginEntryDecl::new(
            "task",
            crate::entry::definition::json_schema_for::<TaskToolInput>(),
        )
        .description(
            "Create or resume a typed subagent task session for explore, implement, or verify delegated work.",
        )
        .behavior(SdkEntryBehavior::Task)
        .search_terms(["delegate", "subagent", "parallel work"])
        .deferred_load()
        .host_capability(HostCapability::SpawnSubtask),
        PluginEntryDecl::new(
            "tool_search",
            crate::entry::definition::json_schema_for::<ToolSearchToolInput>(),
        )
        .description("Search the tool catalog and optionally load deferred tools for later turns.")
        .behavior(SdkEntryBehavior::ReadOnly)
        .search_terms(["discover tools", "load tools", "find capability"])
        .always_load()
        .host_capability(HostCapability::ListTools),
        PluginEntryDecl::new(
            "todo_write",
            crate::entry::definition::json_schema_for::<TodoWriteToolInput>(),
        )
        .description("Replace the session todo list with a short execution plan and updated statuses.")
        .behavior(SdkEntryBehavior::ReadOnly)
        .search_terms(["plan", "todo", "track progress"])
        .always_load(),
        PluginEntryDecl::new(
            "create_goal",
            crate::entry::definition::json_schema_for::<CreateGoalToolInput>(),
        )
        .description(
            "Create a runtime goal for this session so work can continue autonomously toward a specific objective.",
        )
        .behavior(SdkEntryBehavior::Mutating)
        .search_terms(["goal", "objective", "set goal", "budget"])
        .always_load()
        .host_capability(HostCapability::GoalRegistry),
        PluginEntryDecl::new(
            "get_goal",
            crate::entry::definition::json_schema_for::<GetGoalToolInput>(),
        )
        .description("Read the current runtime goal for this session, including status and budget usage.")
        .behavior(SdkEntryBehavior::ReadOnly)
        .search_terms(["goal", "objective", "budget", "status"])
        .always_load()
        .host_capability(HostCapability::GoalRegistry),
        PluginEntryDecl::new(
            "clear_goal",
            crate::entry::definition::json_schema_for::<ClearGoalToolInput>(),
        )
        .description("Clear the current runtime goal for this session, if one exists.")
        .behavior(SdkEntryBehavior::Mutating)
        .search_terms(["goal", "clear goal", "remove goal", "delete goal"])
        .always_load()
        .host_capability(HostCapability::GoalRegistry),
        PluginEntryDecl::new(
            "update_goal",
            crate::entry::definition::json_schema_for::<WorkflowUpdateGoalToolInput>(),
        )
        .description(
            "Update the current runtime goal, including status transitions and optional objective or budget changes. Omit `token_budget` to preserve it, pass `null` to clear it, or an integer to set it.",
        )
        .behavior(SdkEntryBehavior::Mutating)
        .search_terms([
            "goal",
            "pause goal",
            "resume goal",
            "complete goal",
            "budget",
            "clear budget",
        ])
        .always_load()
        .host_capability(HostCapability::GoalRegistry),
        PluginEntryDecl::new(
            "ask_user",
            crate::entry::definition::json_schema_for::<AskUserToolInput>(),
        )
        .description("Ask short questions and wait for answers.")
        .behavior(SdkEntryBehavior::ReadOnly)
        .search_terms([
            "ask user",
            "clarify requirement",
            "human input",
            "single select",
            "multi select",
            "custom answer",
            "request user input",
        ])
        .always_load()
        .concurrency_safe(false)
        .requires_user_interaction(true)
        .host_capability(HostCapability::AskUser),
        PluginEntryDecl::new(
            "enter_plan_mode",
            crate::entry::definition::json_schema_for::<EnterPlanModeToolInput>(),
        )
        .description(
            "Enter plan mode. Allocates a fresh plan markdown file under .agena/plans/, blocks mutating tools, and asks the LLM to draft a plan. Pair with `exit_plan_mode` once the plan is complete.",
        )
        .behavior(SdkEntryBehavior::ReadOnly)
        .search_terms(["plan", "design", "approach", "outline"])
        .tag("filesystem_write")
        .always_load()
        .plan_mode_policy(PlanModePolicy::Allowed)
        .host_capability(HostCapability::PlanRegistry),
        PluginEntryDecl::new(
            "exit_plan_mode",
            crate::entry::definition::json_schema_for::<ExitPlanModeToolInput>(),
        )
        .description(
            "Leave plan mode and return to normal tool execution. Surfaces a permission ask so the human can review the plan before approving the unblock.",
        )
        .behavior(SdkEntryBehavior::ReadOnly)
        .search_terms(["plan", "approve", "exit"])
        .always_load()
        .plan_mode_policy(PlanModePolicy::Allowed)
        .host_capability(HostCapability::PlanRegistry),
        PluginEntryDecl::new(
            "enter_worktree",
            crate::entry::definition::json_schema_for::<EnterWorktreeToolInput>(),
        )
        .description(
            "Create or attach to a git worktree under .agena/worktrees and switch the session into it.",
        )
        .behavior(SdkEntryBehavior::Mutating)
        .search_terms(["git", "worktree", "branch", "isolate"])
        .tag("filesystem_write")
        .deferred_load()
        .host_capability(HostCapability::WorktreeRegistry),
        PluginEntryDecl::new(
            "exit_worktree",
            crate::entry::definition::json_schema_for::<ExitWorktreeToolInput>(),
        )
        .description(
            "Leave the current worktree. action=keep preserves the worktree, action=remove deletes it (refuses unless discard_changes=true when there are uncommitted changes).",
        )
        .behavior(SdkEntryBehavior::Mutating)
        .search_terms(["git", "worktree", "exit", "cleanup"])
        .tag("filesystem_write")
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
                search_terms: vec!["shell".to_string()],
                behavior: Some("mutating".to_string()),
                deferred: true,
                read_only: false,
                plugin_id: None,
            }])
        }

        async fn todo_write(&self, req: HostTodoWriteRequest) -> SdkResult<ToolInvokeOutput> {
            assert_eq!(req.items.len(), 1);
            assert_eq!(req.items[0].content, "ship it");
            Ok(
                crate::plugins::bundled::router::first_party_to_invoke_output(
                    FirstPartyExecution::new(
                        FirstPartyToolOutput::TodoWrite {
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
                crate::plugins::bundled::router::first_party_to_invoke_output(
                    FirstPartyExecution::new(
                        FirstPartyToolOutput::EnterPlanMode {
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
                crate::plugins::bundled::router::first_party_to_invoke_output(
                    FirstPartyExecution::new(
                        FirstPartyToolOutput::ExitPlanMode {
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
                crate::plugins::bundled::router::first_party_to_invoke_output(
                    FirstPartyExecution::new(
                        FirstPartyToolOutput::EnterWorktree {
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
                crate::plugins::bundled::router::first_party_to_invoke_output(
                    FirstPartyExecution::new(
                        FirstPartyToolOutput::ExitWorktree {
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
        let envelope = crate::plugins::bundled::router::payload_to_first_party_envelope(
            output.payload.as_ref(),
        )
        .unwrap();
        match envelope.output {
            FirstPartyToolOutput::AskUser { answers } => {
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
                    objective: None,
                    status: UpdateGoalStatus::Complete,
                    token_budget: None,
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
    }

    #[tokio::test]
    async fn update_goal_passes_through_non_complete_fields() {
        let (plugin, host) = initialized_plugin().await;
        let output = plugin
            .tool_invoke(invoke_input(
                "update_goal",
                WorkflowUpdateGoalToolInput {
                    objective: Some("resume and ship it".to_string()),
                    status: UpdateGoalStatus::Paused,
                    token_budget: Some(Some(256)),
                },
            ))
            .await
            .expect("update_goal host invoke");

        assert_eq!(
            host.recorded_update_goal_request(),
            Some(RecordedUpdateGoalRequest {
                objective: Some("resume and ship it".to_string()),
                status: Some(HostGoalStatus::Paused),
                token_budget: Some(Some(256)),
            })
        );
        assert!(output.output_text.contains("Goal 7 is paused"));
        assert_eq!(
            output
                .payload
                .as_ref()
                .and_then(|payload| payload["goal"]["objective"].as_str()),
            Some("resume and ship it")
        );
        assert_eq!(
            output
                .payload
                .as_ref()
                .and_then(|payload| payload["goal"]["token_budget"].as_u64()),
            Some(256)
        );
    }

    #[tokio::test]
    async fn update_goal_allows_explicit_budget_clear_with_null() {
        let (plugin, host) = initialized_plugin().await;
        let output = plugin
            .tool_invoke(ToolInvokeInput {
                tool_name: "update_goal".to_string(),
                session_id: 1,
                call_id: 2,
                workspace_root: "/tmp".to_string(),
                input: serde_json::json!({
                    "status": "active",
                    "token_budget": null,
                }),
            })
            .await
            .expect("update_goal host invoke");

        assert_eq!(
            host.recorded_update_goal_request(),
            Some(RecordedUpdateGoalRequest {
                objective: None,
                status: Some(HostGoalStatus::Active),
                token_budget: Some(None),
            })
        );
        assert!(output.output_text.contains("Tokens: 48."));
        assert_eq!(
            output
                .payload
                .as_ref()
                .map(|payload| payload["goal"]["token_budget"].is_null()),
            Some(true)
        );
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
        let envelope = crate::plugins::bundled::router::payload_to_first_party_envelope(
            output.payload.as_ref(),
        )
        .unwrap();
        match envelope.output {
            FirstPartyToolOutput::Task {
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
        let envelope = crate::plugins::bundled::router::payload_to_first_party_envelope(
            output.payload.as_ref(),
        )
        .unwrap();
        match envelope.output {
            FirstPartyToolOutput::TodoWrite { items } => {
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
        let enter_plan = crate::plugins::bundled::router::payload_to_first_party_envelope(
            enter_plan.payload.as_ref(),
        )
        .unwrap();
        match enter_plan.output {
            FirstPartyToolOutput::EnterPlanMode { plan_path, slug } => {
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
        let enter_worktree = crate::plugins::bundled::router::payload_to_first_party_envelope(
            enter_worktree.payload.as_ref(),
        )
        .unwrap();
        match enter_worktree.output {
            FirstPartyToolOutput::EnterWorktree { path, branch } => {
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
        let exit_worktree = crate::plugins::bundled::router::payload_to_first_party_envelope(
            exit_worktree.payload.as_ref(),
        )
        .unwrap();
        match exit_worktree.output {
            FirstPartyToolOutput::ExitWorktree { action, path } => {
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
        let exit_plan = crate::plugins::bundled::router::payload_to_first_party_envelope(
            exit_plan.payload.as_ref(),
        )
        .unwrap();
        match exit_plan.output {
            FirstPartyToolOutput::ExitPlanMode {
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
        let envelope = crate::plugins::bundled::router::payload_to_first_party_envelope(
            output.payload.as_ref(),
        )
        .unwrap();
        match envelope.output {
            FirstPartyToolOutput::ToolSearch { results, .. } => {
                assert_eq!(results, vec!["bash".to_string()]);
            }
            other => panic!("unexpected output: {other:?}"),
        }
    }

    #[tokio::test]
    async fn bundled_workflow_entries_render_prompt_text() {
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
        assert!(init.output_text.contains("Save the result to AGENTS.md"));

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
