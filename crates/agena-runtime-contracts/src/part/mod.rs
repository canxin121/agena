//! The retained "everything is a part" model, at contracts top level.

mod attachment;
mod hook;
mod interaction;
mod notice;
mod skill_reference;
mod tool;

pub use attachment::{AttachmentItem, AttachmentKind, AttachmentPart, AttachmentSource};
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
    MonitorToolInput, MonitorWsInput,
    OperationCompletion, OperationPart, BackgroundOperation, ReadToolInput,
    ShellCommandInput,
    ShellMonitorInput, ShellMonitorPatternKind, ShellToolInput, TaskAccess, TaskToolInput,
    ToolSearchToolInput, WebFetchToolInput, WebSearchToolInput,
};
