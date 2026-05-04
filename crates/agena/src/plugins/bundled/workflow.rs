//! First-party `agena.workflow` plugin: orchestration tools (task, tool_search,
//! todo_write, ask_user, enter_plan_mode, exit_plan_mode, enter_worktree,
//! exit_worktree).

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;

use crate::entry::{BuiltinExecution, ToolExecutionView, ask_user, tool_search};
use crate::message::{
    AskUserToolInput, BuiltinToolOutput, EnterPlanModeToolInput, EnterWorktreeToolInput,
    ExitPlanModeToolInput, ExitWorktreeToolInput, TaskToolInput, TodoItem, TodoPriority,
    TodoStatus, TodoWriteToolInput, ToolSearchToolInput,
};
use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::{
    AskUserOption as HostAskUserOption, AskUserQuestion as HostAskUserQuestion, AskUserRequest,
    HostClient, HostEnterPlanModeRequest, HostEnterWorktreeRequest, HostExitPlanModeRequest,
    HostExitWorktreeRequest, HostTodoItem, HostTodoPriority, HostTodoStatus, HostTodoWriteRequest,
    SpawnSubtaskRequest, ToolDescriptor,
};
use crate::plugin::sdk::{
    EntryBehavior as SdkEntryBehavior, HookSubscription, HostCapability, InitContext, InitOutcome,
    PlanModePolicy, Plugin, PluginEntryDecl, PluginManifest, Result as SdkResult, ToolInvokeInput,
    ToolInvokeOutput,
};

pub(crate) const WORKFLOW_PLUGIN_ID: &str = "agena.workflow";

pub(crate) struct WorkflowPlugin {
    host: RwLock<Option<Arc<dyn HostClient>>>,
}

impl WorkflowPlugin {
    pub(crate) fn new() -> Self {
        Self {
            host: RwLock::new(None),
        }
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
        Ok(crate::plugins::bundled::builtin::builtin_to_invoke_output(
            execution,
        ))
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

        let output = BuiltinToolOutput::Task {
            session_id,
            model_provider_id,
            model_id,
        };
        Ok(crate::plugins::bundled::builtin::builtin_to_invoke_output(
            BuiltinExecution::new(output, view),
        ))
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
        Ok(crate::plugins::bundled::builtin::builtin_to_invoke_output(
            execution,
        ))
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
            other => Err(PluginError::invalid_params(format!(
                "unknown workflow plugin entry '{other}'"
            ))),
        }
    }
}

