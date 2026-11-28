//! Claude's session task tools backed by Agena's private plugin storage.

use std::sync::{Arc, OnceLock};

use agena_plugin_sdk::{
    ApplicableSessions, HostClient, ImportedTaskBeforeInput, ImportedTaskChange, InitContext,
    InitOutcome, Plugin, PluginError, PluginManifest, Result, SessionKind, ToolContract,
    ToolDefinition, ToolDocs, ToolInvokeInput, ToolInvokeOutput, ToolTag,
    host_api::{
        HostStorageGetRequest, HostStorageScope, HostStorageSetRequest, HostStorageVisibility,
    },
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};

use crate::{PluginPackage, PluginsConfig, StaticPluginRegistration};

pub(crate) const PLUGIN_ID: &str = "agena.task";
const NAMESPACE: &str = "claude_tasks";
const STORAGE_KEY: &str = "tasks";
const MAX_TASKS: usize = 256;
const MAX_STORAGE: usize = 1024 * 1024;
const MAX_CONFLICT_RETRIES: usize = 16;

pub(crate) fn register(
    config: &mut PluginsConfig,
    static_plugins: &mut Vec<StaticPluginRegistration>,
) {
    let has_imported_plugin = config
        .list
        .values()
        .any(|entry| entry.enabled && matches!(entry.package, PluginPackage::AgentPlugin { .. }));
    if has_imported_plugin {
        config.list.entry(PLUGIN_ID.into()).or_default();
    }
    let key = PLUGIN_ID.parse().expect("valid task bridge ID");
    if config
        .list
        .get(PLUGIN_ID)
        .is_some_and(|entry| matches!(entry.package, PluginPackage::Static {}))
        && !static_plugins.iter().any(|entry| entry.key == key)
    {
        static_plugins.push(StaticPluginRegistration::new(key, TaskPlugin::default()));
    }
}

#[derive(Default)]
struct TaskPlugin {
    host: OnceLock<Arc<dyn HostClient>>,
}

fn tool(
    name: &str,
    summary: &str,
    properties: Value,
    required: &[&str],
    mutate: bool,
) -> ToolDefinition {
    let mut tags = vec![ToolTag::Planning];
    if mutate {
        tags.push(ToolTag::Mutate);
    } else {
        tags.extend([ToolTag::Query, ToolTag::ReadOnly]);
    }
    ToolDefinition {
        name: name.into(),
        contract: ToolContract {
            input_schema: json!({
                "type": "object", "properties": properties, "required": required,
                "additionalProperties": false,
            }),
            ..Default::default()
        },
        docs: ToolDocs {
            summary: Some(summary.into()),
            ..Default::default()
        },
        runtime: Default::default(),
        tags,
        applies_to: ApplicableSessions::only(SessionKind::Project),
    }
}

