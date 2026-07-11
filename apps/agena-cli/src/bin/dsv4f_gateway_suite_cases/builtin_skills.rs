use agena::permission::PermissionMode;
use anyhow::ensure;
use serde_json::json;

use super::{Harness, PendingReply, SuiteReport, assert_contains, baseline_permission};

use super::run_single;

pub(crate) async fn run_skills_cases(
    harness: &Harness,
    report: &mut SuiteReport,
) -> anyhow::Result<()> {
    let _ = run_single(
        harness,
        report,
        "skills.list",
        "skills.list",
        "agena.skills.list",
        json!({"kind": "skill", "offset": 0, "limit": 20, "verbose": true}),
        Some("init"),
    )
    .await?;
    let _ = run_single(
        harness,
        report,
        "skills.get",
        "skills.get",
        "agena.skills.get",
        json!({"name": "init"}),
        Some("init"),
    )
    .await?;
    let _ = run_single(
        harness,
        report,
        "skills.run",
        "skills.run",
        "agena.skills.run",
        json!({"name": "init", "args": "DSV4F_SKILL_ARG"}),
        Some("DSV4F_SKILL_ARG"),
    )
    .await?;
    Ok(())
}

pub(crate) async fn run_code_cases(
    harness: &Harness,
    report: &mut SuiteReport,
) -> anyhow::Result<()> {
    let _ = run_single(
        harness,
        report,
        "code.search_ast",
        "code.search_ast",
        "agena.code.search_ast",
        json!({
            "path": "src/lib.rs",
            "pattern": "probe()",
            "language": "rust",
            "limit": 5
        }),
        Some("probe"),
    )
    .await?;
    let _ = run_single(
        harness,
        report,
        "code.syntax_tree",
        "code.syntax_tree",
        "agena.code.syntax_tree",
        json!({"path": "src/lib.rs", "language": "rust", "max_depth": 3}),
        Some("function_item"),
    )
    .await?;
    Ok(())
}

pub(crate) async fn run_lsp_cases(
    harness: &Harness,
    report: &mut SuiteReport,
) -> anyhow::Result<()> {
    let _ = run_single(
        harness,
        report,
        "lsp.servers",
        "lsp.servers",
        "agena.lsp.servers",
        json!({}),
        Some("rust"),
    )
    .await?;
    let position = json!({"file_path": "src/lib.rs", "line": 2, "character": 28});
    if harness.selector.enabled("lsp.definition") {
        let definition = harness
            .run_gateway_target(
                harness
                    .create_session(
                        "dsv4f lsp definition",
                        &["agena.lsp.definition"],
                        baseline_permission(PermissionMode::Allow),
                    )
                    .await?,
                "lsp.definition",
                "lsp.definition",
                position.clone(),
                PendingReply::None,
                true,
            )
            .await?;
        assert_contains(&definition, "src/lib.rs")?;
        report.pass("lsp.definition");
    }
    if harness.selector.enabled("lsp.references") {
        let references = harness
            .run_gateway_target(
                harness
                    .create_session(
                        "dsv4f lsp references",
                        &["agena.lsp.references"],
                        baseline_permission(PermissionMode::Allow),
                    )
                    .await?,
                "lsp.references",
                "lsp.references",
                json!({"file_path": "src/lib.rs", "line": 2, "character": 28, "include_declaration": true}),
                PendingReply::None,
                true,
            )
            .await?;
        assert_contains(&references, "src/lib.rs")?;
        report.pass("lsp.references");
    }
    if harness.selector.enabled("lsp.hover") {
        let hover = harness
            .run_gateway_target(
                harness
                    .create_session(
                        "dsv4f lsp hover",
                        &["agena.lsp.hover"],
                        baseline_permission(PermissionMode::Allow),
                    )
                    .await?,
                "lsp.hover",
                "lsp.hover",
                position,
                PendingReply::None,
                true,
            )
            .await?;
        assert_contains(&hover, "probe")?;
        report.pass("lsp.hover");
    }
    if harness.selector.enabled("lsp.diagnostics") {
        let diagnostics = harness
            .run_gateway_target(
                harness
                    .create_session(
                        "dsv4f lsp diagnostics",
                        &["agena.lsp.diagnostics"],
                        baseline_permission(PermissionMode::Allow),
                    )
                    .await?,
                "lsp.diagnostics",
                "lsp.diagnostics",
                json!({"file_path": "src/lib.rs"}),
                PendingReply::None,
                true,
            )
            .await?;
        ensure!(
            diagnostics.payload().get("entries").is_some(),
            "lsp.diagnostics did not return an entries payload"
        );
        report.pass("lsp.diagnostics");
    }
    Ok(())
}
