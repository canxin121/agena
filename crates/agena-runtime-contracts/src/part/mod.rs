//! The retained "everything is a part" model, now at contracts top level.
//!
//! `MessagePart` (the v1 transcript part) is not part of this surface; it
//! lives at [`crate::message::part`] until its removal.

mod attachment;
mod content;
mod hook;
mod interaction;
mod notice;
mod skill_reference;
mod tool;

pub use attachment::{AttachmentItem, AttachmentKind, AttachmentPart, AttachmentSource};
pub use content::{PartContent, RuntimeActivity};
pub use hook::HookPart;
pub use interaction::{InteractiveRequestPart, RequestPart};
pub use notice::NoticePart;
pub use skill_reference::{SkillReference, SkillReferencePart};
pub use tool::tool_output_content_blocks;
pub use tool::{
    ApplyPatchToolInput, AskUserToolInput, CronCreateToolInput, CronDeleteToolInput,
    CronHistoryToolInput, CronJobControlToolInput, CronListToolInput, CronMisfirePolicyInput,
    CronRetryPolicyInput, CronUpdateToolInput, EnterSnapshotToolInput, ExitSnapshotToolInput,
    GlobToolInput, GrepToolInput, InteractionNotifyToolInput, LspDefinitionToolInput,
    LspDiagnosticsToolInput, LspHoverToolInput, LspReferencesToolInput, ModelVisibleOutput,
    OperationCompletion, OperationPart, ReadToolInput, ScheduleWakeupToolInput, ShellCommandInput,
    ShellMonitorInput, ShellMonitorPatternKind, ShellToolInput, TaskAccess, TaskToolInput,
    ToolSearchToolInput, WebFetchToolInput, WebSearchToolInput,
};
