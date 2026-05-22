#![allow(clippy::module_inception)]

mod message;
mod metadata;
mod part;
mod usage;

pub use message::{Message, MessageStateTransitionError, MessageStatus};
pub use metadata::{MessageMetadata, MessageSource};
pub use part::{
    AgentRestoreToolInput, AgentSwitchToolInput, ApplyPatchToolInput, ArtifactRef,
    AskUserToolInput, AttachmentItem, AttachmentKind, AttachmentPart, AttachmentSource,
    BashToolInput, ClearGoalToolInput, CreateGoalToolInput, CronCreateToolInput,
    CronDeleteToolInput, CronListToolInput, EnterPlanModeToolInput, EnterWorktreeToolInput,
    ErrorPart, ExecutionStatus, ExitPlanModeToolInput, ExitWorktreeToolInput, FileChangeEntry,
    FileChangeKind, FilesystemAccess, FilesystemEffect, GetGoalToolInput, GlobToolInput,
    GrepToolInput, LspDefinitionToolInput, LspDiagnosticsToolInput, LspHoverToolInput,
    LspReferencesToolInput, MessagePart, ModelVisibleOutput, MonitorEvent, MonitorStatus,
    MonitorStream, MonitorSummary, MonitorToolInput, NotebookCellType, NotebookEditMode,
    NotebookEditToolInput, OperationBlock, OperationError, OperationPart, PartContent, PartKind,
    PartStateTransitionError, PermissionRequestPart, PluginInvocation, PowerShellToolInput,
    ReadMode, ReadToolInput, ReasoningPart, RequestPart, RequestUserInputToolInput,
    ScheduleWakeupToolInput, SearchResultItem, StructuredField, StructuredObject, StructuredValue,
    TableColumn, TaskSubagentType, TaskToolInput, TextPart, TimeRange, TodoItem, TodoPriority,
    TodoStatus, TodoWriteToolInput, ToolAttachment, ToolInvocation, ToolOutput,
    ToolSearchToolInput, UpdateGoalStatus, UpdateGoalToolInput, UserInputOption, UserInputQuestion,
    UserInputReply, UserInputReplyKind, UserInputRequest, UserInputRequestPart, WebFetchToolInput,
    WebSearchResult, WebSearchToolInput, WorkflowPromptToolInput,
};
pub(crate) use part::{deserialize_user_input_answers, user_input_answers_is_empty};
pub use usage::MessageUsage;
