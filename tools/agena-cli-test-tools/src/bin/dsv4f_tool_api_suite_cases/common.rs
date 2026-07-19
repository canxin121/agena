use agena::permission::PermissionMode;
use serde_json::Value;

use super::{
    Harness, PendingReply, SuiteReport, ToolApiOutcome, assert_contains, baseline_permission,
};

pub(crate) async fn run_single(
    harness: &Harness,
    report: &mut SuiteReport,
    label: &str,
    tool_name: &str,
    execution_tool_key: &str,
    input: Value,
    expected_text: Option<&str>,
) -> anyhow::Result<Option<ToolApiOutcome>> {
    if !harness.selector.enabled(label) {
        return Ok(None);
    }
    let session = harness
        .create_session(
            label,
            &[execution_tool_key],
            baseline_permission(PermissionMode::Allow),
        )
        .await?;
    let outcome = harness
        .run_execution_tool(session, label, tool_name, input, PendingReply::None, true)
        .await?;
    if let Some(expected) = expected_text {
        assert_contains(&outcome, expected)?;
    }
    report.pass(label);
    Ok(Some(outcome))
}
