use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};

#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    JsonSchema,
    Display,
    EnumString,
    Default,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
/// Shell used to run a process.
pub enum ProcessShell {
    #[default]
    Bash,
    Powershell,
}
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Display, EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
/// Output stream of a process event.
pub enum ProcessStream {
    Stdout,
    Stderr,
}
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Display, EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
/// Status of a monitored process.
pub enum ProcessStatus {
    Running,
    Exited,
    TimedOut,
    Stopped,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
/// One line of process output.
pub struct ProcessEvent {
    pub seq: u64,
    pub stream: ProcessStream,
    pub ts_ms: i64,
    pub line: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Summary of a monitored process.
pub struct ProcessSummary {
    pub process_id: String,
    pub command: String,
    pub description: String,
    pub status: ProcessStatus,
    pub background: bool,
    /// True when the process was started with shell monitor conditions rather
    /// than as an unconstrained background process.
    #[serde(default)]
    pub monitored: bool,
    pub started_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at_ms: Option<i64>,
    pub buffered_lines: u32,
    pub last_seq: u64,
    pub dropped_lines: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_reason: Option<String>,
}
