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
    CronCreateToolInput, CronDeleteToolInput, CronListToolInput, EnterSnapshotToolInput, ErrorPart,
    ExecutionStatus, ExitSnapshotToolInput, FileChangeKind, FileChangeRecord, FilesystemAccess,
    FilesystemEffect, GlobToolInput, GrepToolInput, InteractionNotificationLevel,
    InteractionNotifyToolInput, InteractiveRequestPart, LspDefinitionToolInput,
    LspDiagnosticsToolInput, LspHoverToolInput, LspPositionToolInput, LspReferencesToolInput,
    MessagePart, ModelVisibleOutput, NetworkEffect, OperationBlock, OperationError, OperationPart,
    PartContent, PartKind, PartStateTransitionError, PendingInteractiveRequest,
    PendingInteractiveRequestKind, PluginInvocation, ProcessEvent, ProcessShell, ProcessStatus,
    ProcessStream, ProcessSummary, ReadMode, ReadToolInput, ReasoningPart, RequestPart,
    ScheduleWakeupToolInput, SearchResultItem, ShellCommandInput, ShellToolInput, StructuredField,
    StructuredObject, StructuredValue, TableColumn, TaskModelSelection, TaskToolInput, TextPart,
    TimeRange, TodoItem, TodoPriority, TodoStatus, ToolInvocation, ToolManagedOutput, ToolOutput,
    ToolResultDisplay, ToolResultEnvelope, ToolResultState, ToolSearchToolInput, UserInputOption,
    UserInputQuestion, UserInputReply, UserInputReplyKind, UserInputRequest, WebFetchToolInput,
    WebSearchResult, WebSearchToolInput, WorkflowPromptToolInput,
};
pub(crate) use part::{deserialize_user_input_answers, user_input_answers_is_empty};
pub use usage::MessageUsage;
