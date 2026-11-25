use agena_testkit::TestResult;
use anyhow::ensure;
use serde_json::{Value, json};

use super::{BundledBehaviorCase, allow_permissions, fixture, fixture_without_memory};

pub(super) const CASES: &[BundledBehaviorCase] = &[
    (
        "agena.slack",
        bundled_chat_channels_offer_verified_callback_routes,
    ),
    (
        "agena.feishu",
        bundled_chat_channels_offer_verified_callback_routes,
    ),
    (
        "agena.discord",
        bundled_chat_channels_offer_verified_callback_routes,
    ),
    (
        "agena.whatsapp",
        bundled_chat_channels_offer_verified_callback_routes,
    ),
    (
        "agena.telegram",
        telegram_bot_catalog_opens_without_a_platform_credential,
    ),
    ("agena.desktop", desktop_requires_operator_configuration),
];

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn telegram_bot_catalog_opens_without_a_platform_credential() -> TestResult {
    fixture()
        .expect_plugin("agena.telegram")
        .env_remove("AGENA_TELEGRAM_BOT_TOKEN")
        .run(|case| async move {
            let catalog: Value = case
                .http()
                .await
                .send_json(
                    reqwest::Method::POST,
                    "/api/v1/plugins/agena.telegram/commands/list_bots/invoke",
                    &json!({}),
                )
                .await?
                .require_success()?
                .json()?;
            assert_eq!(catalog["result"]["output"]["bots"][0]["id"], "default");
            assert_eq!(catalog["result"]["output"]["bots"][0]["has_token"], false);
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bundled_chat_channels_offer_verified_callback_routes() -> TestResult {
    fixture()
        .env("AGENA_FIXTURE_CHAT_KEY", "fixture-secret")
        .configure_global(|config| {
            for (plugin, account) in [
                ("agena.slack", json!({
                    "name":"fixture","team_id":"T123","app_id":"A123","bot_user_id":"U123",
                    "token_env":"AGENA_FIXTURE_CHAT_KEY","signing_secret_env":"AGENA_FIXTURE_CHAT_KEY",
                })),
                ("agena.feishu", json!({
                    "name":"fixture","tenant_key":"t_fixture","app_id":"cli_fixture","bot_open_id":"ou_fixture",
                    "app_secret_env":"AGENA_FIXTURE_CHAT_KEY","encrypt_key_env":"AGENA_FIXTURE_CHAT_KEY",
                    "verification_token_env":"AGENA_FIXTURE_CHAT_KEY",
                })),
                ("agena.whatsapp", json!({
                    "name":"fixture","business_account_id":"123","phone_number_id":"456",
                    "token_env":"AGENA_FIXTURE_CHAT_KEY","app_secret_env":"AGENA_FIXTURE_CHAT_KEY",
                    "verification_token_env":"AGENA_FIXTURE_CHAT_KEY",
                })),
            ] {
                config["plugins"]["list"][plugin] = json!({
                    "package":{"kind":"static"},"settings":{"accounts":[account]},
                });
            }
        })
        .run(|case| async move {
            let bot = case.client().await.ensure_bot_session().await?.session_id;
            let integrations = case.client().await.integrations(bot).await?;
            for provider in ["agena.slack", "agena.feishu", "agena.discord", "agena.whatsapp"] {
                ensure!(integrations.providers.iter().any(|entry| {
                    entry.plugin_id == provider && entry.adapter.channel_context_version == Some(1)
                }), "missing channel provider {provider}");
            }
            let http = case.http().await;
            let slack = http.send_json(reqwest::Method::POST, "/hooks/chat/slack/fixture", &json!({
                "type":"url_verification","challenge":"challenge",
            })).await?;
            assert_eq!(slack.status, 401);
            let feishu: Value = http.send_json(reqwest::Method::POST, "/hooks/chat/feishu/fixture", &json!({
                "type":"url_verification","challenge":"feishu-challenge","token":"fixture-secret",
            })).await?.require_success()?.json()?;
            assert_eq!(feishu["challenge"], "feishu-challenge");
            let whatsapp = http.get(
                "/hooks/chat/whatsapp/fixture?hub.mode=subscribe&hub.verify_token=fixture-secret&hub.challenge=123456",
            ).await?.require_success()?;
            assert_eq!(whatsapp.body, b"123456");
            assert_eq!(http.get(
                "/hooks/chat/whatsapp/fixture?hub.mode=subscribe&hub.verify_token=invalid&hub.challenge=123456",
            ).await?.status, 401);
            assert_eq!(http.send_json(
                reqwest::Method::POST, "/hooks/chat/discord/fixture", &json!({}),
            ).await?.status, 404);
            Ok(())
        }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn desktop_requires_operator_configuration() -> TestResult {
    fixture_without_memory()
        .expect_plugin("agena.desktop")
        .env_remove("DISPLAY")
        .run(|case| async move {
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            let result = user.call_tool("desktop.windows", json!({})).await?;
            result.assert_failed()?;
            result.assert_text_contains("allowed_applications")?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Cua Driver and a fixture-owned desktop window"]
async fn desktop_driver_operates_fixture_window() -> TestResult {
    let application = std::env::var("AGENA_TEST_DESKTOP_APP")?;
    let title = std::env::var("AGENA_TEST_DESKTOP_TITLE")?;
    let mut fixture = fixture_without_memory()
        .expect_plugin("agena.desktop")
        .configure_global(move |config| {
            let model = &mut config["providers"]["testkit"]["adapters"]["openai_responses"]
                ["models"]["fake-model"];
            model["input"] = json!(["text", "image"]);
            config["plugins"]["list"]["agena.desktop"] = json!({
                "package": {"kind":"static"},
                "settings":{"allowed_applications":[application],"allow_input":true}
            });
        });
    for name in [
        "AGENA_DESKTOP_DRIVER",
        "DISPLAY",
        "XAUTHORITY",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "XDG_SESSION_TYPE",
        "XDG_CURRENT_DESKTOP",
        "DBUS_SESSION_BUS_ADDRESS",
    ] {
        if let Some(value) = std::env::var_os(name) {
            fixture = fixture.env(name, value);
        }
    }
    fixture
        .run(|case| async move {
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            let result = user.call_tool("desktop.windows", json!({})).await?;
            result.assert_succeeded()?;
            let window_id = result.data()?["windows"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|window| window["title"] == title)
                .and_then(|window| window["window_id"].as_str())
                .ok_or_else(|| {
                    anyhow::anyhow!("desktop fixture window {title:?} was not listed")
                })?;

            let screenshot = user
                .call_tool("desktop.screenshot", json!({"window_id":window_id}))
                .await?;
            screenshot.assert_succeeded()?;
            let data = screenshot.data()?;
            ensure!(data["image_width"].as_u64().is_some_and(|width| width > 0));
            ensure!(
                data["image_height"]
                    .as_u64()
                    .is_some_and(|height| height > 0)
            );
            for action in [
                json!({"kind":"click", "x":80,"y":50}),
                json!({"kind":"scroll", "x":80,"y":50,"direction":"down","steps":1}),
                json!({"kind":"type", "text":"a"}),
                json!({"kind":"key", "key":"Enter"}),
            ] {
                user.call_tool(
                    "desktop.action",
                    json!({"window_id":window_id,"action":action}),
                )
                .await?
                .assert_succeeded()?;
            }

            let other = case.user().new_session().await?;
            allow_permissions(&other).await?;
            let copied = other
                .call_tool("desktop.screenshot", json!({"window_id":window_id}))
                .await?;
            copied.assert_failed()?;
            copied.assert_text_contains("belongs to another conversation")?;
            Ok(())
        })
        .await
}
