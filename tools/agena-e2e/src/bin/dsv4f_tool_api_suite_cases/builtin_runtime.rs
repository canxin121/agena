use agena_domain::PermissionMode;
use anyhow::ensure;
use serde_json::{Value, json};

use super::{
    Harness, PendingReply, SuiteReport, assert_contains, baseline_permission, payload_string,
};

pub(crate) async fn run_shell_cases(
    harness: &Harness,
    report: &mut SuiteReport,
) -> anyhow::Result<()> {
    if !harness.selector.any_in_group("shell") {
        return Ok(());
    }
    let session = harness
        .create_session(
            "dsv4f shell chain",
            &[
                "agena.shell.run",
                "agena.shell.list",
                "agena.shell.logs",
                "agena.shell.stop",
            ],
            baseline_permission(PermissionMode::Allow),
        )
        .await?;
    let foreground = harness
        .run_execution_tool(
            session,
            "shell.run.foreground",
            "shell.run",
            json!({
                "shell": "bash",
                "command": "printf PROCESS_FG_OK",
                "description": "dsv4f foreground process",
                "timeout_ms": 5000,
                "filesystem_effects": {},
                "network_effects": [],
                "background": false
            }),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&foreground, "PROCESS_FG_OK")?;
    report.pass("shell.run");
    let background = harness
        .run_execution_tool(
            session,
            "shell.run.background",
            "shell.run",
            json!({
                "shell": "bash",
                "command": "printf 'PROCESS_BG_OK\\n'; sleep 300",
                "description": "dsv4f background process",
                "timeout_ms": 310000,
                "filesystem_effects": {},
                "network_effects": [],
                "background": true
            }),
            PendingReply::None,
            true,
        )
        .await?;
    let process_id = payload_string(&background.payload(), "process_id")?;
    let list = harness
        .run_execution_tool(
            session,
            "shell.list",
            "shell.list",
            json!({}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&list, &process_id)?;
    report.pass("shell.list");
    let logs = harness
        .run_execution_tool(
            session,
            "shell.logs",
            "shell.logs",
            json!({"process_id": process_id, "since_seq": 0, "limit": 20, "wait_ms": 1500}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&logs, "PROCESS_BG_OK")?;
    report.pass("shell.logs");
    let stop = harness
        .run_execution_tool(
            session,
            "shell.stop",
            "shell.stop",
            json!({"process_id": payload_string(&background.payload(), "process_id")?}),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        stop.payload().get("status").and_then(Value::as_str) == Some("stopped"),
        "shell.stop did not terminate the still-running fixture process: {}",
        stop.visible_text()
    );
    report.pass("shell.stop");
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
                "agena.session.get",
                "agena.session.rename",
                "agena.interaction.ask",
                "agena.interaction.notify",
            ],
            baseline_permission(PermissionMode::Allow),
        )
        .await?;
    let current = harness
        .run_execution_tool(
            session,
            "session.get",
            "session.get",
            json!({}),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        current.payload().get("session").is_some(),
        "session.get lacks session payload"
    );
    report.pass("session.get");
    let renamed = harness
        .run_execution_tool(
            session,
            "session.rename",
            "session.rename",
            json!({"title": "dsv4f-runtime-renamed"}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&renamed, "dsv4f-runtime-renamed")?;
    report.pass("session.rename");
    let requested = harness
        .run_execution_tool(
            session,
            "interaction.ask",
            "interaction.ask",
            json!({
                "title": "Tool API input probe",
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
    report.pass("interaction.ask");
    let notified = harness
        .run_execution_tool(
            session,
            "interaction.notify",
            "interaction.notify",
            json!({
                "title": "Tool API notification probe",
                "body_markdown": "**NOTIFY_OK**",
                "level": "success"
            }),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&notified, "NOTIFY_OK")?;
    ensure!(
        notified.payload().get("level").and_then(Value::as_str) == Some("success"),
        "interaction.notify lacks success level payload"
    );
    report.pass("interaction.notify");
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
        .run_execution_tool(
            session,
            "plan.set",
            "plan.set",
            json!({
                "objective": "Exercise dsv4f Tool API plan",
                "title": "DSV4F plan",
                "steps": [{
                    "title": "Probe step",
                    "checks": [{"text": "Check Tool API"}]
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
        .run_execution_tool(
            session,
            "plan.get",
            "plan.get",
            json!({"view": "full"}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&get, "Probe step")?;
    report.pass("plan.get");
    let update = harness
        .run_execution_tool(
            session,
            "plan.update",
            "plan.update",
            json!({"step": 1, "check": 1, "status": "completed"}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&update, "completed")?;
    report.pass("plan.update");
    let clear = harness
        .run_execution_tool(
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
