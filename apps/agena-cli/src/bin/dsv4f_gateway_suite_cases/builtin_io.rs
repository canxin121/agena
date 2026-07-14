use std::fs;

use agena::permission::PermissionMode;
use anyhow::ensure;
use serde_json::{Value, json};

use super::{
    Fixture, Harness, PendingReply, SuiteReport, assert_contains, baseline_permission,
    commit_fixture_change, payload_string,
};

use super::run_single;

pub(crate) async fn run_cron_cases(
    harness: &Harness,
    report: &mut SuiteReport,
) -> anyhow::Result<()> {
    if !harness.selector.any_in_group("cron") {
        return Ok(());
    }
    let session = harness
        .create_session(
            "dsv4f cron chain",
            &[
                "agena.cron.list",
                "agena.cron.create",
                "agena.cron.wakeup",
                "agena.cron.delete",
            ],
            baseline_permission(PermissionMode::Allow),
        )
        .await?;
    let list = harness
        .run_gateway_target(
            session,
            "cron.list",
            "cron.list",
            json!({}),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        list.payload().get("jobs").is_some(),
        "cron.list payload lacks jobs"
    );
    report.pass("cron.list");
    let created = harness
        .run_gateway_target(
            session,
            "cron.create",
            "cron.create",
            json!({"expression": "0 0 0 1 1 *", "prompt": "DSV4F cron probe", "max_age_days": 1}),
            PendingReply::None,
            true,
        )
        .await?;
    let cron_id = payload_string(&created.payload(), "id")?;
    report.pass("cron.create");
    let wakeup = harness
        .run_gateway_target(
            session,
            "cron.wakeup",
            "cron.wakeup",
            json!({"delay_seconds": 3600, "prompt": "DSV4F wakeup probe", "reason": "gateway suite"}),
            PendingReply::None,
            true,
        )
        .await?;
    let wakeup_id = payload_string(&wakeup.payload(), "id")?;
    report.pass("cron.wakeup");
    let deleted = harness
        .run_gateway_target(
            session,
            "cron.delete",
            "cron.delete",
            json!({"id": cron_id}),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        deleted.payload().get("removed").and_then(Value::as_bool) == Some(true),
        "cron.delete did not report removed=true"
    );
    report.pass("cron.delete");
    let cleanup = harness
        .run_gateway_target(
            session,
            "cron.delete.wakeup_cleanup",
            "cron.delete",
            json!({"id": wakeup_id}),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        cleanup.payload().get("removed").and_then(Value::as_bool) == Some(true),
        "wakeup cleanup did not report removed=true"
    );
    Ok(())
}

pub(crate) async fn run_fs_cases(
    harness: &Harness,
    fixture: &Fixture,
    report: &mut SuiteReport,
) -> anyhow::Result<()> {
    let _ = run_single(
        harness,
        report,
        "fs.glob",
        "fs.glob",
        "agena.fs.glob",
        json!({"pattern": "**/*.rs", "path": "src"}),
        Some("lib.rs"),
    )
    .await?;
    let _ = run_single(
        harness,
        report,
        "fs.grep",
        "fs.grep",
        "agena.fs.grep",
        json!({"pattern": "pub fn probe", "path": "src", "include": "*.rs"}),
        Some("pub fn probe"),
    )
    .await?;
    let _ = run_single(
        harness,
        report,
        "fs.read",
        "fs.read",
        "agena.fs.read",
        json!({"file_path": "src/lib.rs", "mode": "text", "offset": 1, "limit": 20}),
        Some("pub fn probe"),
    )
    .await?;
    if harness.selector.enabled("fs.apply_patch") {
        let session = harness
            .create_session(
                "dsv4f fs.apply_patch",
                &["agena.fs.apply_patch"],
                baseline_permission(PermissionMode::Allow),
            )
            .await?;
        let outcome = harness
            .run_gateway_target(
                session,
                "fs.apply_patch",
                "fs.apply_patch",
                json!({
                    "patch": "*** Begin Patch\n*** Add File: probe/patch.txt\n+DSV4F_PATCH_MARKER\n*** End Patch"
                }),
                PendingReply::None,
                true,
            )
            .await?;
        assert_contains(&outcome, "probe/patch.txt")?;
        ensure!(
            fs::read_to_string(fixture.workspace.join("probe/patch.txt"))?
                .contains("DSV4F_PATCH_MARKER"),
            "fs.apply_patch did not create the expected file"
        );
        commit_fixture_change(&fixture.workspace, "exercise fs.apply_patch")?;
        report.pass("fs.apply_patch");
    }
    Ok(())
}

