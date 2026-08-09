use std::{fs, path::Path, time::Duration};

use agena_domain::PermissionMode;
use agena_domain::PermissionReplyKind;
use anyhow::{Context, ensure};
use serde_json::{Value, json};

use super::run_single;
use super::{
    Fixture, Harness, PendingReply, SuiteReport, assert_contains, baseline_permission,
    payload_string,
};

pub(crate) async fn run_snapshot_cases(
    harness: &Harness,
    report: &mut SuiteReport,
) -> anyhow::Result<()> {
    if !harness.selector.any_in_group("snapshot") {
        return Ok(());
    }
    let session = harness
        .create_session(
            "dsv4f snapshot chain",
            &["agena.snapshot.enter", "agena.snapshot.exit"],
            baseline_permission(PermissionMode::Allow),
        )
        .await?;
    let entered = harness
        .run_execution_tool(
            session,
            "snapshot.enter",
            "snapshot.enter",
            json!({"target": "new", "name": "dsv4f-snapshot"}),
            PendingReply::None,
            true,
        )
        .await?;
    let snapshot_path = payload_string(&entered.payload(), "path")?;
    ensure!(
        Path::new(&snapshot_path).is_dir(),
        "snapshot path was not created: {snapshot_path}"
    );
    report.pass("snapshot.enter");
    let exited = harness
        .run_execution_tool(
            session,
            "snapshot.exit",
            "snapshot.exit",
            json!({"exit_action": "remove", "discard_changes": false}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&exited, "remove")?;
    ensure!(
        !Path::new(&snapshot_path).exists(),
        "snapshot.exit did not remove {snapshot_path}"
    );
    report.pass("snapshot.exit");
    Ok(())
}

pub(crate) async fn run_task_case(
    harness: &Harness,
    report: &mut SuiteReport,
) -> anyhow::Result<()> {
    if !harness.selector.enabled("tasks.run") {
        return Ok(());
    }
    let session = harness
        .create_session(
            "dsv4f task run",
            &["agena.tasks.run"],
            baseline_permission(PermissionMode::Allow),
        )
        .await?;
    let outcome = harness
        .run_execution_tool_with_timeout(
            session,
            "tasks.run",
            "tasks.run",
            json!({
                "description": "dsv4f Tool API task",
                "prompt": "Reply with exactly SUBTASK_OK. Do not call any tools.",
                "access": "read_only"
            }),
            PendingReply::None,
            true,
            harness.task_timeout,
        )
        .await?;
    let child_id = outcome
        .payload()
        .get("session_id")
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        })
        .context("tasks.run payload lacks session_id")?;
    let child_messages = tokio::time::timeout(harness.task_timeout, async {
        loop {
            let _presentation = harness
                .session_queries
                .session_presentation(child_id)
                .await?;
            if harness
                .execution_control
                .active_execution(child_id)
                .await
                .is_none()
            {
                let messages = harness
                    .session_queries
                    .list_projected_runs(child_id, true)
                    .await
                    .context("load completed tasks.run child transcript")?;
                return Ok::<_, anyhow::Error>(messages);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .context("tasks.run child session did not complete")??;
    ensure!(
        projected_transcript_text(&child_messages).contains("SUBTASK_OK"),
        "tasks.run child transcript did not contain SUBTASK_OK: {}",
        projected_transcript_text(&child_messages)
    );
    report.pass("tasks.run");
    Ok(())
}

fn projected_transcript_text(messages: &[agena_runtime::SessionProjectedRun]) -> String {
    messages
        .iter()
        .flat_map(|message| message.parts.iter())
        .filter_map(|part| match part.detail.as_ref() {
            Some(agena_runtime::SessionProjectedPartDetail::Text { text, .. }) => {
                Some(text.as_str())
            }
            Some(agena_runtime::SessionProjectedPartDetail::Operation(operation)) => {
                Some(operation.model_output.text.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) async fn run_web_cases(
    harness: &Harness,
    fixture: &Fixture,
    report: &mut SuiteReport,
) -> anyhow::Result<()> {
    if !harness.selector.any_in_group("web") {
        return Ok(());
    }
    let session = harness
        .create_session(
            "dsv4f web chain",
            &["agena.web.fetch", "agena.web.crawl", "agena.web.search"],
            baseline_permission(PermissionMode::Allow),
        )
        .await?;
    let fetched = harness
        .run_execution_tool(
            session,
            "web.fetch",
            "web.fetch",
            json!({
                "url": fixture.web.url("/page"),
                "prompt": "return DSV4F_WEB_MARKER",
                "use_cache": false,
                "render_js": false
            }),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&fetched, "DSV4F_WEB_MARKER")?;
    report.pass("web.fetch");
    let crawled = harness
        .run_execution_tool(
            session,
            "web.crawl",
            "web.crawl",
            json!({
                "start_url": fixture.web.url("/"),
                "max_pages": 2,
                "max_depth": 1,
                "same_host_only": true,
                "use_cache": false,
                "render_js": false
            }),
            PendingReply::None,
            true,
        )
        .await?;
    let crawl_payload = crawled.payload();
    ensure!(
        crawl_payload
            .get("stored_count")
            .and_then(Value::as_u64)
            .is_some_and(|count| count >= 2),
        "web.crawl did not index both fixture pages: {crawl_payload}"
    );
    ensure!(
        crawl_payload.get("failure_count").and_then(Value::as_u64) == Some(0),
        "web.crawl reported failures for the local fixture: {crawl_payload}"
    );
    report.pass("web.crawl");
    let search = harness
        .run_execution_tool(
            session,
            "web.search",
            "web.search",
            json!({"query": "Agena DSV4F Tool API probe", "engine": "auto", "limit": 1}),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        search
            .payload()
            .get("attempted_engines")
            .and_then(Value::as_array)
            .is_some_and(|engines| !engines.is_empty()),
        "web.search did not report attempted engines"
    );
    report.pass("web.search");
    Ok(())
}

pub(crate) async fn run_external_plugin_suite(
    harness: &Harness,
    fixture: &Fixture,
    report: &mut SuiteReport,
) -> anyhow::Result<()> {
    if !harness.selector.any_in_group("external") {
        return Ok(());
    }
    let _ = run_single(
        harness,
        report,
        "external.echo",
        "echo",
        "example.echo.echo",
        json!({"text": "hello external"}),
        Some("[PREPARED] HELLO EXTERNAL"),
    )
    .await?;
    if harness.selector.enabled("external.echo_stdio") {
        let session = harness
            .create_session(
                "dsv4f external echo_stdio stream",
                &["example.echo_stdio.echo"],
                baseline_permission(PermissionMode::Allow),
            )
            .await?;
        let outcome = harness
            .run_streaming_execution_tool(
                session,
                "external.echo_stdio",
                "echo_stdio.echo",
                json!({"text": "streaming external"}),
                "stdio-echo: [prepared] streaming external",
            )
            .await?;
        assert_contains(&outcome, "stdio-echo: [prepared] streaming external")?;
        report.pass("external.echo_stdio");
    }
    let _ = run_single(
        harness,
        report,
        "external.notes.format",
        "notes.format",
        "example.notes.format",
        json!({"text": "streaming note\n"}),
        Some("[probe] streaming note"),
    )
    .await?;
    if harness.selector.enabled("external.notes.write") {
        let session = harness
            .create_session(
                "dsv4f external notes.write",
                &["example.notes.write"],
                baseline_permission(PermissionMode::Allow),
            )
            .await?;
        let outcome = harness
            .run_execution_tool(
                session,
                "external.notes.write",
                "notes.write",
                json!({"path": "probe/note.txt", "text": "EXTERNAL_NOTE_OK", "append": false}),
                PendingReply::None,
                true,
            )
            .await?;
        let write_payload = outcome.payload();
        ensure!(
            write_payload.get("path").and_then(Value::as_str) == Some("probe/note.txt")
                && write_payload.get("append").and_then(Value::as_bool) == Some(false)
                && write_payload
                    .get("bytes")
                    .and_then(Value::as_u64)
                    .is_some_and(|bytes| bytes > 0),
            "notes.write returned an unexpected payload: {write_payload}"
        );
        ensure!(
            fs::read_to_string(fixture.workspace.join("probe/note.txt"))?
                .contains("[probe] EXTERNAL_NOTE_OK"),
            "notes.write did not create the formatted fixture note"
        );
        report.pass("external.notes.write");
    }
    Ok(())
}

pub(crate) async fn run_nested_permission_suite(
    harness: &Harness,
    fixture: &Fixture,
    report: &mut SuiteReport,
) -> anyhow::Result<()> {
    if !harness.selector.any_in_group("permission") {
        return Ok(());
    }
    run_nested_permission_mode(
        harness,
        fixture,
        "permission.allow_once",
        PermissionReplyKind::AllowOnce,
        true,
        true,
    )
    .await?;
    run_nested_permission_mode(
        harness,
        fixture,
        "permission.allow_always",
        PermissionReplyKind::AllowAlways,
        true,
        false,
    )
    .await?;
    run_nested_permission_mode(
        harness,
        fixture,
        "permission.deny_once",
        PermissionReplyKind::DenyOnce,
        false,
        true,
    )
    .await?;
    run_nested_permission_mode(
        harness,
        fixture,
        "permission.deny_always",
        PermissionReplyKind::DenyAlways,
        false,
        false,
    )
    .await?;
    report.pass("permission.nested_host_tool_api");
    Ok(())
}

/// Exercise the four persistence modes for a dynamic permission discovered by
/// an execution tool *inside* tools.call. The second invocation distinguishes once from
/// always without granting broad Tool API permissions.
pub(crate) async fn run_nested_permission_mode(
    harness: &Harness,
    fixture: &Fixture,
    label: &str,
    first_reply: PermissionReplyKind,
    first_succeeds: bool,
    second_requires_reply: bool,
) -> anyhow::Result<()> {
    let session = harness
        .create_session(
            label,
            &["agena.web.fetch"],
            baseline_permission(PermissionMode::Ask),
        )
        .await?;
    let input = json!({
        "url": fixture.web.url("/page"),
        "prompt": "return DSV4F_WEB_MARKER",
        "use_cache": false,
        "render_js": false
    });
    let hits_before = fixture.web.hits();
    let first = harness
        .run_execution_tool(
            session,
            &format!("{label}.first"),
            "web.fetch",
            input.clone(),
            PendingReply::Permission(first_reply),
            first_succeeds,
        )
        .await?;
    if first_succeeds {
        assert_contains(&first, "DSV4F_WEB_MARKER")?;
    } else {
        ensure!(
            fixture.web.hits() == hits_before,
            "denied host permission reached the web server"
        );
    }

    let second_reply = if second_requires_reply {
        if first_succeeds {
            PendingReply::Permission(PermissionReplyKind::DenyOnce)
        } else {
            PendingReply::Permission(PermissionReplyKind::AllowOnce)
        }
    } else {
        PendingReply::None
    };
    let second_succeeds = if second_requires_reply {
        !first_succeeds
    } else {
        first_succeeds
    };
    let hits_before_second = fixture.web.hits();
    let second = harness
        .run_execution_tool(
            session,
            &format!("{label}.second"),
            "web.fetch",
            input,
            second_reply,
            second_succeeds,
        )
        .await?;
    if second_succeeds {
        assert_contains(&second, "DSV4F_WEB_MARKER")?;
    } else {
        ensure!(
            fixture.web.hits() == hits_before_second,
            "denied persistent host permission reached the web server"
        );
    }
    Ok(())
}
