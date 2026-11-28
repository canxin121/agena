pub use agena_domain::ProcessShell;
use serde::{Deserialize, Serialize};

use crate::PluginKey;

#[derive(Debug, Clone)]
pub struct ImportedSkillShellRequest {
    pub command_name: String,
    pub shell: Option<ProcessShell>,
    pub commands: Vec<String>,
    pub forked: bool,
    pub explicit_agent: bool,
    pub skill_dir: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct ImportedSkillShellResult {
    pub outputs: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum ImportedSkillRender {
    Prompt(String),
    ForkResult(String),
    Inspection,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportedSkillInvokedBy {
    User,
    Claude,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedSkillShellHookInput {
    pub session_id: i64,
    pub tool_use_id: String,
    pub command_name: String,
    pub command: String,
    pub shell: ProcessShell,
    pub invoked_by: ImportedSkillInvokedBy,
    pub source_plugin: PluginKey,
    pub forked: bool,
    pub subagent: bool,
    pub skill_dir: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", content = "reason", rename_all = "snake_case")]
pub enum ImportedSkillShellDecision {
    Skip(String),
    Stop(String),
}
