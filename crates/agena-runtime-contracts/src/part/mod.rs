//! The retained "everything is a part" model, at contracts top level.

mod attachment;
mod hook;
mod notice;
mod skill_reference;
mod tool;

pub use attachment::{AttachmentItem, AttachmentKind, AttachmentPart, AttachmentSource};
pub use hook::HookPart;
pub use notice::NoticePart;
pub use skill_reference::{SkillReference, SkillReferencePart};
pub use tool::{
    ApplyPatchToolInput, AskUserToolInput, BackgroundOperation, CronCreateToolInput,
    CronDeleteToolInput, CronHistoryToolInput, CronJobControlToolInput, CronListToolInput,
    CronMisfirePolicyInput, CronRetryPolicyInput, CronUpdateToolInput, EnterSnapshotToolInput,
    ExitSnapshotToolInput, GlobToolInput, GrepToolInput, InteractionNotifyToolInput,
    LspDefinitionToolInput, LspDiagnosticsToolInput, LspHoverToolInput, LspReferencesToolInput,
    MonitorToolInput, MonitorWsInput, OperationPart, ReadToolInput, ShellCommandInput,
    ShellMonitorInput, ShellMonitorPatternKind, ShellToolInput, TaskAccess, TaskToolInput,
    ToolSearchToolInput, WebFetchToolInput, WebSearchToolInput,
};