fn entries() -> Vec<PluginEntryDecl> {
    vec![
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
        .behavior(SdkEntryBehavior::WriteSandboxed)
        .search_terms(["git", "worktree", "branch", "isolate"])
        .deferred_load()
        .host_capability(HostCapability::WorktreeRegistry),
        PluginEntryDecl::new(
            "exit_worktree",
            crate::entry::definition::json_schema_for::<ExitWorktreeToolInput>(),
        )
        .description(
            "Leave the current worktree. action=keep preserves the worktree, action=remove deletes it (refuses unless discard_changes=true when there are uncommitted changes).",
        )
        .behavior(SdkEntryBehavior::WriteSandboxed)
        .search_terms(["git", "worktree", "exit", "cleanup"])
        .deferred_load()
        .host_capability(HostCapability::WorktreeRegistry),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::sdk::host_api::{
        EventSubscription, HostEnterPlanModeRequest, HostEnterWorktreeRequest,
        HostExitPlanModeRequest, HostExitWorktreeRequest, LogLevel,
    };
    use crate::plugin::sdk::{EventEnvelope, EventFilter, PermissionAskInput, PermissionDecision};

    struct TestHost;

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
            Ok(crate::plugins::bundled::builtin::builtin_to_invoke_output(
                BuiltinExecution::new(
                    BuiltinToolOutput::TodoWrite {
                        items: vec![TodoItem {
                            content: "ship it".to_string(),
                            status: TodoStatus::InProgress,
                            priority: TodoPriority::High,
                        }],
                    },
                    ToolExecutionView::simple("Todo write", "Updated todo list with 1 item(s):"),
                ),
            ))
        }

        async fn enter_plan_mode(
            &self,
            _req: HostEnterPlanModeRequest,
        ) -> SdkResult<ToolInvokeOutput> {
            Ok(crate::plugins::bundled::builtin::builtin_to_invoke_output(
                BuiltinExecution::new(
                    BuiltinToolOutput::EnterPlanMode {
                        plan_path: "/tmp/plan.md".to_string(),
                        slug: "demo".to_string(),
                    },
                    ToolExecutionView::simple("Plan mode entered", "plan on"),
                ),
            ))
        }

        async fn exit_plan_mode(
            &self,
            _req: HostExitPlanModeRequest,
        ) -> SdkResult<ToolInvokeOutput> {
            Ok(crate::plugins::bundled::builtin::builtin_to_invoke_output(
                BuiltinExecution::new(
                    BuiltinToolOutput::ExitPlanMode {
                        approved: true,
                        plan_path: "/tmp/plan.md".to_string(),
                    },
                    ToolExecutionView::simple("Plan mode exited", "plan off"),
                ),
            ))
        }

        async fn enter_worktree(
            &self,
            req: HostEnterWorktreeRequest,
        ) -> SdkResult<ToolInvokeOutput> {
            assert_eq!(req.name.as_deref(), Some("demo"));
            assert_eq!(req.path, None);
            Ok(crate::plugins::bundled::builtin::builtin_to_invoke_output(
                BuiltinExecution::new(
                    BuiltinToolOutput::EnterWorktree {
                        path: "/tmp/wt".to_string(),
                        branch: "agena/demo".to_string(),
                    },
                    ToolExecutionView::simple("Worktree", "entered worktree"),
                ),
            ))
        }

        async fn exit_worktree(&self, req: HostExitWorktreeRequest) -> SdkResult<ToolInvokeOutput> {
            assert_eq!(req.action, "keep");
            assert!(!req.discard_changes);
            Ok(crate::plugins::bundled::builtin::builtin_to_invoke_output(
                BuiltinExecution::new(
                    BuiltinToolOutput::ExitWorktree {
                        action: "keep".to_string(),
                        path: "/tmp/wt".to_string(),
                    },
                    ToolExecutionView::simple("Worktree", "exited worktree"),
                ),
            ))
        }
    }

    async fn initialized_plugin() -> WorkflowPlugin {
        let plugin = WorkflowPlugin::new();
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
                Arc::new(TestHost),
            )
            .await
            .expect("workflow plugin init");
        plugin
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
        let plugin = initialized_plugin().await;
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
        let envelope =
            crate::plugins::bundled::builtin::payload_to_builtin_envelope(output.payload.as_ref())
                .unwrap();
        match envelope.output {
            BuiltinToolOutput::AskUser { answers } => {
                assert_eq!(answers["color"], vec!["blue".to_string()]);
            }
            other => panic!("unexpected output: {other:?}"),
        }
    }

    #[tokio::test]
    async fn task_invokes_host_without_executor_context() {
        let plugin = initialized_plugin().await;
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
        let envelope =
            crate::plugins::bundled::builtin::payload_to_builtin_envelope(output.payload.as_ref())
                .unwrap();
        match envelope.output {
            BuiltinToolOutput::Task {
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
        let plugin = initialized_plugin().await;
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
        let envelope =
            crate::plugins::bundled::builtin::payload_to_builtin_envelope(output.payload.as_ref())
                .unwrap();
        match envelope.output {
            BuiltinToolOutput::TodoWrite { items } => {
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
        let plugin = initialized_plugin().await;

        let enter_plan = plugin
            .tool_invoke(invoke_input(
                "enter_plan_mode",
                EnterPlanModeToolInput::default(),
            ))
            .await
            .expect("enter_plan_mode host invoke");
        let enter_plan = crate::plugins::bundled::builtin::payload_to_builtin_envelope(
            enter_plan.payload.as_ref(),
        )
        .unwrap();
        match enter_plan.output {
            BuiltinToolOutput::EnterPlanMode { plan_path, slug } => {
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
        let enter_worktree = crate::plugins::bundled::builtin::payload_to_builtin_envelope(
            enter_worktree.payload.as_ref(),
        )
        .unwrap();
        match enter_worktree.output {
            BuiltinToolOutput::EnterWorktree { path, branch } => {
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
        let exit_worktree = crate::plugins::bundled::builtin::payload_to_builtin_envelope(
            exit_worktree.payload.as_ref(),
        )
        .unwrap();
        match exit_worktree.output {
            BuiltinToolOutput::ExitWorktree { action, path } => {
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
        let exit_plan = crate::plugins::bundled::builtin::payload_to_builtin_envelope(
            exit_plan.payload.as_ref(),
        )
        .unwrap();
        match exit_plan.output {
            BuiltinToolOutput::ExitPlanMode {
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
        let plugin = initialized_plugin().await;
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
        let envelope =
            crate::plugins::bundled::builtin::payload_to_builtin_envelope(output.payload.as_ref())
                .unwrap();
        match envelope.output {
            BuiltinToolOutput::ToolSearch { results, .. } => {
                assert_eq!(results, vec!["bash".to_string()]);
            }
            other => panic!("unexpected output: {other:?}"),
        }
    }
}
