//! Static bundled-plugin factories owned by the plugin crate.

use std::sync::Arc;

pub use agena_runtime_tools::tool::*;

use crate::plugins::provided::{
    agent, code, context, cron, environment, fs, image, interaction, lsp, mcp, notebook, planning,
    repo, report, schema_lab, session, settings, shell, skills, tasks, tool_api,
};

pub fn skills_plugin_id() -> &'static str {
    skills::SKILLS_PLUGIN_ID
}

pub fn new_skills_plugin() -> impl agena_plugin_host::sdk::Plugin {
    skills::SkillsPlugin::new()
}

pub fn lsp_plugin_id() -> &'static str {
    "agena.lsp"
}

pub fn new_lsp_plugin() -> impl agena_plugin_host::sdk::Plugin {
    lsp::LspPlugin::new()
}

pub fn cron_plugin_id() -> &'static str {
    cron::CRON_PLUGIN_ID
}

pub fn new_cron_plugin() -> impl agena_plugin_host::sdk::Plugin {
    cron::CronPlugin::new()
}

pub fn code_plugin_id() -> &'static str {
    code::CODE_PLUGIN_ID
}

pub fn new_code_plugin() -> impl agena_plugin_host::sdk::Plugin {
    code::new_plugin()
}

pub fn context_plugin_id() -> &'static str {
    context::CONTEXT_PLUGIN_ID
}

pub fn new_context_plugin() -> impl agena_plugin_host::sdk::Plugin {
    context::ContextPlugin::new()
}

pub fn environment_plugin_id() -> &'static str {
    environment::ENVIRONMENT_PLUGIN_ID
}

pub fn new_environment_plugin() -> impl agena_plugin_host::sdk::Plugin {
    environment::EnvironmentPlugin::new()
}

pub fn fs_plugin_id() -> &'static str {
    fs::FS_PLUGIN_ID
}

pub fn new_fs_plugin() -> impl agena_plugin_host::sdk::Plugin {
    fs::new_plugin()
}

pub fn openai_plugin_id() -> &'static str {
    image::OPENAI_PLUGIN_ID
}

pub fn new_openai_plugin() -> impl agena_plugin_host::sdk::Plugin {
    image::OpenAiToolsPlugin::new()
}

pub fn settings_plugin_id() -> &'static str {
    settings::SETTINGS_PLUGIN_ID
}

pub fn new_settings_plugin() -> impl agena_plugin_host::sdk::Plugin {
    settings::SettingsPlugin::new()
}

pub fn shell_plugin_id() -> &'static str {
    shell::SHELL_PLUGIN_ID
}

pub fn new_shell_plugin() -> impl agena_plugin_host::sdk::Plugin {
    shell::new_plugin()
}

pub fn tool_api_plugin_id() -> &'static str {
    tool_api::TOOL_API_PLUGIN_ID
}

pub fn new_tool_api_plugin() -> impl agena_plugin_host::sdk::Plugin {
    tool_api::ToolApiPlugin::new()
}

pub fn agent_plugin_id() -> &'static str {
    agent::AGENT_PLUGIN_ID
}

pub fn new_agent_plugin() -> impl agena_plugin_host::sdk::Plugin {
    agent::AgentPlugin::new()
}

pub fn session_plugin_id() -> &'static str {
    session::SESSION_PLUGIN_ID
}

pub fn new_session_plugin() -> impl agena_plugin_host::sdk::Plugin {
    session::SessionPlugin::new()
}

pub fn interaction_plugin_id() -> &'static str {
    interaction::INTERACTION_PLUGIN_ID
}

pub fn new_interaction_plugin() -> impl agena_plugin_host::sdk::Plugin {
    interaction::InteractionPlugin::new()
}

pub fn plan_plugin_id() -> &'static str {
    planning::PLAN_PLUGIN_ID
}

pub fn new_plan_plugin() -> impl agena_plugin_host::sdk::Plugin {
    planning::PlanPlugin::new()
}

pub fn tasks_plugin_id() -> &'static str {
    tasks::TASKS_PLUGIN_ID
}

pub fn new_tasks_plugin() -> impl agena_plugin_host::sdk::Plugin {
    tasks::TasksPlugin::new()
}

pub fn report_plugin_id() -> &'static str {
    report::REPORT_PLUGIN_ID
}

pub fn new_report_plugin() -> impl agena_plugin_host::sdk::Plugin {
    report::ReportPlugin::new()
}

pub fn notebook_plugin_id() -> &'static str {
    notebook::NOTEBOOK_PLUGIN_ID
}

pub fn new_notebook_plugin() -> impl agena_plugin_host::sdk::Plugin {
    notebook::NotebookPlugin::new()
}

pub fn snapshot_plugin_id() -> &'static str {
    repo::SNAPSHOT_PLUGIN_ID
}

pub fn new_snapshot_plugin() -> impl agena_plugin_host::sdk::Plugin {
    repo::SnapshotPlugin::new()
}

pub const fn schema_lab_builtin_enabled() -> bool {
    cfg!(feature = "schema-lab")
}

pub fn schema_lab_plugin_id() -> &'static str {
    schema_lab::SCHEMA_LAB_PLUGIN_ID
}

pub fn new_schema_lab_plugin() -> impl agena_plugin_host::sdk::Plugin {
    schema_lab::SchemaLabPlugin::new()
}

pub fn mcp_plugin_id() -> &'static str {
    crate::MCP_PLUGIN_ID
}

pub fn new_mcp_plugin(
    manager: Arc<agena_mcp_client::McpConnectionManager>,
) -> impl agena_plugin_host::sdk::Plugin {
    mcp::McpPlugin::new(manager)
}

pub fn web_plugin_id() -> &'static str {
    crate::web::WEB_PLUGIN_ID
}

pub fn new_web_plugin() -> impl agena_plugin_host::sdk::Plugin {
    crate::web::WebPlugin::new()
}

pub fn memory_plugin_id() -> &'static str {
    crate::memory::MEMORY_PLUGIN_ID
}

pub fn new_memory_plugin() -> impl agena_plugin_host::sdk::Plugin {
    crate::memory::MemoryPlugin::new()
}
