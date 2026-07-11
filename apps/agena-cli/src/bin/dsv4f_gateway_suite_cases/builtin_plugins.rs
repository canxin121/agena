use agena::permission::PermissionMode;
use serde_json::json;

use super::{Harness, PendingReply, SuiteReport, assert_contains, baseline_permission};

use super::run_single;

pub(crate) async fn run_memory_cases(
    harness: &Harness,
    report: &mut SuiteReport,
) -> anyhow::Result<()> {
    if !harness.selector.any_in_group("memory") {
        return Ok(());
    }
    let session = harness
        .create_session(
            "dsv4f memory chain",
            &[
                "agena.memory.write",
                "agena.memory.get",
                "agena.memory.list",
                "agena.memory.search",
                "agena.memory.delete",
            ],
            baseline_permission(PermissionMode::Allow),
        )
        .await?;
    let write = harness
        .run_gateway_target(
            session,
            "memory.write",
            "memory.write",
            json!({
                "name": "probe-memory",
                "description": "dsv4f durable probe",
                "memory_type": "project",
                "content": "DSV4F_MEMORY_MARKER"
            }),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&write, "probe-memory")?;
    report.pass("memory.write");
    let get = harness
        .run_gateway_target(
            session,
            "memory.get",
            "memory.get",
            json!({"name": "probe-memory"}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&get, "DSV4F_MEMORY_MARKER")?;
    report.pass("memory.get");
    let list = harness
        .run_gateway_target(
            session,
            "memory.list",
            "memory.list",
            json!({"limit": 20}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&list, "probe-memory")?;
    report.pass("memory.list");
    let search = harness
        .run_gateway_target(
            session,
            "memory.search",
            "memory.search",
            json!({"query": "DSV4F_MEMORY_MARKER", "limit": 10}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&search, "probe-memory")?;
    report.pass("memory.search");
    let delete = harness
        .run_gateway_target(
            session,
            "memory.delete",
            "memory.delete",
            json!({"name": "probe-memory"}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&delete, "probe-memory")?;
    report.pass("memory.delete");
    Ok(())
}

pub(crate) async fn run_schema_lab_cases(
    harness: &Harness,
    report: &mut SuiteReport,
) -> anyhow::Result<()> {
    let _ = run_single(
        harness,
        report,
        "schema_lab.inspect",
        "schema_lab.inspect",
        "agena.schema_lab.inspect",
        json!({"section": "basics", "include_defaults": true}),
        Some("inspect"),
    )
    .await?;
    let _ = run_single(
        harness,
        report,
        "schema_lab.echo",
        "schema_lab.echo",
        "agena.schema_lab.echo",
        json!({"label": "dsv4f", "payload": {"marker": "SCHEMA_OK", "n": 1}}),
        Some("SCHEMA_OK"),
    )
    .await?;
    Ok(())
}

pub(crate) async fn run_mcp_cases(
    harness: &Harness,
    report: &mut SuiteReport,
) -> anyhow::Result<()> {
    if !harness.selector.any_in_group("mcp") {
        return Ok(());
    }
    let session = harness
        .create_session(
            "dsv4f mcp bridge chain",
            &[
                "agena.mcp.resources.list",
                "agena.mcp.resources.read",
                "agena.mcp.prompts.list",
                "agena.mcp.prompts.get",
                "agena.mcp.tools.call",
            ],
            baseline_permission(PermissionMode::Allow),
        )
        .await?;
    let resources = harness
        .run_gateway_target(
            session,
            "mcp.resources.list",
            "mcp.resources.list",
            json!({"server": "fixture"}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&resources, "fixture://hello")?;
    report.pass("mcp.resources.list");
    let resource = harness
        .run_gateway_target(
            session,
            "mcp.resources.read",
            "mcp.resources.read",
            json!({"server": "fixture", "uri": "fixture://hello"}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&resource, "MCP_RESOURCE_OK")?;
    report.pass("mcp.resources.read");
    let prompts = harness
        .run_gateway_target(
            session,
            "mcp.prompts.list",
            "mcp.prompts.list",
            json!({"server": "fixture"}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&prompts, "probe")?;
    report.pass("mcp.prompts.list");
    let prompt = harness
        .run_gateway_target(
            session,
            "mcp.prompts.get",
            "mcp.prompts.get",
            json!({"server": "fixture", "name": "probe", "arguments": {"name": "dsv4f"}}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&prompt, "MCP_PROMPT_OK")?;
    report.pass("mcp.prompts.get");
    let called = harness
        .run_gateway_target(
            session,
            "mcp.tools.call",
            "mcp.tools.call",
            json!({"server": "fixture", "name": "echo", "arguments": {"value": "MCP_OK"}}),
            PendingReply::None,
            true,
        )
        .await?;
    assert_contains(&called, "MCP_ECHO_OK")?;
    report.pass("mcp.tools.call");
    Ok(())
}
