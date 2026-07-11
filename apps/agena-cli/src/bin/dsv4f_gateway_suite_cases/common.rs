use agena::permission::PermissionMode;
use serde_json::Value;

use super::{
    GatewayOutcome, Harness, PendingReply, SuiteReport, assert_contains, baseline_permission,
};

pub(crate) async fn run_single(
    harness: &Harness,
    report: &mut SuiteReport,
    label: &str,
    target: &str,
    canonical_target: &str,
    input: Value,
    expected_text: Option<&str>,
) -> anyhow::Result<Option<GatewayOutcome>> {
    if !harness.selector.enabled(label) {
        return Ok(None);
    }
    let session = harness
        .create_session(
            label,
            &[canonical_target],
            baseline_permission(PermissionMode::Allow),
        )
        .await?;
    let outcome = harness
        .run_gateway_target(session, label, target, input, PendingReply::None, true)
        .await?;
    if let Some(expected) = expected_text {
        assert_contains(&outcome, expected)?;
    }
    report.pass(label);
    Ok(Some(outcome))
}
