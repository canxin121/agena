use super::{
    Fixture, Harness, PendingReply, SuiteReport, TOOLS_CALL_HANDLER_KEY, TOOLS_HELP_HANDLER_KEY,
    TOOLS_LIST_HANDLER_KEY, TOOLS_SEARCH_HANDLER_KEY, TOOLS_TAGS_HANDLER_KEY, ToolApiOutcome,
    assert_contains, baseline_permission, commit_fixture_change, operations_since, payload_string,
    transcript_since,
};

mod builtin_io;
mod builtin_plugins;
mod builtin_runtime;
mod builtin_skills;
mod common;
mod integration;
mod meta;

pub(super) use self::builtin_io::{run_cron_cases, run_fs_cases, run_settings_cases};
pub(super) use self::builtin_plugins::{run_mcp_cases, run_memory_cases, run_schema_lab_cases};
pub(super) use self::builtin_runtime::{run_plan_cases, run_runtime_cases, run_shell_cases};
pub(super) use self::builtin_skills::{run_code_cases, run_lsp_cases, run_skills_cases};
pub(super) use self::common::run_single;
pub(super) use self::integration::{
    run_external_plugin_suite, run_nested_permission_suite, run_snapshot_cases, run_task_case,
    run_web_cases,
};
pub(super) use self::meta::run_tool_api_meta_suite;

pub(crate) async fn run_builtin_suite(
    harness: &Harness,
    fixture: &Fixture,
    report: &mut SuiteReport,
) -> anyhow::Result<()> {
    run_skills_cases(harness, report).await?;
    run_code_cases(harness, report).await?;
    run_lsp_cases(harness, report).await?;
    run_cron_cases(harness, report).await?;
    run_fs_cases(harness, fixture, report).await?;
    run_settings_cases(harness, report).await?;
    run_shell_cases(harness, report).await?;
    run_runtime_cases(harness, report).await?;
    run_plan_cases(harness, report).await?;
    run_memory_cases(harness, report).await?;
    run_schema_lab_cases(harness, report).await?;
    run_mcp_cases(harness, report).await?;
    run_snapshot_cases(harness, report).await?;
    run_task_case(harness, report).await?;
    run_web_cases(harness, fixture, report).await?;
    Ok(())
}
