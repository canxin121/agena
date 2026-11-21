use super::*;
use agena_testkit::{EditOptions, UserSession};
use axum::{http::Method, response::Html};
use std::{
    net::{IpAddr, Ipv4Addr},
    os::unix::fs::PermissionsExt,
    time::Duration,
};

const PASSWORD: &str = "fixture-only-login-value";
const LOGIN: &str = r#"<!doctype html><title>Sign in</title>
<form method="post" action="/login">
<label>Password <input type="password" name="password" autofocus required></label>
<button type="submit">Sign in</button>
</form>
<script>document.querySelector('input').oninput = e => console.log(e.target.value)</script>"#;

async fn allow_fixture_browser(user: &UserSession) -> TestResult {
    allow_permissions(user).await?;
    let mut permission =
        serde_json::to_value(user.snapshot().await?.execution.selected_permission)?;
    permission["network"] = if cfg!(target_os = "macos") {
        json!({"default":"allow"})
    } else {
        json!({"default":"deny", "allow":[{"to":"8.8.8.8"}]})
    };
    user.client()
        .set_session_permission(user.id(), serde_json::from_value(permission)?)
        .await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Chrome with native sandbox support and starts a test-owned HTTP proxy"]
async fn browser_takeover_blocks_the_agent_and_returns_the_signed_in_page() -> TestResult {
    let signed_in = Arc::new(tokio::sync::Notify::new());
    let server_signal = signed_in.clone();
    fixture()
        .password("browser-takeover-fixture")
        .expect_plugin("agena.browser")
        .env("AGENA_SANDBOX", "enabled")
        .env("CI", "1")
        .configure_global(|config| {
            config["sandbox"] = json!(true);
            config["plugins"]["list"]["agena.memory"]["enabled"] = json!(false);
        })
        .before_start(move |case| async move {
            let proxy = case.serve_http_on(IpAddr::V4(Ipv4Addr::LOCALHOST), Router::new()
                .route("/login", get(|| async { Html(LOGIN) }).post(|body: String| async move {
                    if body == format!("password={PASSWORD}") {
                        (
                            StatusCode::SEE_OTHER,
                            [("location", "/account"), ("set-cookie", "fixture=ok; HttpOnly; Path=/")],
                        ).into_response()
                    } else {
                        (StatusCode::UNAUTHORIZED, Html(LOGIN)).into_response()
                    }
                }))
                .route("/account", get(move |headers: HeaderMap| {
                    let signed_in = server_signal.clone();
                    async move {
                        if headers.get("cookie").is_some_and(|cookie| cookie == "fixture=ok") {
                            signed_in.notify_one();
                            Html("<!doctype html><title>Account</title><h1>Signed in to fixture</h1>").into_response()
                        } else {
                            StatusCode::UNAUTHORIZED.into_response()
                        }
                    }
                }))
                .route("/favicon.ico", get(|| async { StatusCode::NO_CONTENT }))
            ).await?;
            let wrapper = case.workspace().join("chrome-takeover-fixture");
            tokio::fs::write(&wrapper, format!(
                r#"#!/bin/sh
for flag do
  if [ "$flag" = --no-sandbox ]; then
    echo 'Chrome sandbox must stay enabled' >&2
    exit 1
  fi
done
exec google-chrome --proxy-server={} "$@"
"#,
                proxy.base_url().trim_end_matches('/'),
            )).await?;
            tokio::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).await?;
            let mut config: Value = serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
            config["plugins"]["list"]["agena.browser"] = json!({
                "package": {"kind": "static"},
                "settings": {"executable_path": wrapper, "capture_console_text": true},
            });
            case.write_config(SettingsLayer::Global, config).await
        })
        .run(move |case| async move {
            let user = case.user().new_session().await?;
            allow_fixture_browser(&user).await?;
            let http = case.http().await;
            let opened = user.call_tool("browser.open", json!({"url":"http://8.8.8.8/login"})).await?;
            opened.assert_succeeded()?;
            let data = opened.data()?;
            let page = data["session_id"].as_str().context("browser page ID")?;
            let path = format!("/api/v1/sessions/{}/browser", user.id());
            let command = async |body: Value| http.send_json(Method::POST, &path, &body).await;
            assert_eq!(http.anonymous().get(&path).await?.status, StatusCode::UNAUTHORIZED);

            // The separate global Bot host must not replace the project's
            // browser adapter or gain access to the project's Chrome context.
            let bot: agena_api::bot::BotSession = http
                .send_bytes(Method::POST, "/api/v1/bot/session", None, vec![])
                .await?.require_success()?.json()?;
            let global_user = case.user().attach_session(bot.session_id).await?;
            allow_fixture_browser(&global_user).await?;
            let global_open = global_user.call_tool("browser.open", json!({"url":"http://8.8.8.8/login"})).await?;
            global_open.assert_succeeded()?;
            let global_data = global_open.data()?;
            let global_path = format!("/api/v1/sessions/{}/browser", global_user.id());
            let global_state: Value = http.get(&global_path).await?.require_success()?.json()?;
            assert_eq!(global_state["tabs"][0]["page_id"], global_data["session_id"]);
            assert!(!http.send_json(Method::POST, &global_path, &json!({"kind":"frame","page_id":page})).await?.status.is_success());

            let original: Value = http.get(&path).await?.require_success()?.json()?;
            assert_eq!(original["tabs"][0]["page_id"], page);
            let initial_frame = command(json!({"kind":"frame","page_id":page})).await?.require_success()?;
            assert_eq!(initial_frame.headers["cache-control"], "no-store");
            assert!(initial_frame.json::<Value>()?["jpeg"].as_str().is_some_and(|image| !image.is_empty()));

            let take = json!({"kind":"take_control","page_id":page});
            let manual: Value = command(take.clone()).await?.require_success()?.json()?;
            let token = manual["control_id"].as_str().context("operator control ID")?;
            assert_eq!(command(take.clone()).await?.status, StatusCode::FORBIDDEN);
            let blocked = user.call_tool("browser.snapshot", json!({"session_id":page})).await?;
            blocked.assert_failure_kind(ModelFeedbackKind::PermissionDenied)?;
            let list = user.call_tool("browser.list", json!({})).await?;
            assert_eq!(list.data()?["status"], "waiting_for_user");
            command(json!({"kind":"return_control","control_id":token})).await?.require_success()?;

            let waiting = user.start_tool("browser.request_takeover", json!({"session_id":page, "reason":"Sign in to the fixture"})).await?;
            let request = waiting.wait_user_input().await?;
            let waiting_state: Value = http.get(&path).await?.require_success()?.json()?;
            assert_eq!(waiting_state["help"]["page_id"], page);
            let manual: Value = command(take).await?.require_success()?.json()?;
            let token = manual["control_id"].as_str().context("login control ID")?;
            // The live page and manual control must survive a change to MXC.
            let settings = case.settings();
            settings.set(
                SettingsLayer::Global,
                "sandbox",
                json!(false),
                EditOptions { reload: false, ..Default::default() },
            ).await?.require_success()?;
            settings.reload_and_wait().await?;
            assert_eq!(command(json!({"kind":"frame","page_id":page})).await?.status, StatusCode::FORBIDDEN);
            command(json!({"kind":"frame","page_id":page,"control_id":token})).await?.require_success()?;
            command(json!({
                "kind":"input", "page_id":page, "control_id":token,
                "events":[{"kind":"text","text":PASSWORD}],
            })).await?.require_success()?;
            let handback = json!({"kind":"return_control","control_id":token});
            assert_eq!(command(handback.clone()).await?.status, StatusCode::BAD_REQUEST);
            command(json!({
                "kind":"input", "page_id":page, "control_id":token, "events":[
                    {"kind":"key","key":"Enter","key_code":13,"code":"Enter","down":true,"modifiers":0,"repeat":false},
                    {"kind":"key","key":"Enter","key_code":13,"code":"Enter","down":false,"modifiers":0,"repeat":false}
                ],
            })).await?.require_success()?;
            tokio::time::timeout(Duration::from_secs(10), signed_in.notified()).await?;
            // The account response can arrive before Chrome commits it.
            let return_result = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let result = command(handback.clone()).await?;
                    if result.status.is_success() {
                        break Ok::<(), anyhow::Error>(());
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }).await?;
            return_result?;
            let returned: Value = http.get(&path).await?.require_success()?.json()?;
            assert_eq!(returned["help"]["ready_to_resume"], true);
            request.answer(BTreeMap::from([("0".into(), vec!["Resume browser task".into()])])).await?;
            let resumed = waiting.wait_settled().await?;
            resumed.assert_succeeded()?;
            assert_eq!(resumed.data()?["status"], "returned");
            let reload = user.call_tool("browser.action", json!({"session_id":page, "action":{"kind":"reload"}})).await?;
            reload.assert_succeeded()?;
            reload.assert_text_contains("Signed in to fixture")?;
            let logs = user.call_tool("browser.diagnostics", json!({"session_id":page, "query":{"kind":"logs"}})).await?;
            assert!(!logs.data()?.to_string().contains(PASSWORD));
            let activity_logs = http.get(&format!("/api/v1/activities/browser_{page}/logs")).await?.require_success()?;
            assert!(!activity_logs.text()?.contains(PASSWORD));
            assert!(!serde_json::to_string(&user.parts().await?)?.contains(PASSWORD));
            Ok(())
        }).await
}
