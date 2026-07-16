use agena::{message::ExecutionStatus, permission::PermissionMode};
use anyhow::ensure;
use serde_json::{Value, json};

use super::{
    GATEWAY_CALL, GATEWAY_HELP, GATEWAY_LIST, GATEWAY_SEARCH, GATEWAY_TAGS, Harness, PendingReply,
    SuiteReport, assert_contains, baseline_permission, operations_since, transcript_since,
};

pub(crate) async fn run_gateway_meta_suite(
    harness: &Harness,
    report: &mut SuiteReport,
) -> anyhow::Result<()> {
    if harness.selector.enabled("tools.list") {
        let session = harness
            .create_session(
                "dsv4f tools.list",
                &[GATEWAY_LIST],
                baseline_permission(PermissionMode::Allow),
            )
            .await?;
        let outcome = harness
            .run_native_gateway_function(
                session,
                "tools.list",
                "tools_list",
                GATEWAY_LIST,
                json!({"offset": 0, "limit": 5, "tags": ["read_only"]}),
                None,
            )
            .await?;
        ensure!(
            outcome.payload().get("tools").is_some(),
            "tools.list did not return a tools payload"
        );
        report.pass("tools.list");
    }
    if harness.selector.enabled("tools.search") {
        let session = harness
            .create_session(
                "dsv4f tools.search",
                &[GATEWAY_SEARCH],
                baseline_permission(PermissionMode::Allow),
            )
            .await?;
        let outcome = harness
            .run_native_gateway_function(
                session,
                "tools.search",
                "tools_search",
                GATEWAY_SEARCH,
                json!({"query": "schema_lab", "offset": 0, "limit": 5}),
                None,
            )
            .await?;
        assert_contains(&outcome, "schema_lab")?;
        report.pass("tools.search");
    }
    if harness.selector.enabled("tools.tags") {
        let session = harness
            .create_session(
                "dsv4f tools.tags",
                &[GATEWAY_TAGS],
                baseline_permission(PermissionMode::Allow),
            )
            .await?;
        let outcome = harness
            .run_native_gateway_function(
                session,
                "tools.tags",
                "tools_tags",
                GATEWAY_TAGS,
                json!({"offset": 0, "limit": 10}),
                None,
            )
            .await?;
        ensure!(
            outcome.payload().get("tags").is_some(),
            "tools.tags did not return a tags payload"
        );
        report.pass("tools.tags");
    }

    if harness.selector.enabled("tools.help") {
        let session = harness
            .create_session(
                "dsv4f provider tools help",
                &[GATEWAY_HELP, "agena.schema_lab.echo"],
                baseline_permission(PermissionMode::Allow),
            )
            .await?;
        harness
            .run_native_gateway_function(
                session,
                "tools.help",
                "tools_help",
                GATEWAY_HELP,
                json!({"tool": "schema_lab.echo"}),
                Some("schema_lab.echo"),
            )
            .await?;
        report.pass("tools.help");
    }
    if harness.selector.enabled("tools.call") {
        // Deliberately use a fresh session with no tools_help receipt. This is
        // the real-provider regression for the reusable/advisory help
        // protocol: execution must never depend on consumable preflight state.
        let session = harness
            .create_session(
                "dsv4f provider direct tools call",
                &[GATEWAY_CALL, "agena.schema_lab.echo"],
                baseline_permission(PermissionMode::Allow),
            )
            .await?;
        let call = harness
            .run_native_gateway_function(
                session,
                "tools.call",
                "tools_call",
                GATEWAY_CALL,
                json!({
                    "tool": "schema_lab.echo",
                    "input": {"label": "gateway", "payload": {"marker": "GATEWAY_CALL_OK", "n": 1}}
                }),
                Some("GATEWAY_CALL_OK"),
            )
            .await?;
        assert_contains(&call, "GATEWAY_CALL_OK")?;
        report.pass("tools.call");
    }
    if harness.selector.enabled("tools.call.batch") {
        let session = harness
            .create_session(
                "dsv4f provider batched tools calls",
                &[GATEWAY_CALL, "agena.schema_lab.echo"],
                baseline_permission(PermissionMode::Allow),
            )
            .await?;
        let first = json!({
            "tool": "schema_lab.echo",
            "input": {"label": "first", "payload": {"marker": "GATEWAY_BATCH_FIRST"}}
        });
        let second = json!({
            "tool": "schema_lab.echo",
            "input": {"label": "second", "payload": {"marker": "GATEWAY_BATCH_SECOND"}}
        });
        let start_message_count = harness.manager.get_session(session).await?.messages.len();
        let mut options = harness.options.clone();
        options.request_override.set_parallel_tool_calls(Some(true));
        let prompt = format!(
            "This is an automated gateway batch test. In one assistant response, call the native function tools_call exactly twice, with these exact argument objects: {} and {}. Do not call tools_help or any other function. Both calls are independent and every supplied value is mandatory. After both tool results arrive, reply exactly DSV4F_TOOLS_CALL_BATCH_OK.",
            serde_json::to_string(&first)?,
            serde_json::to_string(&second)?,
        );
        let completed = harness
            .run_model_turn_with_options(
                session,
                prompt,
                PendingReply::None,
                harness.case_timeout,
                options,
            )
            .await?;
        let operations = operations_since(&completed, start_message_count);
        ensure!(
            operations
                .iter()
                .all(|operation| operation.invocation.name != GATEWAY_HELP),
            "batched tools_call unexpectedly requested tools_help"
        );
        let calls = operations
            .iter()
            .filter(|operation| operation.invocation.name == GATEWAY_CALL)
            .collect::<Vec<_>>();
        ensure!(
            calls.len() == 2,
            "expected two batched tools_call operations"
        );
        let actual = calls
            .iter()
            .map(|operation| Value::from(operation.invocation.input.clone()))
            .collect::<Vec<_>>();
        ensure!(
            actual.contains(&first) && actual.contains(&second),
            "batched tools_call inputs differed from the requested values: {actual:?}"
        );
        ensure!(
            calls.iter().all(|operation| {
                operation.status() == ExecutionStatus::Completed
                    && operation.error_message().is_none()
            }),
            "one or more batched tools_call operations failed"
        );
        ensure!(
            transcript_since(&completed, start_message_count).contains("DSV4F_TOOLS_CALL_BATCH_OK"),
            "batch test did not reach its terminal marker"
        );
        report.pass("tools.call.batch");
    }
    Ok(())
}
