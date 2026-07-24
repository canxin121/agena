#![allow(clippy::module_inception)]

mod message;
mod metadata;
mod part;

pub use message::Message;
pub use metadata::{MessageMetadata, MessageProviderState};
pub use part::tool_output_content_blocks;
pub use part::{
    AgentSwitchToolInput, ApplyPatchToolInput, AskUserToolInput, AttachmentItem, AttachmentKind,
    AttachmentSource, CronCreateToolInput, CronDeleteToolInput, CronListToolInput,
    EnterSnapshotToolInput, ExitSnapshotToolInput, GlobToolInput, GrepToolInput,
    InteractionNotifyToolInput, InteractiveRequestPart, LspDefinitionToolInput,
    LspDiagnosticsToolInput, LspHoverToolInput, LspReferencesToolInput, MessagePart,
    ModelVisibleOutput, OperationBlock, OperationPart, PartContent, ReadToolInput, RequestPart,
    ScheduleWakeupToolInput, ShellCommandInput, ShellToolInput, TaskToolInput, ToolSearchToolInput,
    WebFetchToolInput, WebSearchToolInput,
};
