#![allow(clippy::module_inception)]

mod message;
mod metadata;
mod part;
mod usage;

pub use message::{Message, MessageStateTransitionError, MessageStatus};
pub use metadata::{AssistantReasoningField, MessageMetadata, MessageProviderState, MessageSource};
pub use part::{
    AgentRestoreToolInput, AgentSwitchToolInput, ApplyPatchToolInput, ArtifactRef,
    AskUserToolInput, AttachmentItem, AttachmentKind, AttachmentPart, AttachmentSource,
    CronCreateToolInput, CronDeleteToolInput, CronListToolInput, EnterWorktreeToolInput, ErrorPart,
    ExecutionStatus, ExitWorktreeToolInput, FileChangeKind, FileChangeRecord, FilesystemAccess,
    FilesystemEffect, GlobToolInput, GrepToolInput, InteractiveRequestPart, LspDefinitionToolInput,
    LspDiagnosticsToolInput, LspHoverToolInput, LspPositionToolInput, LspReferencesToolInput,
    MessagePart, ModelVisibleOutput, MonitorEvent, MonitorStatus, MonitorStream, MonitorSummary,
    MonitorToolInput, NetworkEffect, OperationBlock, OperationError, OperationPart, PartContent,
    PartKind, PartStateTransitionError, PendingInteractiveRequest, PendingInteractiveRequestKind,
    PluginInvocation, ReadMode, ReadToolInput, ReasoningPart, RequestPart, ScheduleWakeupToolInput,
    SearchResultItem, ShellCommandInput, StructuredField, StructuredObject, StructuredValue,
    TableColumn, TaskSubagentType, TaskToolInput, TextPart, TimeRange, TodoItem, TodoPriority,
    TodoStatus, TodoWriteToolInput, ToolInvocation, ToolOutput, ToolSearchToolInput,
    UserInputOption, UserInputQuestion, UserInputReply, UserInputReplyKind, UserInputRequest,
    WebFetchToolInput, WebSearchResult, WebSearchToolInput, WorkflowPromptToolInput,
};
pub(crate) use part::{deserialize_user_input_answers, user_input_answers_is_empty};
pub use usage::MessageUsage;
