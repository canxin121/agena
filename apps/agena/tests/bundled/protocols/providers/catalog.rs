use agena_testkit::{EditOptions, SettingsLayer, TestAgenaBuilder, TestResult};
use anyhow::{Context, ensure};
use axum::http::Method;
use serde_json::{Value, json};

use super::{API_KEY, Vendor, peer::*};

fn catalog_fixture(peer: &Peer) -> TestAgenaBuilder {
    scripted(peer).configure_global(|config| {
        let model = config["providers"]["testkit"]["adapters"]["openai_responses"]["models"]["fake-model"].clone();
        let mut image = model.clone();
        image["description"] = json!("Drawing and visual illustration");
        image["output_modalities"] = json!(["image"]);
        config["providers"]["testkit"]["adapters"]["openai_responses"]["models"]["image-route"] = image;
        config["providers"]["testkit"]["adapters"]["openai_responses"]["models"]["shared-model"] = model.clone();
        config["providers"]["testkit"]["adapters"]["gemini"] =
            json!({"enabled":true,"model_discovery":"configured_only","models":{"shared-model":model}});
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_catalog_disambiguates_routes_and_searches_full_metadata() -> TestResult {
    let peer = Peer::default();
    catalog_fixture(&peer)
        .run(move |case| async move {
            let user = session(&case).await?;
            let filtered = user
                .call_tool("provider.models", json!({"query":"DRAWING"}))
                .await?;
            filtered.assert_succeeded()?;
            ensure!(filtered.data()?["total"] == 1);
            ensure!(filtered.data()?["models"][0]["id"] == "image-route");
            ensure!(filtered.data()?["models"][0].get("metadata").is_none());
            let detailed = user
                .call_tool(
                    "provider.models",
                    json!({"action":"get","id":"image-route"}),
                )
                .await?;
            detailed.assert_succeeded()?;
            ensure!(detailed.data()?["model"]["metadata"]["output_modalities"][0] == "image");
            let ambiguous = user
                .call_tool(
                    "provider.models",
                    json!({"action":"get","id":"shared-model"}),
                )
                .await?;
            ambiguous.assert_failed()?;
            ambiguous.assert_text_contains("multiple routes")?;
            let selected = user
                .call_tool(
                    "provider.models",
                    json!({"action":"get","id":"shared-model","adapter_id":"gemini"}),
                )
                .await?;
            selected.assert_succeeded()?;
            ensure!(selected.data()?["model"]["adapter_id"] == "gemini");
            ensure!(selected.data()?["model"]["source"] == "configured");
            ensure!(
                peer.requests().is_empty(),
                "configured-only catalog performed a vendor request"
            );
            peer.assert_finished()
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_catalog_rejects_invalid_inputs_and_cursor_from_another_provider() -> TestResult {
    let peer = Peer::default();
    catalog_fixture(&peer)
        .run(move |case| async move {
            let user = session(&case).await?;
            for input in [
                json!({"limit":0}),
                json!({"limit":201}),
                json!({"adapter_id":"unknown"}),
                json!({"action":"get"}),
                json!({"id":"unused"}),
            ] {
                user.call_tool("provider.models", input)
                    .await?
                    .assert_failed()?;
            }
            let first = user
                .call_tool("provider.models", json!({"limit":1}))
                .await?;
            first.assert_succeeded()?;
            let cursor = first.data()?["next_cursor"]
                .as_str()
                .context("catalog cursor")?
                .to_owned();
            let changed = user
                .call_tool("provider.models", json!({"query":"image","cursor":cursor}))
                .await?;
            changed.assert_failed()?;
            changed.assert_text_contains("filters changed")?;
            let config: Value =
                serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
            case.settings()
                .set(
                    SettingsLayer::Global,
                    "providers.second",
                    config["providers"]["testkit"].clone(),
                    EditOptions::default(),
                )
                .await?
                .require_success()?;
            user.client().update_session_selection(user.id(), serde_json::from_value(json!({
            "model":{"provider_id":"second","adapter_id":"openai_responses","model_id":"fake-model"}
        }))?).await?;
            let stale = user
                .call_tool("provider.models", json!({"limit":1,"cursor":cursor}))
                .await?;
            stale.assert_failed()?;
            ensure!(
                stale.output.to_string().contains("cursor"),
                "{}",
                stale.output
            );
            peer.assert_finished()
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_tools_follow_live_endpoint_changes_and_custom_headers() -> TestResult {
    let peer = Peer::default();
    scripted(&peer)
        .configure_global(|config| {
            config["providers"]["testkit"]["adapters"]["openai_responses"]["extra_headers"] =
                json!({"x-gateway-token":"custom-provider-secret"});
            for adapter in ["anthropic", "gemini"] {
                config["providers"]["testkit"]["adapters"][adapter] = json!({
                    "enabled":true,"model_discovery":"configured_only","models":{},
                    "extra_headers":{"x-gateway-token":"custom-provider-secret"}
                });
            }
        })
        .run(move |case| async move {
            let user = session(&case).await?;
            let selected_model = case.model().model_id().to_owned();
            for prefix in ["first", "second"] {
                case.settings()
                    .set(
                        SettingsLayer::Global,
                        "providers.testkit.auth.base_url",
                        json!(peer.url(&format!("/{prefix}/v1"))),
                        EditOptions::default(),
                    )
                    .await?
                    .require_success()?;
                for vendor in [Vendor::ChatGpt, Vendor::Claude, Vendor::Gemini] {
                    let model = if prefix == "first" {
                        selected_model.as_str()
                    } else {
                        "team/override"
                    };
                    peer.json(
                        Method::POST,
                        format!("/{prefix}{}", vendor.path()),
                        vendor.response(model),
                    );
                    let mut input = json!({"prompt":"live-provider-connection"});
                    if prefix == "second" {
                        input["model"] = json!(model);
                    }
                    let result = user.call_tool(vendor.tool(), input).await?;
                    peer.assert_finished()?;
                    result.assert_succeeded()?;
                    ensure!(result.data()?["model"] == model);
                    ensure!(
                        result.data()?["text"]
                            == format!("{}-local-provider-result", vendor.name())
                    );
                    let request = peer.last()?;
                    assert_request(&request, vendor, user.id())?;
                    ensure!(request.headers["x-gateway-token"] == "custom-provider-secret");
                    ensure!(request.json()?["model"] == model);
                    for secret in [API_KEY, "custom-provider-secret"] {
                        ensure!(
                            !result.output.to_string().contains(secret),
                            "credential escaped provider transport"
                        );
                    }
                }
            }
            peer.assert_finished()
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn loaded_provider_manifests_expose_only_current_public_contracts() -> TestResult {
    let peer = Peer::default();
    scripted(&peer)
        .run(move |case| async move {
            let user = session(&case).await?;
            for id in [
                "agena.provider",
                "agena.chatgpt",
                "agena.claude",
                "agena.gemini",
                "agena.session",
            ] {
                let manifest = user
                    .client()
                    .plugin_inspect(id)
                    .await?
                    .plugin
                    .manifest
                    .context("loaded manifest")?;
                if id == "agena.provider" {
                    let models = manifest
                        .tools
                        .iter()
                        .find(|tool| tool.name == "models")
                        .context("provider models tool")?;
                    let fields = models.contract.input_schema["properties"]
                        .as_object()
                        .context("model input schema")?;
                    for obsolete in ["model", "prompt", "path", "api_key", "base_url"] {
                        ensure!(
                            !fields.contains_key(obsolete),
                            "model discovery exposed unrelated input {obsolete}"
                        );
                    }
                    continue;
                }
                ensure!(
                    !manifest.tools.iter().any(|tool| tool.name == "models"),
                    "obsolete vendor/session discovery alias returned"
                );
                if id == "agena.session" {
                    continue;
                }
                let settings = serde_json::to_string(&manifest.settings)?;
                for legacy in ["base_url", "api_key_env", "timeout_secs", "image_model"] {
                    ensure!(
                        !settings.contains(legacy),
                        "connection settings leaked back into {id}"
                    );
                }
                for tool in manifest.tools {
                    ensure!(!tool.name.starts_with("cloud_"));
                    let has_model = tool.contract.input_schema["properties"]
                        .get("model")
                        .is_some();
                    ensure!(
                        has_model
                            != matches!(
                                tool.name.as_str(),
                                "file_upload" | "file_status" | "file_delete"
                            ),
                        "incorrect session-model override contract on {id}.{}",
                        tool.name
                    );
                }
            }
            peer.assert_finished()
        })
        .await
}
