use std::time::Duration;

use agena_testkit::{SettingsLayer, TestResult};
use anyhow::{Context, ensure};
use serde_json::{Value, json};

use super::{allow_permissions, fixture};

pub(super) const CASES: &[(&str, fn() -> TestResult)] = &[
    (
        "agena.settings",
        settings_tools_preserve_layers_revisions_and_activation,
    ),
    (
        "agena.provider",
        provider_models_use_current_config_and_validate_pagination,
    ),
];

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn settings_tools_preserve_layers_revisions_and_activation() -> TestResult {
    fixture()
        .expect_plugin("agena.settings")
        .builtin_plugin(
            "agena.settings",
            json!({
                "reads": {"source": "file", "file_layer": "workspace"},
                "edits": {"file_layer": "workspace", "reload_after_write": false}
            }),
        )
        .configure_global(|config| {
            config["session"]["max_turns"] = json!(8);
        })
        .workspace_config(json!({"session": {"max_turns": 6}}))
        .run(|case| async move {
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            let initial = user
                .call_tool("settings.get", json!({"path": "session.max_turns"}))
                .await?;
            initial.assert_succeeded()?;
            ensure!(initial.data()?["value"] == 6 && initial.data()?["layer"] == "workspace");
            let revision = initial.data()?["revision"]
                .as_str()
                .context("file revision")?
                .to_owned();

            let dry = user
                .call_tool(
                    "settings.set",
                    json!({
                        "path": "session.max_turns", "value": 9,
                        "expected_revision": revision, "dry_run": true
                    }),
                )
                .await?;
            dry.assert_succeeded()?;
            ensure!(
                dry.data()?["committed"] == false
                    && dry.data()?["activation_state"] == "not_committed"
            );
            let source = case
                .settings()
                .layer(SettingsLayer::Workspace, None)
                .await?
                .require_success()?
                .json::<Value>()?;
            ensure!(source["value"]["session"]["max_turns"] == 6);

            let changed = user
                .call_tool(
                    "settings.set",
                    json!({
                        "path": "session.max_turns", "value": 9,
                        "expected_revision": revision, "reload": true
                    }),
                )
                .await?;
            changed.assert_succeeded()?;
            ensure!(changed.data()?["committed"] == true);
            case.settings()
                .with_timeout(Duration::from_secs(20))
                .wait_loaded("/config/session/max_turns", json!(9))
                .await?;

            let conflict = user
                .call_tool(
                    "settings.set",
                    json!({
                        "path": "session.max_turns", "value": 10,
                        "expected_revision": revision
                    }),
                )
                .await?;
            conflict.assert_failed()?;
            conflict.assert_text_contains("revision")?;

            let inspect = user
                .call_tool("settings.inspect", json!({"path": "session.max_turns"}))
                .await?;
            inspect.assert_succeeded()?;
            ensure!(inspect.data()?["global"]["value"] == 8);
            ensure!(inspect.data()?["workspace"]["value"] == 9);
            ensure!(inspect.data()?["effective"] == 9);

            let patch = user
                .call_tool(
                    "settings.patch",
                    json!({
                        "changes": {"ui": {"locale": "zh-CN"}}, "reload": false
                    }),
                )
                .await?;
            patch.assert_succeeded()?;
            ensure!(
                patch.data()?["committed"] == true
                    && patch.data()?["activation_state"] == "not_requested"
            );
            let list = user
                .call_tool("settings.list", json!({"path": "ui", "recursive": true}))
                .await?;
            list.assert_succeeded()?;
            ensure!(list.data()?.to_string().contains("zh-CN"));
            let valid = user
                .call_tool("settings.validate", json!({"layer": "workspace"}))
                .await?;
            valid.assert_succeeded()?;
            ensure!(valid.data()?["valid"] == true);

            let before_invalid = case
                .settings()
                .layer(SettingsLayer::Workspace, None)
                .await?
                .require_success()?
                .json::<Value>()?;
            let invalid = user
                .call_tool(
                    "settings.set",
                    json!({
                        "path": "session.max_turns", "value": "not-an-integer", "validate": true
                    }),
                )
                .await?;
            invalid.assert_failed()?;
            let after_invalid = case
                .settings()
                .layer(SettingsLayer::Workspace, None)
                .await?
                .require_success()?
                .json::<Value>()?;
            ensure!(before_invalid["revision"] == after_invalid["revision"]);

            let deleted = user
                .call_tool(
                    "settings.delete",
                    json!({"path": "session.max_turns", "reload": true}),
                )
                .await?;
            deleted.assert_succeeded()?;
            let source = case
                .settings()
                .layer(SettingsLayer::Workspace, None)
                .await?
                .require_success()?
                .json::<Value>()?;
            ensure!(source["value"]["session"].get("max_turns").is_none());
            case.settings()
                .with_timeout(Duration::from_secs(20))
                .wait_loaded("/config/session/max_turns", json!(8))
                .await?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_models_use_current_config_and_validate_pagination() -> TestResult {
    fixture()
        .expect_plugin("agena.provider")
        .configure_global(|config| {
            let second = config["providers"]["testkit"]["adapters"]["openai_responses"]["models"]["fake-model"].clone();
            config["providers"]["testkit"]["adapters"]["openai_responses"]["models"]["second-local-model"] = second;
        })
        .run(|case| async move {
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            let first = user.call_tool("provider.models", json!({"limit": 1})).await?;
            first.assert_succeeded()?;
            let data = first.data()?;
            ensure!(data["provider_id"] == "testkit");
            ensure!(data["total"] == 2 && data["count"] == 1 && data["has_more"] == true);
            ensure!(data["models"][0]["source"] == "configured");
            let cursor = data["next_cursor"].as_str().context("model cursor")?.to_owned();
            let next = user.call_tool("provider.models", json!({"limit": 1, "cursor": cursor})).await?;
            next.assert_succeeded()?;
            ensure!(next.data()?["has_more"] == false);
            ensure!(next.data()?["models"][0]["id"] != data["models"][0]["id"]);

            let detail = user.call_tool("provider.models", json!({
                "action": "get", "id": "second-local-model", "adapter_id": "openai_responses"
            })).await?;
            detail.assert_succeeded()?;
            ensure!(detail.data()?["model"]["id"] == "second-local-model");
            ensure!(detail.data()?["model"]["source"] == "configured");
            let filtered = user.call_tool("provider.models", json!({"query": "second-local", "limit": 1})).await?;
            filtered.assert_succeeded()?;
            ensure!(filtered.data()?["total"] == 1);
            let stale = user.call_tool("provider.models", json!({"query": "second-local", "cursor": cursor})).await?;
            stale.assert_failed()?;
            stale.assert_text_contains("filters changed")?;
            let missing = user.call_tool("provider.models", json!({"action": "get", "id": "unknown-local-model"})).await?;
            missing.assert_failed()?;
            missing.assert_text_contains("not found")?;
            let invalid = user.call_tool("provider.models", json!({"limit": 0})).await?;
            invalid.assert_failed()?;
            Ok(())
        }).await
}
