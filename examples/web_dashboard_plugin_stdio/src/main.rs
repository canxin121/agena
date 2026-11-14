//! An independently packaged Web page backed by a persistent stdio plugin.
//!
//! The browser uses two declared Method commands. This plugin owns its data;
//! the host owns authentication, transport, storage and the page lifecycle.

use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use agena_plugin_sdk::host_api::{
    HostClient, HostStorageGetRequest, HostStorageScope, HostStorageSetRequest,
    HostStorageVisibility,
};
use agena_plugin_sdk::{
    CommandDefinition, CommandDocs, CommandInvokeInput, CommandResult, CommandTarget, InitContext,
    InitOutcome, Plugin, PluginError, PluginManifest, PluginWebAssets, PluginWebContributions,
    PluginWebPage, PluginWebPlacement, PluginWebScope, Result, SettingsContract, TransportKind,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::Mutex;

const STORAGE_NAMESPACE: &str = "board";
const STORAGE_KEY: &str = "state";
const MAX_TASKS: usize = 100;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Status {
    Planned,
    InProgress,
    Done,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Priority {
    Low,
    Normal,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Task {
    id: String,
    title: String,
    description: String,
    status: Status,
    priority: Priority,
    updated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Board {
    schema_version: u32,
    revision: u64,
    next_id: u64,
    tasks: Vec<Task>,
}

impl Default for Board {
    fn default() -> Self {
        let timestamp = now_ms();
        Self {
            schema_version: 1,
            revision: 1,
            next_id: 4,
            tasks: vec![
                Task {
                    id: "task-1".into(),
                    title: "Explore your plugin workspace".into(),
                    description: "This example page is loaded from the plugin package. Edit or remove these sample items to make the board your own.".into(),
                    status: Status::Planned,
                    priority: Priority::Normal,
                    updated_at_ms: timestamp,
                },
                Task {
                    id: "task-2".into(),
                    title: "Try an independent backend".into(),
                    description: "Change a status or add a task. Each change is saved by the stdio plugin in its private host storage.".into(),
                    status: Status::InProgress,
                    priority: Priority::High,
                    updated_at_ms: timestamp,
                },
                Task {
                    id: "task-3".into(),
                    title: "Open a real plugin page".into(),
                    description: "The page has its own JavaScript, CSS and backend commands. Reload the browser to see your saved changes.".into(),
                    status: Status::Done,
                    priority: Priority::Low,
                    updated_at_ms: timestamp,
                },
            ],
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateInput {
    revision: u64,
    operation: Operation,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Operation {
    Save {
        #[serde(default)]
        id: Option<String>,
        title: String,
        #[serde(default)]
        description: String,
        status: Status,
        priority: Priority,
    },
    Delete {
        id: String,
    },
}

impl Board {
    fn validate(&self) -> Result<()> {
        if self.schema_version != 1
            || self.revision == 0
            || self.next_id == 0
            || self.tasks.len() > MAX_TASKS
        {
            return Err(PluginError::internal(
                "The saved board has an unsupported version or invalid size.",
            ));
        }
        let mut ids = std::collections::BTreeSet::new();
        for task in &self.tasks {
            let number = task
                .id
                .strip_prefix("task-")
                .and_then(|id| id.parse::<u64>().ok());
            if !ids.insert(&task.id)
                || number.is_none_or(|number| number == 0 || number >= self.next_id)
            {
                return Err(PluginError::internal(
                    "The saved board has invalid task identifiers.",
                ));
            }
            validate_text(&task.title, &task.description)?;
        }
        Ok(())
    }

    fn apply(&self, operation: Operation) -> Result<Self> {
        let mut next = self.clone();
        match operation {
            Operation::Save {
                id,
                title,
                description,
                status,
                priority,
            } => {
                let title = title.trim().to_owned();
                let description = description.trim().to_owned();
                validate_text(&title, &description)?;
                let timestamp = now_ms();
                if let Some(id) = id {
                    let task = next
                        .tasks
                        .iter_mut()
                        .find(|task| task.id == id)
                        .ok_or_else(|| {
                            PluginError::invalid_params(
                                "This task no longer exists. Refresh the board.",
                            )
                        })?;
                    task.title = title;
                    task.description = description;
                    task.status = status;
                    task.priority = priority;
                    task.updated_at_ms = timestamp;
                } else {
                    if next.tasks.len() >= MAX_TASKS {
                        return Err(PluginError::invalid_params(
                            "This example board supports up to 100 tasks.",
                        ));
                    }
                    let id = format!("task-{}", next.next_id);
                    next.next_id = next
                        .next_id
                        .checked_add(1)
                        .ok_or_else(|| PluginError::internal("Task identifier limit reached."))?;
                    next.tasks.push(Task {
                        id,
                        title,
                        description,
                        status,
                        priority,
                        updated_at_ms: timestamp,
                    });
                }
            }
            Operation::Delete { id } => {
                let index = next
                    .tasks
                    .iter()
                    .position(|task| task.id == id)
                    .ok_or_else(|| {
                        PluginError::invalid_params(
                            "This task no longer exists. Refresh the board.",
                        )
                    })?;
                next.tasks.remove(index);
            }
        }
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or_else(|| PluginError::internal("Board revision limit reached."))?;
        Ok(next)
    }
}

fn validate_text(title: &str, description: &str) -> Result<()> {
    if title.trim().is_empty() || title.chars().count() > 160 {
        return Err(PluginError::invalid_params(
            "A task title must contain 1–160 characters.",
        ));
    }
    if description.chars().count() > 2000 {
        return Err(PluginError::invalid_params(
            "A task description must contain at most 2,000 characters.",
        ));
    }
    if title.contains('\0') || description.contains('\0') {
        return Err(PluginError::invalid_params(
            "Task text cannot contain a null character.",
        ));
    }
    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[derive(Default)]
struct DashboardPlugin {
    host: OnceLock<Arc<dyn HostClient>>,
    transaction: Mutex<()>,
}

impl DashboardPlugin {
    fn host(&self) -> Result<&Arc<dyn HostClient>> {
        self.host
            .get()
            .ok_or_else(|| PluginError::internal("The board has not been initialized."))
    }

    async fn read_board(&self) -> Result<Board> {
        let stored = self
            .host()?
            .storage_get(HostStorageGetRequest {
                scope: HostStorageScope::Global,
                visibility: HostStorageVisibility::Private,
                namespace: STORAGE_NAMESPACE.into(),
                key: STORAGE_KEY.into(),
            })
            .await?;
        if let Some(value) = stored.value {
            let board: Board = serde_json::from_str(&value).map_err(|error| {
                PluginError::internal(format!("The saved board could not be read: {error}"))
            })?;
            board.validate()?;
            Ok(board)
        } else {
            let board = Board::default();
            self.write_board(&board).await?;
            Ok(board)
        }
    }

    async fn write_board(&self, board: &Board) -> Result<()> {
        self.host()?
            .storage_set(HostStorageSetRequest {
                scope: HostStorageScope::Global,
                visibility: HostStorageVisibility::Private,
                namespace: STORAGE_NAMESPACE.into(),
                key: STORAGE_KEY.into(),
                value: serde_json::to_string(board)
                    .map_err(|error| PluginError::internal_error(&error))?,
            })
            .await
    }
}

fn command(id: &str, title: &str, input: SettingsContract) -> CommandDefinition {
    CommandDefinition {
        id: id.into(),
        title: title.into(),
        group: "Workspace board".into(),
        category: Some("web".into()),
        slash: None,
        aliases: Vec::new(),
        applies_to: Default::default(),
        docs: CommandDocs {
            summary: Some(title.into()),
            ..Default::default()
        },
        input,
        target: CommandTarget::Method { handler: id.into() },
    }
}

#[async_trait]
impl Plugin for DashboardPlugin {
    fn manifest(&self) -> PluginManifest {
        let mut manifest = PluginManifest::new("demo", "web_dashboard", env!("CARGO_PKG_VERSION"));
        manifest.summary =
            Some("An independent Web workspace with a persistent task board.".into());
        manifest.help = Some("Open the Workspace board page to create, edit and complete tasks. The three initial items are sample content; changes persist in this plugin's private storage.".into());
        manifest.transports = vec![TransportKind::Stdio];
        manifest.commands = vec![
            command(
                "list",
                "Read the board",
                SettingsContract::empty_object(
                    "Read board",
                    "Return the saved tasks and board revision.",
                ),
            ),
            command(
                "update",
                "Update the board",
                SettingsContract::bounded_json(
                    "Update board",
                    "Save or delete one task at the expected board revision.",
                    16_384,
                    6,
                ),
            ),
        ];
        manifest.surface.web = Some(PluginWebContributions {
            api_version: 1,
            assets: PluginWebAssets {
                root: "web/dist".into(),
            },
            pages: vec![PluginWebPage {
                id: "dashboard".into(),
                title: "Workspace board".into(),
                entry: "dashboard.js".into(),
                styles: vec!["dashboard.css".into()],
                placement: PluginWebPlacement::Workspace,
                scope: PluginWebScope::Instance,
                commands: vec!["list".into(), "update".into()],
                settings: false,
            }],
        });
        manifest
    }

    async fn init(&self, _ctx: InitContext, host: Arc<dyn HostClient>) -> Result<InitOutcome> {
        self.host
            .set(host)
            .map_err(|_| PluginError::internal("The board was already initialized."))?;
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn command_invoke(&self, input: CommandInvokeInput) -> Result<CommandResult> {
        let _transaction = self.transaction.lock().await;
        match input.command_id.as_str() {
            "list" => {
                let board = self.read_board().await?;
                let mut result = CommandResult::succeeded("Board loaded.");
                result.output = Some(json!(board));
                Ok(result)
            }
            "update" => {
                let request: UpdateInput = serde_json::from_value(input.input)
                    .map_err(|error| PluginError::invalid_params_error(&error))?;
                let board = self.read_board().await?;
                if request.revision != board.revision {
                    let mut result = CommandResult::failed(
                        "This board changed in another page. Review the latest board and save again.",
                    );
                    result.output = Some(json!({ "conflict": true, "board": board }));
                    return Ok(result);
                }
                let next = board.apply(request.operation)?;
                self.write_board(&next).await?;
                let mut result = CommandResult::succeeded("Board saved.");
                result.output = Some(json!(next));
                Ok(result)
            }
            other => Err(PluginError::not_implemented(format!("command {other}"))),
        }
    }
}

agena_plugin_sdk::export_stdio!(DashboardPlugin::default());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_preserve_identity_and_advance_revision() {
        let board = Board::default();
        let next = board
            .apply(Operation::Save {
                id: Some("task-1".into()),
                title: "  A real task  ".into(),
                description: "Notes".into(),
                status: Status::Done,
                priority: Priority::High,
            })
            .unwrap();
        assert_eq!(next.tasks[0].id, "task-1");
        assert_eq!(next.tasks[0].title, "A real task");
        assert_eq!(next.tasks[0].status, Status::Done);
        assert_eq!(next.revision, board.revision + 1);
        assert_eq!(board.tasks[0].status, Status::Planned);
    }

    #[test]
    fn invalid_updates_leave_the_saved_board_unchanged() {
        let board = Board::default();
        let before = board.clone();
        assert!(
            board
                .apply(Operation::Save {
                    id: Some("task-1".into()),
                    title: " ".into(),
                    description: String::new(),
                    status: Status::Done,
                    priority: Priority::Normal,
                })
                .is_err()
        );
        assert!(
            board
                .apply(Operation::Delete {
                    id: "missing".into()
                })
                .is_err()
        );
        assert_eq!(board, before);
    }

    #[test]
    fn adding_after_deleting_never_reuses_an_identifier() {
        let board = Board::default()
            .apply(Operation::Delete {
                id: "task-3".into(),
            })
            .unwrap();
        let next = board
            .apply(Operation::Save {
                id: None,
                title: "New task".into(),
                description: String::new(),
                status: Status::Planned,
                priority: Priority::Normal,
            })
            .unwrap();
        assert_eq!(next.tasks.last().unwrap().id, "task-4");
        next.validate().unwrap();
    }

    #[test]
    fn persisted_data_is_checked_before_becoming_page_state() {
        let mut board = Board::default();
        board.tasks[1].id = board.tasks[0].id.clone();
        assert!(board.validate().is_err());
        let board = Board {
            schema_version: 2,
            ..Board::default()
        };
        assert!(board.validate().is_err());
        let board = Board {
            next_id: 2,
            ..Board::default()
        };
        assert!(board.validate().is_err());
    }
}
