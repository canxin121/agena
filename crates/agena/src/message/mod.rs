#![allow(clippy::module_inception)]

mod message;
mod metadata;
mod part;
mod usage;

pub use message::{Message, MessageStateTransitionError, MessageStatus};
pub use metadata::{MessageMetadata, MessageSource};
pub use part::{
    ApplyPatchToolInput, AskUserToolInput, AttachmentItem, AttachmentKind, AttachmentPart,
    AttachmentSource, BashToolInput, ClearGoalToolInput, CommandExecutionPart, CreateGoalToolInput,
    CronCreateToolInput, CronDeleteToolInput, CronListToolInput, EnterPlanModeToolInput,
    EnterWorktreeToolInput, ErrorPart, ExecutionStatus, ExitPlanModeToolInput,
    ExitWorktreeToolInput, FileChangeEntry, FileChangeKind, FileChangePart, FilesystemAccess,
    FilesystemEffect, GetGoalToolInput, GlobToolInput, GrepToolInput, LspDefinitionToolInput,
    LspDiagnosticsToolInput, LspHoverToolInput, LspReferencesToolInput, MessagePart, MonitorEvent,
    MonitorStatus, MonitorStream, MonitorSummary, MonitorToolInput, NotebookCellType,
    NotebookEditMode, NotebookEditToolInput, PartContent, PartKind, PartStateTransitionError,
    PermissionRequestPart, PluginInvocation, PowerShellToolInput, ReadToolInput, ReasoningPart,
    RequestUserInputToolInput, ScheduleWakeupToolInput, StructuredField, StructuredObject,
    StructuredValue, TaskSubagentType, TaskToolInput, TextPart, TimeRange, TodoItem, TodoListPart,
    TodoPriority, TodoStatus, TodoWriteToolInput, ToolAttachment, ToolExecutionPart,
    ToolInvocation, ToolOutput, ToolResultBlock, ToolSearchToolInput, UpdateGoalStatus,
    UpdateGoalToolInput, UserInputOption, UserInputQuestion, UserInputReply, UserInputReplyKind,
    UserInputRequest, UserInputRequestPart, ViewFileToolInput, WebFetchToolInput, WebSearchPart,
    WebSearchResult, WebSearchToolInput, WorkflowPromptToolInput,
};
pub(crate) use part::{deserialize_user_input_answers, user_input_answers_is_empty};
pub use usage::MessageUsage;