pub(crate) async fn run_settings_cases(
    harness: &Harness,
    report: &mut SuiteReport,
) -> anyhow::Result<()> {
    if !harness.selector.any_in_group("settings") {
        return Ok(());
    }
    let session = harness
        .create_session(
            "dsv4f settings chain",
            &[
                "agena.settings.set",
                "agena.settings.get",
                "agena.settings.inspect",
                "agena.settings.patch",
                "agena.settings.list",
                "agena.settings.delete",
                "agena.settings.validate",
            ],
            baseline_permission(PermissionMode::Allow),
        )
        .await?;
    let set = harness
        .run_gateway_target(
            session,
            "settings.set",
            "settings.set",
            json!({
                "path": "tracing.filter",
                "value": "dsv4f=debug",
                "dry_run": false,
                "validate": true,
                "reload": false
            }),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        set.payload().get("changed").and_then(Value::as_bool) == Some(true),
        "settings.set did not report a persistent change"
    );
    report.pass("settings.set");
    let get = harness
        .run_gateway_target(
            session,
            "settings.get",
            "settings.get",
            json!({"path": "tracing.filter", "source": "file"}),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        get.payload().to_string().contains("dsv4f=debug"),
        "settings.get did not observe the value written by settings.set"
    );
    report.pass("settings.get");
    let inspect = harness
        .run_gateway_target(
            session,
            "settings.inspect",
            "settings.inspect",
            json!({"path": "tracing.filter"}),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        inspect
            .payload()
            .get("global")
            .and_then(|global| global.get("value"))
            .and_then(Value::as_str)
            == Some("dsv4f=debug"),
        "settings.inspect did not report the persisted global value"
    );
    ensure!(
        inspect
            .payload()
            .get("workspace")
            .and_then(|workspace| workspace.get("layer"))
            .and_then(Value::as_str)
            == Some("workspace"),
        "settings.inspect did not include the workspace layer"
    );
    report.pass("settings.inspect");
    let patch = harness
        .run_gateway_target(
            session,
            "settings.patch",
            "settings.patch",
            json!({
                "path": "tracing",
                "changes": {"filter": "dsv4f=trace"},
                "dry_run": false,
                "validate": true,
                "reload": false
            }),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        patch.payload().get("changed").and_then(Value::as_bool) == Some(true),
        "settings.patch did not report a persistent change"
    );
    report.pass("settings.patch");
    let list = harness
        .run_gateway_target(
            session,
            "settings.list",
            "settings.list",
            json!({"path": "tracing", "source": "file", "recursive": true}),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        list.payload().to_string().contains("filter"),
        "settings.list did not enumerate the patched setting"
    );
    report.pass("settings.list");
    let delete = harness
        .run_gateway_target(
            session,
            "settings.delete",
            "settings.delete",
            json!({
                "path": "tracing.filter",
                "dry_run": false,
                "validate": true,
                "reload": false
            }),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        delete.payload().get("deleted").and_then(Value::as_bool) == Some(true),
        "settings.delete did not report deleted=true"
    );
    report.pass("settings.delete");
    let validate = harness
        .run_gateway_target(
            session,
            "settings.validate",
            "settings.validate",
            json!({}),
            PendingReply::None,
            true,
        )
        .await?;
    ensure!(
        validate.payload().get("valid").and_then(Value::as_bool) == Some(true),
        "settings.validate did not report valid=true"
    );
    report.pass("settings.validate");
    Ok(())
}
