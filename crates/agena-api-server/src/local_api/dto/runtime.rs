use super::*;

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeOperatorResource {
    pub mcp: RuntimeMcpResource,
    pub lsp: RuntimeLspResource,
    pub agents: RuntimeAgentsResource,
    pub skills: RuntimeSkillsResource,
    pub ui: agena::plugin::PluginUiCatalog,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeMcpResource {
    pub server_count: usize,
    pub tool_count: usize,
    pub servers: Vec<RuntimeMcpServerResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeMcpServerResource {
    pub name: String,
    pub tool_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeLspResource {
    pub server_count: usize,
    pub diagnostics_count: usize,
    pub files_with_diagnostics: usize,
    pub servers: Vec<RuntimeLspServerResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeLspServerResource {
    pub name: String,
    pub command: String,
    pub file_extensions: Vec<String>,
    pub root_markers: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeSkillsResource {
    pub skill_count: usize,
    pub command_count: usize,
    pub skills: Vec<RuntimeSkillResource>,
    pub commands: Vec<RuntimeSkillResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeSkillResource {
    pub name: String,
    pub description: String,
    pub aliases: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeAgentsResource {
    pub default_agent: String,
    pub total_count: usize,
    pub primary_count: usize,
    pub subagent_count: usize,
    pub hidden_count: usize,
    pub agents: Vec<RuntimeAgentResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeAgentResource {
    pub name: String,
    pub description: String,
    pub mode: AgentMode,
    pub hidden: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steps: Option<usize>,
    pub allowed_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "AgentPermissionConfig::is_empty")]
    pub permission: AgentPermissionConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub aliases: Vec<String>,
    pub scope: AgentScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeReloadResponse {
    pub cause: &'static str,
    pub previous_generation: u64,
    pub generation: u64,
    pub loaded_at: DateTime<Utc>,
}