#[async_trait]
impl Plugin for TaskPlugin {
    fn manifest(&self) -> PluginManifest {
        let mut manifest = PluginManifest::new("agena", "task", env!("CARGO_PKG_VERSION"));
        manifest.display_name = Some("Claude-compatible Tasks".into());
        manifest.summary =
            Some("Track tasks and dependencies in the current project conversation.".into());
        let id = json!({"type": "string", "minLength": 1});
        let subject = json!({"type": "string", "minLength": 1});
        let description = json!({"type": "string"});
        let active_form = json!({"type": "string", "description": "Present-continuous text for the in-progress task."});
        let metadata = json!({"type": "object"});
        let dependencies =
            json!({"type": "array", "items": id, "uniqueItems": true, "maxItems": MAX_TASKS});
        manifest.tools = vec![
            tool(
                "create",
                "Create a pending task (Claude TaskCreate).",
                json!({
                    "subject": subject, "description": description, "activeForm": active_form, "metadata": metadata,
                }),
                &["subject", "description"],
                true,
            ),
            tool(
                "get",
                "Read a task's details and dependencies (Claude TaskGet).",
                json!({"taskId": id}),
                &["taskId"],
                false,
            ),
            tool(
                "list",
                "List this conversation's tasks and open blockers (Claude TaskList).",
                json!({}),
                &[],
                false,
            ),
            tool(
                "update",
                "Update a task's details, assignment, dependencies or status (Claude TaskUpdate).",
                json!({
                    "taskId": id, "subject": subject, "description": description, "activeForm": active_form,
                    "metadata": metadata, "owner": {"type": "string"}, "addBlocks": dependencies,
                    "addBlockedBy": dependencies,
                    "status": {"enum": ["pending", "in_progress", "completed", "deleted"]},
                }),
                &["taskId"],
                true,
            ),
            tool(
                "todo_write",
                "Replace the current conversation's checklist (Claude TodoWrite).",
                json!({
                    "todos": {
                        "type": "array", "maxItems": MAX_TASKS,
                        "items": {
                            "type": "object",
                            "properties": {
                                "content": subject,
                                "activeForm": active_form,
                                "status": {"enum": ["pending", "in_progress", "completed"]},
                            },
                            "required": ["content", "activeForm", "status"],
                            "additionalProperties": false,
                        },
                    },
                }),
                &["todos"],
                true,
            ),
        ];
        manifest
    }

