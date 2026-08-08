//! Runtime-neutral message types shared across layers.

#![allow(clippy::module_inception)]

mod message;
mod metadata;
mod part;

pub use message::Message;
pub use metadata::{MessageMetadata, MessageProviderState};
pub use part::tool_output_content_blocks;
pub use part::{
    ApplyPatchToolInput, AskUserToolInput, AttachmentItem, AttachmentKind, AttachmentSource,
    CronCreateToolInput, CronDeleteToolInput, CronHistoryToolInput, CronJobControlToolInput,
    CronListToolInput, CronMisfirePolicyInput, CronRetryPolicyInput, CronUpdateToolInput,
    EnterSnapshotToolInput, ExitSnapshotToolInput, GlobToolInput, GrepToolInput, HookPart,
    InteractionNotifyToolInput, InteractiveRequestPart, LspDefinitionToolInput,
    LspDiagnosticsToolInput, LspHoverToolInput, LspReferencesToolInput, MessagePart,
    ModelVisibleOutput, NoticePart, OperationCompletion, OperationPart, PartContent, ReadToolInput,
    RequestPart, RuntimeActivity, ScheduleWakeupToolInput, ShellCommandInput, ShellMonitorInput,
    ShellMonitorPatternKind, ShellToolInput, SkillReference, SkillReferencePart, TaskAccess,
    TaskToolInput, ToolSearchToolInput, WebFetchToolInput, WebSearchToolInput,
};
