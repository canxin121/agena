use super::*;
use agena_testkit::{EditOptions, Reply, RequestMatch, UserSession};
use axum::{http::Method, response::Html};
use base64::Engine as _;
use std::{
    net::{IpAddr, Ipv4Addr},
    os::unix::fs::PermissionsExt,
    time::Duration,
};

const PASSWORD: &str = "fixture-only-login-value";
const SEARCH: &str = r#"<!doctype html><title>Search fixture</title>
<form action="/login"><label>Search <input id="query" name="q"></label><button>Search</button></form>"#;
const LOGIN: &str = r#"<!doctype html><title>Sign in</title>
<style>
html { background: rgb(36, 112, 176); }
#frame-marker { position: fixed; inset: 65% 0 0 65%; background: rgb(216, 120, 48); }
</style>
<div id="frame-marker" aria-hidden="true"></div>
<h1>Human verification required</h1>
<form method="post" action="/login">
<label>Password <input type="password" name="password" autofocus required></label>
<button type="submit">Sign in</button>
</form>
<script>document.querySelector('input').oninput = e => console.log(e.target.value)</script>"#;

// This test already requires Chrome. Use its canvas to decode the wire JPEG
// without adding a Cargo dependency or requiring another executable.
async fn assert_frame_pixels(
    user: &UserSession,
    page: &str,
    label: &str,
    frame: &Value,
) -> TestResult {
    let jpeg = frame["jpeg"]
        .as_str()
        .with_context(|| format!("{label}: frame JPEG"))?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(jpeg)?;
    assert!(bytes.starts_with(&[0xff, 0xd8]), "{label}: expected a JPEG");
    let pixels = imagesize::blob_size(&bytes)?;
    assert_eq!(frame["width"], pixels.width, "{label}");
    assert_eq!(frame["height"], pixels.height, "{label}");
    let expression = format!(
        r#"(async () => {{
            const img = new Image();
            img.src = 'data:image/jpeg;base64,' + {};
            await img.decode();
            const canvas = document.createElement('canvas');
            canvas.width = img.naturalWidth;
            canvas.height = img.naturalHeight;
            const ctx = canvas.getContext('2d');
            ctx.drawImage(img, 0, 0);
            return [[0.75, 0.5], [0.5, 0.75], [0.85, 0.85]].map(([x, y]) =>
                Array.from(ctx.getImageData(Math.floor(x * canvas.width),
                    Math.floor(y * canvas.height), 1, 1).data).slice(0, 3));
        }})()"#,
        serde_json::to_string(jpeg)?,
    );
    ensure!(
        expression.len() <= 32_768,
        "{label}: JPEG exceeds the browser.evaluate input limit"
    );
    let result = user
        .call_tool(
            "browser.evaluate",
            json!({
                "session_id": page, "expression": expression, "timeout_ms": 5000,
            }),
        )
        .await?;
    result.assert_succeeded()?;
    let decoded = result.data()?;
    assert_eq!(decoded["status"], "ok", "{label}: {decoded}");
    let samples: [[u8; 3]; 3] = serde_json::from_value(decoded["value"].clone())?;
    // Stay away from the top-left form and allow for lossy JPEG encoding.
    for (actual, expected) in
        samples
            .into_iter()
            .zip([[36, 112, 176], [36, 112, 176], [216, 120, 48]])
    {
        assert!(
            actual
                .into_iter()
                .zip(expected)
                .all(|(a, e)| a.abs_diff(e) <= 16),
            "{label}: sampled {actual:?}, expected {expected:?}"
        );
    }
    Ok(())
}

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
                .route("/search", get(|| async { Html(SEARCH) }))
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
# Keep screenshot data small enough for the browser.evaluate expression limit.
exec google-chrome --proxy-server={} "$@" --window-size=800,600 --force-device-scale-factor=1
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
            let normal = json!({"kind":"frame","page_id":page});
            let initial_frame = command(normal.clone()).await?.require_success()?;
            assert_eq!(initial_frame.headers["cache-control"], "no-store");
            let initial: Value = initial_frame.json()?;

            let thumbnail = json!({"kind":"frame","page_id":page,"thumbnail":true});
            // Match simultaneous sidebar and main-view requests, then check
            // that the next full frame is also intact after both have finished.
            let (preview, overlap, following) = tokio::time::timeout(Duration::from_secs(10), async {
                let (preview, overlap) = tokio::try_join!(command(thumbnail.clone()), command(normal.clone()))?;
                let following = command(normal).await?;
                Ok::<_, anyhow::Error>((preview, overlap, following))
            }).await.context("overlapping and sequential frame requests")??;
            let preview: Value = preview.require_success()?.json()?;
            let overlap: Value = overlap.require_success()?.json()?;
            let following: Value = following.require_success()?.json()?;
            assert!(preview["width"].as_u64().is_some_and(|width| (1..=192).contains(&width)));
            assert!(preview["height"].as_u64().is_some_and(|height| (1..=108).contains(&height)));
            assert!(initial["width"].as_u64().is_some_and(|width| width > 192));
            assert!(initial["height"].as_u64().is_some_and(|height| height > 108));
            for (label, frame) in [("overlapping full frame", &overlap), ("subsequent full frame", &following)] {
                assert_eq!(frame["width"], initial["width"], "{label}");
                assert_eq!(frame["height"], initial["height"], "{label}");
            }
            for (label, frame) in [
                ("initial full frame", &initial),
                ("thumbnail", &preview),
                ("overlapping full frame", &overlap),
                ("subsequent full frame", &following),
            ] {
                assert_frame_pixels(&user, page, label, frame).await?;
            }
            let take = json!({"kind":"take_control","page_id":page});
            let manual: Value = command(take.clone()).await?.require_success()?.json()?;
            let token = manual["control_id"].as_str().context("operator control ID")?;
            assert_eq!(command(take.clone()).await?.status, StatusCode::FORBIDDEN);
            assert_eq!(command(thumbnail).await?.status, StatusCode::FORBIDDEN);
            let blocked = user.call_tool("browser.snapshot", json!({"session_id":page})).await?;
            blocked.assert_failure_kind(ModelFeedbackKind::PermissionDenied)?;
            let list = user.call_tool("browser.list", json!({})).await?;
            assert_eq!(list.data()?["status"], "waiting_for_user");
            command(json!({"kind":"return_control","control_id":token})).await?.require_success()?;

            let search = user.call_tool("browser.action", json!({
                "session_id":page, "action":{"kind":"navigate","url":"http://8.8.8.8/search"},
            })).await?;
            search.assert_succeeded()?;

            let request_text = "Find the result for Agena in the search fixture.";
            let model = case.model();
            let read_submit = model
                .expect(RequestMatch::user_text(request_text))
                .reply(Reply::function_call("submit-help", "tools_help", json!({"tool":"browser.type"})));
            let submit = model
                .expect(RequestMatch::tool_output("submit-help"))
                .after(&read_submit)
                .check(|request| {
                    let text = request.tool_output_text("submit-help").context("browser.type contract")?;
                    ensure!(text.contains("verification") && text.contains("browser.request_takeover"), "{text}");
                    Ok(())
                })
                .reply(Reply::function_call("submit-search", "tools_call", json!({
                    "tool":"browser.type", "input":{"session_id":page,"selector":"#query","text":"Agena","press_enter":true},
                })));
            let read_takeover = model
                .expect(RequestMatch::tool_output("submit-search"))
                .after(&submit)
                .check(|request| {
                    let text = request.tool_output_text("submit-search").context("submitted page")?;
                    ensure!(text.contains("Human verification required"), "{text}");
                    ensure!(text.contains("takeover_hint") && text.contains("browser.request_takeover"), "{text}");
                    Ok(())
                })
                .reply(Reply::function_call("takeover-help", "tools_help", json!({"tool":"browser.request_takeover"})));
            let take_over = model
                .expect(RequestMatch::tool_output("takeover-help"))
                .after(&read_takeover)
                .reply(Reply::function_call("ask-for-browser-help", "tools_call", json!({
                    "tool":"browser.request_takeover",
                    "input":{"session_id":page,"reason":"Complete the human verification to view the search result."},
                })));
            let expected_page = page.to_owned();
            let read_result = model
                .expect(RequestMatch::tool_output("ask-for-browser-help"))
                .after(&take_over)
                .check(move |request| {
                    let text = request.tool_output_text("ask-for-browser-help").context("browser handback")?;
                    ensure!(text.contains("returned") && text.contains(&expected_page), "{text}");
                    Ok(())
                })
                .reply(Reply::tool_call("read-signed-in-page", "browser.snapshot", json!({"session_id":page})));
            model
                .expect(RequestMatch::tool_output("read-signed-in-page"))
                .after(&read_result)
                .check(|request| {
                    let text = request.tool_output_text("read-signed-in-page").context("result after handback")?;
                    ensure!(text.contains("Signed in to fixture"), "{text}");
                    Ok(())
                })
                .reply(Reply::text("Read the result after verification."));

            let waiting = user.send(request_text).await?;
            let request = waiting.wait_user_input().await?;
            let waiting_state: Value = http.get(&path).await?.require_success()?.json()?;
            assert_eq!(waiting_state["help"]["page_id"], page);
            assert_eq!(waiting_state["help"]["reason"], "Complete the human verification to view the search result.");
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
            resumed.assert_assistant_text("Read the result after verification.")?;
            let reload = user.call_tool("browser.action", json!({"session_id":page, "action":{"kind":"reload"}})).await?;
            reload.assert_succeeded()?;
            reload.assert_text_contains("Signed in to fixture")?;
            let logs = user.call_tool("browser.diagnostics", json!({"session_id":page, "query":{"kind":"logs"}})).await?;
            assert!(!logs.data()?.to_string().contains(PASSWORD));
            let parts = user.parts().await?;
            assert!(parts.iter().any(|part| part.presentation.as_ref().is_some_and(|view| {
                view.blocks.iter().any(|block| matches!(block, agena_domain::ViewBlock::BrowserPage {
                    page_id, ..
                } if page_id == &page))
            })));
            assert!(!serde_json::to_string(&parts)?.contains(PASSWORD));
            Ok(())
        }).await
}
