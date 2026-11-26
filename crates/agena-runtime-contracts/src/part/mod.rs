//! The retained "everything is a part" model, at contracts top level.

mod attachment;
mod hook;
mod notice;
mod tool;

pub use attachment::{AttachmentItem, AttachmentKind, AttachmentPart, AttachmentSource};
pub use hook::HookPart;
pub use notice::NoticePart;
pub use tool::{
    ApplyPatchToolInput, AskUserToolInput, BackgroundOperation, CronCreateToolInput,
    CronDeleteToolInput, CronHistoryToolInput, CronJobControlToolInput, CronListToolInput,
    CronMisfirePolicyInput, CronRetryPolicyInput, CronUpdateToolInput, GlobKind, GlobToolInput,
    GrepCase, GrepMode, GrepToolInput, InteractionNotifyToolInput, LspDefinitionToolInput,
    LspDiagnosticsManyToolInput, LspDiagnosticsToolInput, LspHoverToolInput, LspPositionToolInput,
    LspReferencesToolInput, LspStartToolInput, LspStartToolOutput, LspStopToolInput,
    LspStopToolOutput, LspSymbolsToolInput, OperationPart, ReadMediaInput, ReadToolInput,
    ShellCommandInput, ShellLaunchInput, ShellMonitorInput, ShellMonitorPatternKind,
    ShellOpenInput, ShellReadInput, ShellSignal, ShellToolInput, ShellWatchInput,
    ShellWatchNotifications, ShellWatchPolicy, ShellWriteInput, ToolSearchToolInput,
    WebFetchToolInput, WebSearchToolInput,
};
