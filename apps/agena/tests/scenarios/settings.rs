use super::fixture;
use agena_api::resource::{ModelRef, RunOptions};
use agena_testkit::{
    ConfigFile, EditOptions, ModelOptions, Reply, RequestMatch, SettingsLayer, TestResult,
};
use anyhow::ensure;
use serde_json::{Value, json};
use std::{ffi::OsString, time::Duration};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn settings_sources_are_resolved_by_product_and_environment_can_be_removed_on_restart()
-> TestResult {
    fixture()
        .configure_global(|config| {
            config["session"]["max_turns"] = json!(11);
        })
        .workspace_config(json!({"session": {"max_turns": 12}}))
        .env("AGENA_SESSION_MAX_TURNS", "13")
        .cli_override("ui.locale=zh-CN")
        .run(|case| async move {
            let before = case.config_snapshot().await?;
            ensure!(
                before.global.pointer("/value/session/max_turns") == Some(&json!(11)),
                "{:?}",
                before.global
            );
            ensure!(
                before.workspace.pointer("/value/session/max_turns") == Some(&json!(12)),
                "{:?}",
                before.workspace
            );
            ensure!(
                before.loaded.pointer("/config/session/max_turns") == Some(&json!(13)),
                "{:?}",
                before.loaded
            );
            ensure!(before.loaded.pointer("/config/ui/locale") == Some(&json!("zh-CN")));

            case.settings()
                .set(
                    SettingsLayer::Workspace,
                    "session.max_turns",
                    json!(14),
                    EditOptions::default(),
                )
                .await?
                .require_success()?;
            let edited = case.config_snapshot().await?;
            ensure!(edited.workspace.pointer("/value/session/max_turns") == Some(&json!(14)));
            ensure!(
                edited.loaded.pointer("/config/session/max_turns") == Some(&json!(13)),
                "environment did not retain precedence"
            );

            case.restart_with_env([("AGENA_SESSION_MAX_TURNS", None::<OsString>)])
                .await?;
            let restarted = case.config_snapshot().await?;
            ensure!(
                restarted.loaded.pointer("/config/session/max_turns") == Some(&json!(14)),
                "{:?}",
                restarted.loaded
            );
            ensure!(restarted.server.instance_id == before.server.instance_id);
            ensure!(restarted.server.id != before.server.id);
            ensure!(restarted.loaded.pointer("/config/ui/locale") == Some(&json!("zh-CN")));
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn settings_dry_run_revision_conflicts_validation_and_delete_flags() -> TestResult {
    fixture()
        .run(|case| async move {
            let settings = case.settings();
            let original: Value = settings
                .layer(SettingsLayer::Global, None)
                .await?
                .require_success()?
                .json()?;
            let revision = original["revision"]
                .as_str()
                .expect("preset file has revision")
                .to_owned();
            let dry: Value = settings
                .set(
                    SettingsLayer::Global,
                    "session.max_turns",
                    json!(21),
                    EditOptions {
                        dry_run: true,
                        expected_revision: Some(revision.clone()),
                        ..Default::default()
                    },
                )
                .await?
                .require_success()?
                .json()?;
            ensure!(dry["dry_run"] == true && dry["changed"] == true, "{dry}");
            ensure!(dry["reload"].is_null(), "dry run triggered direct reload");
            let after_dry: Value = settings
                .layer(SettingsLayer::Global, None)
                .await?
                .require_success()?
                .json()?;
            ensure!(
                original["revision"] == after_dry["revision"]
                    && original["value"] == after_dry["value"]
            );

            let invalid = settings
                .set(
                    SettingsLayer::Global,
                    "session.max_turns",
                    json!("not-a-number"),
                    EditOptions::default(),
                )
                .await?;
            ensure!(
                invalid.status.is_client_error(),
                "invalid type was accepted: {invalid:?}"
            );

            let edit: Value = settings
                .set(
                    SettingsLayer::Global,
                    "session.max_turns",
                    json!(22),
                    EditOptions {
                        expected_revision: Some(revision.clone()),
                        ..Default::default()
                    },
                )
                .await?
                .require_success()?
                .json()?;
            ensure!(
                edit["reload_requested"] == true && edit["reload"].is_object(),
                "{edit}"
            );
            let stale = settings
                .set(
                    SettingsLayer::Global,
                    "session.max_turns",
                    json!(23),
                    EditOptions {
                        expected_revision: Some(revision),
                        ..Default::default()
                    },
                )
                .await?;
            ensure!(
                stale.status.is_client_error(),
                "stale revision did not conflict: {stale:?}"
            );
            let stale_problem: Value = stale.json()?;
            ensure!(
                stale_problem
                    .pointer("/problem/user/fallback")
                    .and_then(Value::as_str)
                    .is_some_and(|message| message.contains("revision changed")),
                "{stale_problem}"
            );
            ensure!(
                case.config_snapshot()
                    .await?
                    .loaded
                    .pointer("/config/session/max_turns")
                    == Some(&json!(22))
            );

            let deleted: Value = settings
                .delete(
                    SettingsLayer::Global,
                    "session.max_turns",
                    EditOptions {
                        reload: false,
                        ..Default::default()
                    },
                )
                .await?
                .require_success()?
                .json()?;
            ensure!(deleted["deleted"] == true, "{deleted}");
            ensure!(
                deleted["reload_requested"] == false && deleted["reload"].is_null(),
                "{deleted}"
            );
            // The independent production file watcher may reload later; reload=false
            // promises no direct reload by this request, not an unchanged generation.
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn per_run_options_reach_the_real_provider_request() -> TestResult {
    fixture().model_options(ModelOptions {
        models: vec!["fake-model".into(), "another-local-model".into()],
        ..Default::default()
    }).configure_global(|config| {
        config["providers"]["testkit"]["adapters"]["openai_responses"]["models"]["another-local-model"] =
            json!({"context_window_tokens": 131072, "max_output_tokens": 8192,
                   "native_compaction": false, "agena_tools": {"mode": "provider_protocol"}});
    }).run(|case| async move {
        case.model().expect(RequestMatch::user_text("run-options-marker").model("another-local-model"))
            .check(|request| {
                ensure!(request.body["max_output_tokens"] == 123, "output limit lost: {}", request.body);
                ensure!(request.body.to_string().contains("per-run-system-marker"), "run instructions lost");
                Ok(())
            })
            .reply(Reply::text("run-options-applied"));
        let user = case.user().new_session().await?;
        let result = user.send_with("run-options-marker", RunOptions {
            model: Some(ModelRef::new_with_adapter("testkit", "openai_responses", "another-local-model")),
            system: Some("per-run-system-marker".into()),
            max_output_tokens: Some(123),
            ..Default::default()
        }).await?.wait_settled().await?;
        result.assert_succeeded()?;
        result.assert_assistant_text("run-options-applied")?;
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_source_fails_startup_with_process_diagnostics() -> TestResult {
    let failure = fixture()
        .raw()
        .global_config(ConfigFile::Bytes(b"{ malformed configuration".to_vec()))
        .expect_start_failure()
        .await?;
    ensure!(
        !failure.cause.contains("readiness timed out"),
        "configuration failure was hidden by readiness timeout: {}",
        failure.cause
    );
    ensure!(
        !failure.stderr.is_empty() || !failure.stdout.is_empty(),
        "missing startup diagnostics"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn raw_http_exposes_real_authentication_and_validation() -> TestResult {
    fixture()
        .run(|case| async move {
            let http = case.http().await;
            let unauthorized = http.anonymous().get("/api/v1/runtime").await?;
            ensure!(unauthorized.status.as_u16() == 401, "{unauthorized:?}");
            let authenticated = http.get("/api/v1/runtime").await?;
            authenticated.require_success()?;
            let malformed = http
                .send_bytes(
                    reqwest::Method::PUT,
                    "/api/v1/settings/layers/global",
                    Some("application/json"),
                    b"{broken".to_vec(),
                )
                .await?;
            ensure!(malformed.status.is_client_error(), "{malformed:?}");
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn settings_waits_observe_loaded_values_and_only_the_requested_reload() -> TestResult {
    fixture()
        .run(|case| async move {
            let settings = case.settings();
            let before = settings.snapshot().await?;
            settings
                .set(
                    SettingsLayer::Workspace,
                    "session.max_turns",
                    json!(17),
                    EditOptions {
                        reload: false,
                        ..Default::default()
                    },
                )
                .await?
                .require_success()?;
            let _loaded = settings
                .wait_loaded("/config/session/max_turns", json!(17))
                .await?;
            ensure!(
                settings
                    .snapshot()
                    .await?
                    .workspace
                    .pointer("/value/session/max_turns")
                    == Some(&json!(17))
            );

            // Calls may start independent tasks or coalesce onto the same in-flight
            // task. Each handle must still resolve its own accepted receipt.
            let first = settings.start_reload().await?;
            let second = settings.start_reload().await?;
            for reload in [&first, &second] {
                let snapshot = reload.wait().await?;
                let runtime: agena_api::resource::RuntimeStatusResponse =
                    serde_json::from_value(snapshot.runtime)?;
                let task = runtime
                    .background_tasks
                    .iter()
                    .find(|task| task.id == reload.task_id())
                    .ok_or_else(|| anyhow::anyhow!("completed reload receipt is absent"))?;
                ensure!(task.status == agena_api::resource::RuntimeBackgroundTaskStatus::Succeeded);
                ensure!(snapshot.server == before.server);
            }
            let fresh_before = settings.snapshot().await?;
            let fresh = settings.reload_and_wait().await?;
            ensure!(fresh.generation > fresh_before.generation);

            // Other successful tasks and newer generations cannot satisfy a made-up
            // receipt. Keep this an actual product query, not a fake status object.
            let unknown = settings
                .track_reload("unknown-reload-receipt", &before)
                .with_timeout(Duration::from_secs(1))
                .wait()
                .await
                .unwrap_err()
                .to_string();
            ensure!(
                unknown.contains("unknown-reload-receipt") && unknown.contains("task absent"),
                "{unknown}"
            );
            let mismatch = settings
                .with_timeout(Duration::from_secs(1))
                .wait_loaded("/config/session/max_turns", json!(18))
                .await
                .unwrap_err()
                .to_string();
            ensure!(
                mismatch.contains("/config/session/max_turns")
                    && mismatch.contains("18")
                    && mismatch.contains("17")
                    && mismatch.contains("generation"),
                "{mismatch}"
            );
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_reload_keeps_loaded_settings_and_old_receipts_reject_a_new_server() -> TestResult {
    fixture()
        .configure_global(|config| {
            config["session"]["max_turns"] = json!(11);
        })
        .run(|case| async move {
            let settings = case.settings();
            let original = tokio::fs::read(&case.paths().global_config).await?;
            let before = settings.snapshot().await?;
            case.write_config(
                SettingsLayer::Global,
                ConfigFile::Bytes(b"{ malformed reload".to_vec()),
            )
            .await?;
            let failed = settings.start_reload().await?;
            let error = failed.wait().await.unwrap_err().to_string();
            ensure!(
                error.contains(failed.task_id()) && error.contains("Failed"),
                "{error}"
            );
            let client = case.client().await;
            let status = client.runtime_status().await?;
            ensure!(status.generation == before.generation);
            let task = status
                .background_tasks
                .iter()
                .find(|task| task.id == failed.task_id())
                .ok_or_else(|| anyhow::anyhow!("failed task receipt is absent"))?;
            ensure!(
                task.status == agena_api::resource::RuntimeBackgroundTaskStatus::Failed
                    && task.failure.is_some()
            );
            let still_loaded = settings
                .wait_loaded("/config/session/max_turns", json!(11))
                .await?;
            ensure!(
                still_loaded.generation == before.generation
                    && still_loaded.server == before.server
            );
            ensure!(
                settings.snapshot().await.is_err(),
                "malformed source unexpectedly parsed"
            );

            case.write_config(SettingsLayer::Global, ConfigFile::Bytes(original))
                .await?;
            let restored = settings.reload_and_wait().await?;
            ensure!(restored.loaded.pointer("/config/session/max_turns") == Some(&json!(11)));
            let old = settings.start_reload().await?;
            old.wait().await?;
            case.restart().await?;
            let stale = old.wait().await.unwrap_err().to_string();
            ensure!(
                stale.contains(old.task_id()) && stale.contains("different server identity"),
                "{stale}"
            );
            let current = case.settings().reload_and_wait().await?;
            ensure!(current.server != before.server);
            ensure!(current.loaded.pointer("/config/session/max_turns") == Some(&json!(11)));
            Ok(())
        })
        .await
}
