//! Static bundled-plugin factories owned by the plugin crate.

use std::sync::Arc;

pub use agena_runtime_tools::tool::*;

use crate::plugins::provided::{
    code, cron, fs, image, interaction, lsp, mcp, monitor, notebook, planning, repo, report,
    session, settings, shell, skills, tasks, terminal, tool_api,
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

pub fn monitor_plugin_id() -> &'static str {
    monitor::MONITOR_PLUGIN_ID
}

pub fn new_monitor_plugin() -> impl agena_plugin_host::sdk::Plugin {
    monitor::new_plugin()
}

pub fn tool_api_plugin_id() -> &'static str {
    tool_api::TOOL_API_PLUGIN_ID
}

pub fn new_tool_api_plugin() -> impl agena_plugin_host::sdk::Plugin {
    tool_api::ToolApiPlugin::new()
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

pub fn terminal_plugin_id() -> &'static str {
    terminal::TERMINAL_PLUGIN_ID
}

pub fn new_terminal_plugin() -> impl agena_plugin_host::sdk::Plugin {
    terminal::TerminalPlugin::new()
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

pub fn chatgpt_plugin_id() -> &'static str {
    crate::plugins::provided::chatgpt::CHATGPT_PLUGIN_ID
}

pub fn new_chatgpt_plugin() -> impl agena_plugin_host::sdk::Plugin {
    crate::plugins::provided::chatgpt::ChatGptToolsPlugin::new()
}

pub fn gemini_plugin_id() -> &'static str {
    crate::plugins::provided::gemini::GEMINI_PLUGIN_ID
}

pub fn new_gemini_plugin() -> impl agena_plugin_host::sdk::Plugin {
    crate::plugins::provided::gemini::GeminiToolsPlugin::new()
}

pub fn claude_plugin_id() -> &'static str {
    crate::plugins::provided::claude::CLAUDE_PLUGIN_ID
}

pub fn new_claude_plugin() -> impl agena_plugin_host::sdk::Plugin {
    crate::plugins::provided::claude::ClaudeToolsPlugin::new()
}