    async fn init(&self, _context: InitContext, host: Arc<dyn HostClient>) -> Result<InitOutcome> {
        self.host
            .set(host)
            .map_err(|_| PluginError::internal("task bridge already initialized"))?;
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> Result<ToolInvokeOutput> {
        if input.session_id <= 0 || input.workspace_root.is_empty() {
            return Err(PluginError::invalid_params(
                "task tools require a project conversation",
            ));
        }
        let host = self
            .host
            .get()
            .ok_or_else(|| PluginError::internal("task store is unavailable"))?;
        let store = TaskStore {
            host: host.as_ref(),
            session: input.session_id,
        };
        let (summary, result) = match input.tool_name.as_str() {
            "create" => ("Created task", store.create(parse(input.input)?).await?),
            "update" => ("Updated task", store.update(parse(input.input)?).await?),
            "todo_write" => (
                "Updated checklist",
                store.replace_todos(parse(input.input)?).await?,
            ),
            "get" => {
                let request: GetInput = parse(input.input)?;
                let (state, _) = store.load().await?;
                (
                    "Read task",
                    json!({"task": state.find(&request.task_id).map(|task| state.details(task))}),
                )
            }
            "list" => {
                let (state, _) = store.load().await?;
                ("Listed tasks", json!({"tasks": state.list()}))
            }
            _ => return Err(PluginError::not_implemented("task tool")),
        };
        Ok(ToolInvokeOutput::from_parts(
            summary,
            summary,
            result.to_string(),
            Some(result),
            Default::default(),
            Vec::new(),
        ))
    }
}

fn parse<T: DeserializeOwned>(input: Value) -> Result<T> {
    serde_json::from_value(input).map_err(|error| PluginError::invalid_params_error(&error))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateInput {
    subject: String,
    description: String,
    active_form: Option<String>,
    #[serde(default)]
    metadata: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GetInput {
    task_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateInput {
    task_id: String,
    subject: Option<String>,
    description: Option<String>,
    active_form: Option<String>,
    owner: Option<String>,
    status: Option<TaskStatus>,
    #[serde(default)]
    metadata: Map<String, Value>,
    #[serde(default)]
    add_blocks: Vec<String>,
    #[serde(default)]
    add_blocked_by: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TodoWriteInput {
    todos: Vec<Todo>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Todo {
    content: String,
    active_form: String,
    status: TodoStatus,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Deleted,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Task {
    id: String,
    subject: String,
    description: String,
    status: TaskStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_form: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    owner: Option<String>,
    blocked_by: Vec<String>,
    metadata: Map<String, Value>,
}

#[derive(Serialize, Deserialize)]
struct Tasks {
    next_id: u64,
    entries: Vec<Task>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    todos: Vec<Todo>,
}

impl Default for Tasks {
    fn default() -> Self {
        Self {
            next_id: 1,
            entries: Vec::new(),
            todos: Vec::new(),
        }
    }
}

impl Tasks {
    fn find(&self, id: &str) -> Option<&Task> {
        self.entries.iter().find(|task| task.id == id)
    }

    fn details(&self, task: &Task) -> Value {
        let mut result = json!(task);
        result["blocks"] = json!(
            self.entries
                .iter()
                .filter(|other| other.blocked_by.contains(&task.id))
                .map(|other| &other.id)
                .collect::<Vec<_>>()
        );
        result
    }

    fn list(&self) -> Vec<Value> {
        let mut entries = self.entries.iter().collect::<Vec<_>>();
        entries.sort_by_key(|task| task.id.parse::<u64>().unwrap_or(u64::MAX));
        entries.into_iter().map(|task| {
            let blockers = task.blocked_by.iter().filter(|id| self.find(id).is_some_and(|blocker| blocker.status != TaskStatus::Completed)).collect::<Vec<_>>();
            let mut row = json!({"id": task.id, "subject": task.subject, "status": task.status, "blockedBy": blockers});
            if let Some(owner) = &task.owner {
                row["owner"] = json!(owner);
            }
            row
        }).collect()
    }

    fn update(&mut self, request: &UpdateInput, current: &Task) -> Result<Value> {
        let id = &current.id;
        if request.status == Some(TaskStatus::Deleted) {
            self.entries.retain(|task| task.id != *id);
            for task in &mut self.entries {
                task.blocked_by.retain(|blocker| blocker != id);
            }
            return Ok(json!({"taskId": id, "success": true, "status": "deleted"}));
        }
        let mut updated = current.clone();
        if let Some(subject) = &request.subject {
            updated.subject = task_subject(subject)?;
        }
        if let Some(description) = &request.description {
            updated.description.clone_from(description);
        }
        if let Some(active_form) = &request.active_form {
            updated.active_form = Some(active_form.clone());
        }
        if let Some(owner) = &request.owner {
            updated.owner = (!owner.trim().is_empty()).then(|| owner.clone());
        }
        if let Some(status) = request.status {
            updated.status = status;
        }
        for (key, value) in &request.metadata {
            if value.is_null() {
                updated.metadata.remove(key);
            } else {
                updated.metadata.insert(key.clone(), value.clone());
            }
        }
        for related in request.add_blocked_by.iter().chain(&request.add_blocks) {
            if related == id || self.find(related).is_none() {
                return Err(PluginError::invalid_params(format!(
                    "invalid dependency {related:?} for task {id}"
                )));
            }
        }
        for blocker in &request.add_blocked_by {
            add_blocker(&mut updated, blocker);
        }
        let status = updated.status;
        for task in &mut self.entries {
            if task.id == *id {
                task.clone_from(&updated);
            } else if request.add_blocks.contains(&task.id) {
                add_blocker(task, id);
            }
        }
        Ok(json!({"taskId": id, "success": true, "status": status}))
    }
}

fn task_subject(subject: &str) -> Result<String> {
    let subject = subject.trim();
    if subject.is_empty() {
        return Err(PluginError::invalid_params(
            "a task subject cannot be empty",
        ));
    }
    Ok(subject.into())
}

fn add_blocker(task: &mut Task, blocker: &String) {
    if !task.blocked_by.contains(blocker) {
        task.blocked_by.push(blocker.clone());
    }
}

struct TaskStore<'a> {
    host: &'a dyn HostClient,
    session: i64,
}

impl TaskStore<'_> {
    async fn load(&self) -> Result<(Tasks, Option<String>)> {
        let response = self
            .host
            .storage_get(HostStorageGetRequest {
                scope: HostStorageScope::Session,
                visibility: HostStorageVisibility::Private,
                namespace: NAMESPACE.into(),
                key: STORAGE_KEY.into(),
            })
            .await?;
        let state = response
            .value
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|error| PluginError::internal_error(&error))?
            .unwrap_or_default();
        Ok((state, response.value))
    }

    async fn commit<T>(&self, mut change: impl FnMut(&mut Tasks) -> Result<T>) -> Result<T> {
        for _ in 0..MAX_CONFLICT_RETRIES {
            let (mut state, expected) = self.load().await?;
            let result = change(&mut state)?;
            let value = serde_json::to_string(&state)
                .map_err(|error| PluginError::internal_error(&error))?;
            if value.len() > MAX_STORAGE {
                return Err(PluginError::invalid_params(
                    "task storage would exceed 1 MiB",
                ));
            }
            let committed = self
                .host
                .storage_compare_exchange(
                    HostStorageSetRequest {
                        scope: HostStorageScope::Session,
                        visibility: HostStorageVisibility::Private,
                        namespace: NAMESPACE.into(),
                        key: STORAGE_KEY.into(),
                        value,
                    },
                    expected,
                )
                .await?;
            if committed {
                return Ok(result);
            }
        }
        Err(PluginError::invalid_params(
            "the task list is busy, retry the task operation",
        ))
    }

    async fn check(&self, task: &Task, change: ImportedTaskChange) -> Result<()> {
        self.host
            .check_imported_task(ImportedTaskBeforeInput {
                session_id: self.session,
                change,
                task_id: task.id.clone(),
                subject: task.subject.clone(),
                description: task.description.clone(),
                agent_id: None,
            })
            .await
    }

    async fn replace_todos(&self, input: TodoWriteInput) -> Result<Value> {
        if input.todos.len() > MAX_TASKS
            || input
                .todos
                .iter()
                .any(|todo| todo.content.trim().is_empty())
        {
            return Err(PluginError::invalid_params(
                "the checklist must contain at most 256 todos with nonempty content",
            ));
        }
        self.commit(|state| {
            let previous = std::mem::replace(&mut state.todos, input.todos.clone());
            Ok(json!({"oldTodos": previous, "newTodos": state.todos}))
        })
        .await
    }

    async fn create(&self, input: CreateInput) -> Result<Value> {
        let subject = task_subject(&input.subject)?;
        let id = self
            .commit(|state| {
                check_capacity(state)?;
                let id = state.next_id;
                state.next_id = id
                    .checked_add(1)
                    .ok_or_else(|| PluginError::internal("task IDs exhausted"))?;
                Ok(id.to_string())
            })
            .await?;
        let task = Task {
            id,
            subject,
            description: input.description,
            status: TaskStatus::Pending,
            active_form: input.active_form,
            metadata: input.metadata,
            owner: None,
            blocked_by: Vec::new(),
        };
        self.check(&task, ImportedTaskChange::Create).await?;
        self.commit(|state| {
            check_capacity(state)?;
            state.entries.push(task.clone());
            Ok(json!({"task": {"id": task.id, "subject": task.subject}}))
        })
        .await
    }

    async fn update(&self, input: UpdateInput) -> Result<Value> {
        let (mut state, _) = self.load().await?;
        let current = state.find(&input.task_id).cloned().ok_or_else(|| {
            PluginError::invalid_params(format!("task {:?} was not found", input.task_id))
        })?;
        state.update(&input, &current)?;
        if current.status != TaskStatus::Completed && input.status == Some(TaskStatus::Completed) {
            let updated = state
                .find(&input.task_id)
                .expect("completing a task keeps its record");
            self.check(updated, ImportedTaskChange::Complete).await?;
        }
        self.commit(|state| {
            if state.find(&input.task_id) != Some(&current) {
                return Err(PluginError::invalid_params(
                    "the task changed, read its current state before retrying",
                ));
            }
            state.update(&input, &current)
        })
        .await
    }
}

fn check_capacity(state: &Tasks) -> Result<()> {
    if state.entries.len() >= MAX_TASKS {
        return Err(PluginError::invalid_params(format!(
            "a conversation can store up to {MAX_TASKS} tasks"
        )));
    }
    Ok(())
}
