use agena::permission::PermissionMode;
use anyhow::ensure;
use serde_json::json;

use super::{
    GATEWAY_CALL, GATEWAY_HELP, GATEWAY_LIST, GATEWAY_SEARCH, GATEWAY_TAGS, Harness, SuiteReport,
    assert_contains, baseline_permission,
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

    if harness.selector.any_in_group("tools.help") || harness.selector.any_in_group("tools.call") {
        let session = harness
            .create_session(
                "dsv4f provider tools help/call",
                &[GATEWAY_HELP, GATEWAY_CALL, "agena.schema_lab.echo"],
                baseline_permission(PermissionMode::Allow),
            )
            .await?;
        let help = harness
            .run_native_gateway_function(
                session,
                "tools.help",
                "tools_help",
                GATEWAY_HELP,
                json!({"tool": "schema_lab.echo"}),
                Some("schema_lab.echo"),
            )
            .await?;
        if harness.selector.enabled("tools.help") {
            report.pass("tools.help");
        }
        if harness.selector.enabled("tools.call") {
            let call = harness
                .run_native_gateway_function(
                    help.session.id,
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
    }
    Ok(())
}
