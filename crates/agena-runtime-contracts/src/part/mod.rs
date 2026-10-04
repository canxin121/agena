//! The retained "everything is a part" model, at contracts top level.

mod attachment;
mod command_reference;
mod hook;
mod notice;
mod tool;

pub use attachment::{AttachmentItem, AttachmentKind, AttachmentPart, AttachmentSource};
pub use command_reference::{CommandReference, CommandReferencePart};
pub use hook::HookPart;
pub use notice::NoticePart;
pub use tool::{
    ApplyPatchToolInput, AskUserToolInput, BackgroundOperation, CronCreateToolInput,
    CronDeleteToolInput, CronHistoryToolInput, CronJobControlToolInput, CronListToolInput,
    CronMisfirePolicyInput, CronRetryPolicyInput, CronUpdateToolInput, EnterSnapshotToolInput,
    ExitSnapshotToolInput, GlobKind, GlobToolInput, GrepCase, GrepMode, GrepToolInput,
    InteractionNotifyToolInput, LspDefinitionToolInput, LspDiagnosticsToolInput, LspHoverToolInput,
    LspReferencesToolInput, MonitorToolInput, MonitorWsInput, OperationPart, ReadToolInput,
    ShellCommandInput, ShellMonitorInput, ShellMonitorPatternKind, ShellSignal, ShellToolInput,
    ShellWriteInput, TaskToolInput, ToolSearchToolInput, WebFetchToolInput, WebSearchToolInput,
};
