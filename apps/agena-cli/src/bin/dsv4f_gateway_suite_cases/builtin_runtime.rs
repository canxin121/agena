use agena::permission::PermissionMode;
use anyhow::ensure;
use serde_json::{Value, json};

use super::{
    Harness, PendingReply, SuiteReport, assert_contains, baseline_permission, payload_string,
};

pub(crate) async fn run_process_cases(
    harness: &Harness,
    report: &mut SuiteReport,
) -> anyhow::Result<()> {
    if !harness.selector.any_in_group("process") {
        return Ok(());
    }
    let session = harness
        .create_session(
            "dsv4f process chain",
            &[
                "agena.process.run",
                "agena.process.list",
                "agena.process.logs",
                "agena.process.stop",
            ],
            baseline_permission(PermissionMode::Allow),
        )
        .await?;
    let foreground = harness
        .run_gateway_target(
            session,
            "process.run.foreground",
            "process.run",
            json!({
                "shell": "bash",
                "command": "printf PROCESS_FG_OK",
                "description": "dsv4f foreground process",
                "timeout_ms": 5000,
                "filesystem_effects": [],
                "network_effects": [],
                "background": false
            }),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&foreground, "PROCESS_FG_OK")?;
    report.pass("process.run");
    let background = harness
        .run_gateway_target(
            session,
            "process.run.background",
            "process.run",
            json!({
                "shell": "bash",
                "command": "printf 'PROCESS_BG_OK\\n'; sleep 300",
                "description": "dsv4f background process",
                "timeout_ms": 310000,
                "filesystem_effects": [],
                "network_effects": [],
                "background": true
            }),
            PendingReply::None,
            true,
        )
        .await?;
    let process_id = payload_string(&background.payload(), "process_id")?;
    let list = harness
        .run_gateway_target(
            session,
            "process.list",
            "process.list",
            json!({}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&list, &process_id)?;
    report.pass("process.list");
    let logs = harness
        .run_gateway_target(
            session,
            "process.logs",
            "process.logs",
            json!({"process_id": process_id, "since_seq": 0, "limit": 20, "wait_ms": 1500}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&logs, "PROCESS_BG_OK")?;
    report.pass("process.logs");
    let stop = harness
        .run_gateway_target(
            session,
            "process.stop",
            "process.stop",
            json!({"process_id": payload_string(&background.payload(), "process_id")?}),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        stop.payload().get("status").and_then(Value::as_str) == Some("stopped"),
        "process.stop did not terminate the still-running fixture process: {}",
        stop.visible_text()
    );
    report.pass("process.stop");
    Ok(())
}

pub(crate) async fn run_runtime_cases(
    harness: &Harness,
    report: &mut SuiteReport,
) -> anyhow::Result<()> {
    if !harness.selector.any_in_group("runtime") {
        return Ok(());
    }
    let session = harness
        .create_session(
            "dsv4f runtime chain",
            &[
                "agena.runtime.get",
                "agena.runtime.rename",
                "agena.runtime.switch",
                "agena.runtime.restore",
                "agena.runtime.request_input",
            ],
            baseline_permission(PermissionMode::Allow),
        )
        .await?;
    let current = harness
        .run_gateway_target(
            session,
            "runtime.get",
            "runtime.get",
            json!({}),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        current.payload().get("session").is_some(),
        "runtime.get lacks session payload"
    );
    report.pass("runtime.get");
    let renamed = harness
        .run_gateway_target(
            session,
            "runtime.rename",
            "runtime.rename",
            json!({"title": "dsv4f-runtime-renamed"}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&renamed, "dsv4f-runtime-renamed")?;
    report.pass("runtime.rename");
    let switched = harness
        .run_gateway_target(
            session,
            "runtime.switch",
            "runtime.switch",
            json!({"agent": "verify", "push_previous": true}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&switched, "verify")?;
    report.pass("runtime.switch");
    let restored = harness
        .run_gateway_target(
            session,
            "runtime.restore",
            "runtime.restore",
            json!({}),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        restored.payload().get("restored").and_then(Value::as_bool) == Some(true),
        "runtime.restore did not report restored=true"
    );
    report.pass("runtime.restore");
    let requested = harness
        .run_gateway_target(
            session,
            "runtime.request_input",
            "runtime.request_input",
            json!({
                "title": "Gateway input probe",
                "body_markdown": "Return TEST_OK.",
                "kind": "text",
                "submit_label": "Submit",
                "cancel_label": "Cancel",
                "questions": [{
                    "id": "answer",
                    "header": "Answer",
                    "question": "Reply TEST_OK",
                    "allow_custom": true
                }]
            }),
            PendingReply::Input,
            true,
        )
        .await?;
    assert_contains(&requested, "TEST_OK")?;
    report.pass("runtime.request_input");
    Ok(())
}

pub(crate) async fn run_plan_cases(
    harness: &Harness,
    report: &mut SuiteReport,
) -> anyhow::Result<()> {
    if !harness.selector.any_in_group("plan") {
        return Ok(());
    }
    let session = harness
        .create_session(
            "dsv4f plan chain",
            &[
                "agena.plan.set",
                "agena.plan.get",
                "agena.plan.update",
                "agena.plan.clear",
            ],
            baseline_permission(PermissionMode::Allow),
        )
        .await?;
    let set = harness
        .run_gateway_target(
            session,
            "plan.set",
            "plan.set",
            json!({
                "objective": "Exercise dsv4f plan gateway",
                "title": "DSV4F plan",
                "steps": [{
                    "id": "probe-step",
                    "title": "Probe step",
                    "checks": [{"id": "probe-check", "text": "Check gateway"}]
                }],
                "autorun": false
            }),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&set, "DSV4F plan")?;
    report.pass("plan.set");
    let get = harness
        .run_gateway_target(
            session,
            "plan.get",
            "plan.get",
            json!({"view": "full"}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&get, "probe-step")?;
    report.pass("plan.get");
    let update = harness
        .run_gateway_target(
            session,
            "plan.update",
            "plan.update",
            json!({"step_id": "probe-step", "check_id": "probe-check", "status": "completed"}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&update, "completed")?;
    report.pass("plan.update");
    let clear = harness
        .run_gateway_target(
            session,
            "plan.clear",
            "plan.clear",
            json!({}),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        clear.payload().get("cleared").and_then(Value::as_bool) == Some(true),
        "plan.clear did not report cleared=true"
    );
    report.pass("plan.clear");
    Ok(())
}
