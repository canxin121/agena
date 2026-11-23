use super::*;
use agena_plugin_host::sdk::{
    InitContext, Plugin, ToolInvokeInput,
    hooks::{EventEnvelope, EventFilter},
    host_api::{
        EventSubscription, HostCallbackContext, LogLevel, PathPermissionQuery, PermissionQuery,
        current_host_callback_context, run_in_isolated_host_callback_context,
    },
};
use agena_plugin_sdk::browser::{BrowserControlCommand, BrowserControlResult};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::sync::Mutex as SyncMutex;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

struct FixtureHost;

#[async_trait]
impl HostClient for FixtureHost {
    async fn bot_context(&self) -> SdkResult<Value> {
        Ok(
            match current_host_callback_context().and_then(|context| context.session_id) {
                Some(91 | 92) => json!({"enabled": true, "bot_id": uuid::Uuid::from_u128(1)}),
                Some(93) => json!({"enabled": true, "bot_id": uuid::Uuid::from_u128(2)}),
                _ => json!({"enabled": false}),
            },
        )
    }
    async fn log(&self, _: LogLevel, _: String, _: Value) {}
    async fn publish_event(&self, _: EventEnvelope) -> SdkResult<()> {
        Ok(())
    }
    async fn publish_activity(&self, _: BackgroundActivity) -> SdkResult<()> {
        Ok(())
    }
    async fn register_browser_controller(&self, _: Arc<dyn BrowserController>) -> SdkResult<()> {
        Ok(())
    }
    async fn subscribe_events(&self, _: EventFilter) -> SdkResult<EventSubscription> {
        unreachable!()
    }
    async fn read_config(&self, _: Option<String>) -> SdkResult<Value> {
        unreachable!()
    }
    async fn invoke_tool(&self, _: String, _: Value) -> SdkResult<ToolInvokeOutput> {
        unreachable!()
    }
    async fn require_path_permission(&self, _: PathPermissionQuery) -> SdkResult<()> {
        Ok(())
    }
    async fn require_network_permission(&self, target: String) -> SdkResult<()> {
        let url = url::Url::parse(&target).unwrap();
        if matches!(url.host_str(), Some("8.8.8.8" | "1.1.1.1"))
            && current_host_callback_context().and_then(|context| context.session_id) != Some(94)
        {
            Ok(())
        } else {
            Err(PluginError::from_kind(
                PluginErrorKind::PolicyDenied,
                "Fixture denied the destination",
            ))
        }
    }
    async fn check_resolved_network_permission(
        &self,
        _: String,
        _: String,
    ) -> SdkResult<PermissionQuery> {
        Ok(PermissionQuery::Allow {})
    }
}

struct Fixture {
    plugin: BrowserPlugin,
    workspace: tempfile::TempDir,
    requests: Arc<SyncMutex<Vec<String>>>,
    proxy: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn new() -> Self {
        let real_chrome = which::which("google-chrome")
            .or_else(|_| which::which("chromium"))
            .unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = listener.local_addr().unwrap();
        let wrapper = workspace.path().join("chrome-fixture");
        let chrome_path = real_chrome.to_str().unwrap().replace('\'', "'\\''");
        std::fs::write(
            &wrapper,
            format!("#!/bin/sh\nexec '{chrome_path}' --proxy-server=http://{proxy_addr} \"$@\"\n"),
        )
        .unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let requests = Arc::new(SyncMutex::new(Vec::new()));
        let proxy = tokio::spawn(serve(listener, requests.clone()));
        let plugin = BrowserPlugin::new();
        Plugin::init(
            &plugin,
            InitContext {
                agena_version: "test".into(),
                workspace_root: workspace.path().to_owned(),
                plugin_id: BROWSER_PLUGIN_ID.parse().unwrap(),
                host_callback_url: None,
                host_callback_token: None,
                settings: json!({"executable_path": wrapper, "capture_console_text": true}),
                protocol_version: agena_plugin_sdk::rpc::PROTOCOL_VERSION,
            },
            Arc::new(FixtureHost),
        )
        .await
        .unwrap();
        Self {
            plugin,
            workspace,
            requests,
            proxy,
        }
    }

    async fn call_for(&self, owner: i64, tool: &str, input: Value) -> SdkResult<ToolInvokeOutput> {
        self.call_plugin(&self.plugin, owner, tool, input, true)
            .await
    }

    async fn call_plugin(
        &self,
        plugin: &dyn Plugin,
        owner: i64,
        tool: &str,
        input: Value,
        sandbox_enabled: bool,
    ) -> SdkResult<ToolInvokeOutput> {
        self.call_in_workspace(
            plugin,
            owner,
            self.workspace.path().to_str().unwrap(),
            tool,
            input,
            sandbox_enabled,
        )
        .await
    }

    async fn call_in_workspace(
        &self,
        plugin: &dyn Plugin,
        owner: i64,
        workspace: &str,
        tool: &str,
        input: Value,
        sandbox_enabled: bool,
    ) -> SdkResult<ToolInvokeOutput> {
        let context = HostCallbackContext {
            session_id: Some(owner),
            call_id: Some(1),
            sandbox_enabled: Some(sandbox_enabled),
            ..Default::default()
        };
        run_in_isolated_host_callback_context(
            context,
            Plugin::tool_invoke(
                plugin,
                ToolInvokeInput {
                    tool_name: tool.to_owned(),
                    session_id: owner,
                    call_id: 1,
                    workspace_root: workspace.to_owned(),
                    caller: None,
                    input,
                },
            ),
        )
        .await
    }

    async fn call(&self, tool: &str, input: Value) -> Value {
        self.call_for(19, tool, input.clone())
            .await
            .unwrap_or_else(|error| panic!("{tool} {input}: {}", error.diagnostic_message()))
            .payload
            .unwrap()
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "starts Chrome with sandbox support and a local HTTP fixture"]
async fn scoped_contexts_share_tabs_and_storage_with_eligible_sessions() {
    let fixture = Fixture::new().await;
    let workspace = fixture.workspace.path().to_str().unwrap();
    let other_workspace = tempfile::tempdir().unwrap();
    let other_workspace = other_workspace.path().to_str().unwrap();
    let call = async |owner, workspace, tool, input| {
        fixture
            .call_in_workspace(&fixture.plugin, owner, workspace, tool, input, true)
            .await
            .map(|result| result.payload.unwrap())
    };
    let mut context_ids = BTreeMap::new();
    for scope in ["session", "workspace", "bot", "global"] {
        let args = json!({"name": "fixture", "scope": scope, "keep_alive": true});
        let created = call(91, workspace, "create_context", args.clone())
            .await
            .unwrap();
        let reused = call(91, workspace, "create_context", args).await.unwrap();
        assert_eq!(created["created"], true);
        assert_eq!(reused["created"], false);
        assert_eq!(created["context_id"], reused["context_id"]);
        assert_eq!(created["scope"], scope);
        assert_eq!(created["keep_alive"], true);
        context_ids.insert(scope, created["context_id"].clone());
    }

    for (owner, path, expected) in [
        (
            91,
            workspace,
            &["session", "workspace", "bot", "global"][..],
        ),
        (92, other_workspace, &["bot", "global"][..]),
        (93, workspace, &["workspace", "global"][..]),
        (93, "", &["global"][..]),
        (19, workspace, &["workspace", "global"][..]),
    ] {
        let listed = call(owner, path, "contexts", json!({})).await.unwrap();
        let contexts = listed["contexts"].as_array().unwrap();
        assert_eq!(contexts.len(), expected.len());
        for scope in expected {
            assert!(
                contexts
                    .iter()
                    .any(|context| context["context_id"] == context_ids[*scope])
            );
        }
    }
    let private = &context_ids["session"];
    let denied = call(
        92,
        workspace,
        "open",
        json!({"url":"http://8.8.8.8/","context_id":private}),
    )
    .await;
    assert!(denied.is_err());
    assert!(
        call(
            93,
            workspace,
            "close_context",
            json!({"context_id": context_ids["bot"]})
        )
        .await
        .is_err()
    );
    assert!(
        call(
            19,
            workspace,
            "create_context",
            json!({"name":"invalid","scope":"bot"})
        )
        .await
        .is_err()
    );

    let global = &context_ids["global"];
    let open = async |owner, path, id: &Value| {
        call(
            owner,
            path,
            "open",
            json!({"url":"http://8.8.8.8/","context_id":id}),
        )
        .await
        .unwrap()["session_id"]
            .clone()
    };
    let first_tab = open(91, workspace, global).await;
    let second_tab = open(92, other_workspace, global).await;
    let isolated_tab = open(91, workspace, private).await;
    let eval = async |owner, path, tab: &Value, expression| {
        call(
            owner,
            path,
            "evaluate",
            json!({"session_id":tab,"expression":expression}),
        )
        .await
    };
    let write = "document.cookie = 'fixture_context=shared; Path=/'; localStorage.setItem('fixture_context', 'shared')";
    eval(91, workspace, &first_tab, write).await.unwrap();
    let read = "[document.cookie, localStorage.getItem('fixture_context')]";
    for tab in [&first_tab, &second_tab] {
        let data = eval(92, other_workspace, tab, read).await.unwrap();
        assert_eq!(data["value"], json!(["fixture_context=shared", "shared"]));
    }
    assert_eq!(
        eval(91, workspace, &isolated_tab, read).await.unwrap()["value"],
        json!(["", null])
    );
    let denied = eval(94, other_workspace, &first_tab, read)
        .await
        .unwrap_err();
    assert_eq!(denied.kind, PluginErrorKind::PolicyDenied);
    assert_eq!(
        call(92, other_workspace, "list", json!({"context_id": global}))
            .await
            .unwrap()["sessions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    call(
        92,
        other_workspace,
        "close",
        json!({"session_id":first_tab}),
    )
    .await
    .unwrap();
    let result = call(91, workspace, "shutdown", json!({})).await.unwrap();
    assert_eq!(result["closed_sessions"], json!([isolated_tab]));
    assert_eq!(
        eval(92, other_workspace, &second_tab, read).await.unwrap()["value"],
        json!(["fixture_context=shared", "shared"])
    );
    let closed = call(
        92,
        other_workspace,
        "close_context",
        json!({"context_id":global}),
    )
    .await
    .unwrap();
    assert_eq!(closed["closed_sessions"], json!([second_tab]));
    assert!(fixture.plugin.browser_state.local.running().await.unwrap());
    Plugin::shutdown(&fixture.plugin).await.unwrap();
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.proxy.abort();
    }
}

#[tokio::test]
#[ignore = "starts Chrome with sandbox support and a local HTTP fixture"]
async fn full_page_screenshot_keeps_concurrent_previews_at_viewport_size() {
    let fixture = Fixture::new().await;
    let opened = fixture
        .call("open", json!({"url": "http://8.8.8.8/preview-frame"}))
        .await;
    let page_id = opened["session_id"].as_str().unwrap();
    let state = &fixture.plugin.browser_state;
    let preview = async |thumbnail| {
        let frame = state
            .operator_command(
                19,
                BrowserControlCommand::Frame {
                    page_id: page_id.to_owned(),
                    control_id: None,
                    thumbnail,
                },
                None,
                Arc::new(FixtureHost),
                &[],
            )
            .await
            .unwrap();
        let BrowserControlResult::Frame { jpeg, .. } = frame else {
            panic!("expected a browser frame");
        };
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(jpeg)
            .unwrap();
        image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg)
            .unwrap()
            .into_rgb8()
    };

    let initial = preview(false).await;
    let initial_thumbnail = preview(true).await;
    let page = state.pages.lock().await.get(page_id).cloned().unwrap();
    let capture_slot = page.driver.lock_capture().await.unwrap();
    let screenshot = fixture.call(
        "screenshot",
        json!({"session_id": page_id, "path": "full.png", "full_page": true}),
    );
    let frame = preview(false);
    let thumbnail = preview(true);
    tokio::pin!(screenshot, frame, thumbnail);
    let wait = Duration::from_millis(350);
    let (screenshot_wait, frame_wait, thumbnail_wait) = tokio::join!(
        biased;
        tokio::time::timeout(wait, &mut screenshot),
        tokio::time::timeout(wait, &mut frame),
        tokio::time::timeout(wait, &mut thumbnail),
    );
    assert!(screenshot_wait.is_err() && frame_wait.is_err() && thumbnail_wait.is_err());
    drop(capture_slot);
    let (_, overlap, overlap_thumbnail) = tokio::join!(screenshot, frame, thumbnail);
    let following = preview(false).await;
    let full_page = image::image_dimensions(fixture.workspace.path().join("full.png")).unwrap();
    assert!(full_page.0 > initial.width() && full_page.1 > initial.height());

    for (label, frame, expected) in [
        ("concurrent preview", overlap, initial.dimensions()),
        (
            "concurrent thumbnail",
            overlap_thumbnail,
            initial_thumbnail.dimensions(),
        ),
        ("following preview", following, initial.dimensions()),
    ] {
        assert_eq!(frame.dimensions(), expected, "{label}");
        let [r, g, b] = frame
            .get_pixel(frame.width() * 4 / 5, frame.height() * 4 / 5)
            .0;
        assert!(
            r > 180 && (80..160).contains(&g) && b < 90,
            "{label}: bottom-right marker was [{r}, {g}, {b}]"
        );
    }
    Plugin::shutdown(&fixture.plugin).await.unwrap();
}

const PREVIEW_FRAME: &str = r#"<!doctype html><title>Viewport fixture</title>
<style>
body { margin: 0; width: 2600px; height: 2800px; }
#viewport { position: fixed; inset: 0; background: rgb(36, 112, 176); }
#marker { position: fixed; inset: 65% 0 0 65%; background: rgb(216, 120, 48); }
</style>
<div id="viewport"></div><div id="marker"></div>"#;

async fn serve(listener: TcpListener, requests: Arc<SyncMutex<Vec<String>>>) {
    while let Ok((stream, _)) = listener.accept().await {
        let requests = requests.clone();
        tokio::spawn(async move {
            let mut stream = BufReader::new(stream);
            let mut line = String::new();
            stream.read_line(&mut line).await.unwrap_or_default();
            let mut parts = line.split_ascii_whitespace();
            let method = parts.next().unwrap_or_default();
            let target = parts.next().and_then(|url| url::Url::parse(url).ok());
            let path = target.as_ref().map_or("/", url::Url::path);
            let (status, content_type, disposition, body) = match (method, path) {
                ("GET", "/") => (200, "text/html; charset=utf-8", "", PAGE),
                ("GET", "/preview-frame") => (200, "text/html; charset=utf-8", "", PREVIEW_FRAME),
                ("GET", "/search") => (200, "text/html; charset=utf-8", "", SEARCH_FORM),
                ("GET", "/results") => (200, "text/html; charset=utf-8", "", SEARCH_RESULTS),
                ("GET", "/search-api") => (200, "application/json", "", "{}"),
                ("GET", "/frame" | "/remote") => (200, "text/html; charset=utf-8", "", FRAME),
                ("GET", "/popup") => (
                    200,
                    "text/html; charset=utf-8",
                    "",
                    "<title>Fixture Popup</title><h1>Popup ready</h1>",
                ),
                ("GET", "/attachment") => (
                    200,
                    "text/plain",
                    "Content-Disposition: attachment; filename=fixture.txt\r\n",
                    "download-body",
                ),
                ("GET", "/api") => (
                    503,
                    "application/json",
                    "",
                    r#"{"error":"fixture response"}"#,
                ),
                _ => (404, "text/plain", "", "fixture missing"),
            };
            if let Some(url) = &target {
                requests.lock().unwrap().push(url.path().to_owned());
            }
            loop {
                line.clear();
                match stream.read_line(&mut line).await {
                    Ok(0) | Err(_) => return,
                    Ok(_) if line == "\r\n" => break,
                    Ok(_) => {}
                }
            }
            match path {
                "/search-api" => tokio::time::sleep(Duration::from_millis(900)).await,
                "/unreachable" => return,
                _ => {}
            }
            let response = format!(
                "HTTP/1.1 {status} Fixture\r\nContent-Type: {content_type}\r\n{disposition}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len(),
            );
            let _ = stream.write_all(response.as_bytes()).await;
        });
    }
}

const FRAME: &str = r#"<!doctype html><title>Frame fixture</title>
<button id="frame-button" onclick="this.innerText='Frame clicked'">Frame button</button>
<button id="remove-frame" onclick="frameElement.remove()">Remove frame</button>"#;

const SEARCH_FORM: &str = r#"<!doctype html><title>Search fixture</title>
<form id="search"><label>Search <input id="query" name="q"></label><button>Submit</button></form>
<script>
document.querySelector('#search').onsubmit = async (event) => {
  event.preventDefault();
  const query = new URLSearchParams(new FormData(event.target));
  history.pushState({}, '', '/pending?' + query);
  await fetch('/search-api');
  setTimeout(() => location.replace('/results?' + query), 180);
};
</script>"#;

const SEARCH_RESULTS: &str = r#"<!doctype html><title>Search results</title>
<main><h1>Fixture search result</h1></main>
<iframe src="http://8.8.4.4/forbidden-frame"></iframe>"#;

#[tokio::test]
#[ignore = "starts Chrome and simulates deferred search navigation through a local proxy"]
async fn search_submission_settles_and_failed_navigation_is_recoverable() {
    let fixture = Fixture::new().await;
    let opened = fixture
        .call("open", json!({"url": "http://8.8.8.8/search"}))
        .await;
    let page = &opened["session_id"];
    let submitted = fixture
        .call(
            "type",
            json!({
                "session_id": page, "selector": "#query", "text": "Rust", "press_enter": true,
            }),
        )
        .await;
    assert_eq!(
        submitted["snapshot"]["url"],
        "http://8.8.8.8/results?q=Rust"
    );
    let snapshot = fixture.call("snapshot", json!({"session_id": page})).await;
    assert!(
        snapshot["snapshot"]["tree"]
            .as_str()
            .unwrap()
            .contains("Fixture search result")
    );
    let text = fixture
        .call("content", json!({"session_id": page, "selector": "main"}))
        .await;
    assert_eq!(text["content"], "Fixture search result");
    assert!(
        !fixture
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|path| path == "/forbidden-frame")
    );

    let failure = fixture.call_for(19, "action", json!({
        "session_id": page, "action": {"kind": "navigate", "url": "http://8.8.8.8/unreachable"},
    })).await.unwrap_err();
    let network = fixture
        .call(
            "diagnostics",
            json!({
                "session_id": page, "query": {"kind": "network", "url_contains": "/unreachable"},
            }),
        )
        .await;
    assert_eq!(
        network["requests"][0]["state"], "failed",
        "{failure} {network}"
    );
    for tool in ["snapshot", "content"] {
        let error = fixture
            .call_for(19, tool, json!({"session_id": page}))
            .await
            .unwrap_err();
        assert!(
            error
                .diagnostic_message()
                .contains("Chrome could not load the page (net::ERR_"),
            "{error} {network}"
        );
    }
    let recovered = fixture
        .call(
            "action",
            json!({
                "session_id": page, "action": {"kind": "navigate", "url": "http://8.8.8.8/results"},
            }),
        )
        .await;
    assert_eq!(recovered["snapshot"]["title"], "Search results");
    fixture.call("shutdown", json!({})).await;
}

const PAGE: &str = r#"<!doctype html><title>Agena fixture</title>
<main id="main"><p class="row">第一行🙂</p><p class="row">second</p></main>
<button id="hover" onmouseover="this.textContent='Hovered'" ondblclick="this.textContent='Double clicked'">Hover target</button>
<input id="entry" value="start">
<input id="upload" type="file">
<a id="download" href="/attachment">Export</a>
<a id="blob" download="local.txt">Generated file</a>
<a id="popup" href="/popup" rel="noopener" target="_blank">Open popup</a>
<a id="blocked" href="http://8.8.4.4/blocked" target="_blank">Blocked popup</a>
<iframe name="same" src="/frame"></iframe>
<iframe name="remote" src="http://1.1.1.1/remote"></iframe>
<div id="drag-source" draggable="true" style="width:80px" ondragstart="event.dataTransfer.setData('text/plain', 'fixture')">Drag source</div>
<div id="drag-target" style="width:100px" ondragover="event.preventDefault()" ondrop="this.textContent='Dropped'">Drag target</div>
<style>#spacer {height:1200px}</style><div id="spacer"></div><p id="below">Below fold</p>
<script>
  const node = document.createElement('span');
  node.id = 'rendered';
  node.textContent = 'Created by JS';
  document.getElementById('main').append(node);
  document.getElementById('blob').href = URL.createObjectURL(new Blob(['generated']));
  console.warn('fixture console');
  throw new Error('fixture exception');
</script>"#;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "starts an isolated Chrome and a local HTTP fixture"]
async fn advanced_tools_with_chrome() {
    let fixture = Fixture::new().await;
    let opened = fixture
        .call("open", json!({"url": "http://8.8.8.8/"}))
        .await;
    let page = opened["session_id"].as_str().unwrap();
    let eval = |expression: &str| {
        fixture.call(
            "evaluate",
            json!({"session_id": page, "expression": expression}),
        )
    };

    assert_eq!(
        eval("Promise.resolve({answer: 42})").await["value"]["answer"],
        42
    );
    assert_eq!(eval("undefined").await["unserializable"], "undefined");
    assert!(
        eval("null")
            .await
            .as_object()
            .unwrap()
            .contains_key("value")
    );
    assert_eq!(eval("123n").await["unserializable"], "123n");
    let exception = eval("(() => { throw new Error('fixture evaluation') })()").await;
    assert_eq!(exception["status"], "exception");
    assert!(
        exception["exception"]["message"]
            .as_str()
            .unwrap()
            .contains("fixture evaluation")
    );
    assert_eq!(
        eval("'a'.repeat(30000)").await["status"],
        "result_too_large"
    );

    let content = fixture
        .call(
            "content",
            json!({
                "session_id": page, "format": "dom_html", "selector": "#main", "max_chars": 30,
            }),
        )
        .await;
    assert_eq!(content["returned_chars"], 30);
    assert_eq!(
        eval("document.querySelector('#main').remove(); true").await["value"],
        true
    );
    let continued = fixture.call("content", json!({
        "session_id": page, "content_id": content["content_id"], "offset": content["next_offset"],
    })).await;
    assert!(
        continued["content"]
            .as_str()
            .unwrap()
            .contains("<span id=\"rendered\">")
    );
    assert!(
        fixture
            .call_for(
                20,
                "content",
                json!({
                    "session_id": page, "content_id": content["content_id"],
                })
            )
            .await
            .is_err()
    );
    let response = fixture
        .call(
            "content",
            json!({"session_id": page, "format": "response_html"}),
        )
        .await;
    let source = response["content"].as_str().unwrap();
    assert!(source.starts_with("<!doctype html>"));
    assert!(!source.contains("<span id=\"rendered\">"));

    let frame_list = fixture.call("frames", json!({"session_id": page})).await;
    let frames = frame_list["frames"].as_array().unwrap();
    let same = frames.iter().find(|frame| frame["name"] == "same").unwrap()["frame_id"]
        .as_str()
        .unwrap();
    let remote = frames
        .iter()
        .find(|frame| frame["url"] == "http://1.1.1.1/remote")
        .unwrap()["frame_id"]
        .as_str()
        .unwrap();
    assert_eq!(
        fixture
            .call(
                "evaluate",
                json!({
                    "session_id": page, "frame_id": remote, "expression": "document.title"
                })
            )
            .await["value"],
        "Frame fixture"
    );
    for id in [same, remote] {
        let result = fixture.call("click", json!({
            "session_id": page, "frame_id": id, "selector": "#frame-button", "return_snapshot": false,
        })).await;
        assert_eq!(result["action"]["status"], "completed");
        let frame_text = fixture
            .call(
                "content",
                json!({
                    "session_id": page, "frame_id": id, "selector": "#frame-button",
                }),
            )
            .await;
        assert_eq!(frame_text["content"], "Frame clicked");
    }

    let snapshot = fixture
        .call("snapshot", json!({"session_id": page, "frame_id": same}))
        .await;
    let row = snapshot["snapshot"]["tree"]
        .as_str()
        .unwrap()
        .lines()
        .find(|line| line.contains("Remove frame"))
        .unwrap();
    let reference = row
        .split("[ref=")
        .nth(1)
        .unwrap()
        .split(']')
        .next()
        .unwrap();
    let reference = format!("@{reference}");
    let inspected = fixture
        .call(
            "inspect",
            json!({
                "session_id": page, "frame_id": same, "selector": reference, "attributes": ["id"],
            }),
        )
        .await;
    assert_eq!(inspected["element"]["attributes"]["id"], "remove-frame");
    let removed = fixture
        .call(
            "click",
            json!({
                "session_id": page, "frame_id": same, "selector": reference,
            }),
        )
        .await;
    assert_eq!(removed["action"]["status"], "completed");
    assert_eq!(removed["observation"]["status"], "failed");

    let failed = eval(
        "fetch('/api').then(async response => { await response.text(); return response.status })",
    )
    .await;
    assert_eq!(failed["value"], 503);
    let reqs = fixture
        .call(
            "diagnostics",
            json!({
                "session_id": page, "query": {"kind": "network", "url_contains": "/api"},
            }),
        )
        .await;
    let request_id = &reqs["requests"][0]["request_id"];
    assert_eq!(reqs["requests"][0]["status"], 503);
    let details = fixture.call("diagnostics", json!({
        "session_id": page, "query": {"kind": "request", "request_id": request_id, "include_body": true},
    })).await;
    assert_eq!(details["body"]["text"], r#"{"error":"fixture response"}"#);
    let logs = fixture
        .call(
            "diagnostics",
            json!({"session_id": page, "query": {"kind": "logs"}}),
        )
        .await;
    assert!(
        logs["logs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["kind"] == "exception")
    );
    assert!(
        logs["logs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["message"] == "fixture console")
    );

    for (kind, selector) in [
        ("hover", "#hover"),
        ("double_click", "#hover"),
        ("focus", "#entry"),
        ("scroll_into_view", "#below"),
    ] {
        let result = fixture.call("action", json!({
            "session_id": page, "action": {"kind": kind, "selector": selector}, "return_snapshot": false,
        })).await;
        assert_eq!(result["action"]["status"], "completed");
    }
    assert_eq!(
        eval("document.querySelector('#hover').textContent").await["value"],
        "Double clicked"
    );
    assert_eq!(eval("document.activeElement.id").await["value"], "entry");
    let drag = fixture.call("action", json!({
        "session_id": page, "action": {"kind": "drag", "source": "#drag-source", "target": "#drag-target"},
    })).await;
    assert_eq!(drag["action"]["status"], "completed");

    let waited = fixture.call("wait", json!({
        "session_id": page, "selector": "#main", "state": "detached", "return_snapshot": false,
    })).await;
    assert_eq!(waited["action"]["status"], "completed");
    fixture
        .call(
            "wait",
            json!({"session_id": page, "url": "*8.8.8.8/*", "return_snapshot": false}),
        )
        .await;
    fixture
        .call(
            "wait",
            json!({"session_id": page, "load_state": "load", "return_snapshot": false}),
        )
        .await;

    let upload = fixture.workspace.path().join("input.txt");
    tokio::fs::write(&upload, "uploaded content").await.unwrap();
    fixture.call("upload", json!({
        "session_id": page, "selector": "#upload", "files": ["input.txt"], "return_snapshot": false,
    })).await;
    assert_eq!(
        eval("document.querySelector('#upload').files[0].text()").await["value"],
        "uploaded content"
    );
    for (selector, contents, filename) in [
        ("#download", "download-body", "fixture.txt"),
        ("#blob", "generated", "local.txt"),
    ] {
        let download = fixture
            .call(
                "download",
                json!({"session_id": page, "selector": selector, "timeout_ms": 5000}),
            )
            .await;
        assert_eq!(download["filename"], filename);
        assert_eq!(
            tokio::fs::read_to_string(download["path"].as_str().unwrap())
                .await
                .unwrap(),
            contents
        );
    }

    let popup = fixture
        .call(
            "click",
            json!({
                "session_id": page, "selector": "#popup", "return_snapshot": false,
            }),
        )
        .await;
    let popup_id = popup["opened_sessions"][0]
        .as_str()
        .expect("Chrome popup was not adopted");
    let listed = fixture.call("list", json!({})).await;
    assert!(
        listed["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tab| { tab["session_id"] == popup_id && tab["opener_session_id"] == page })
    );
    let popup_content = fixture
        .call("content", json!({"session_id": popup_id}))
        .await;
    assert_eq!(popup_content["content"], "Popup ready");
    fixture
        .call(
            "click",
            json!({"session_id": page, "selector": "#blocked", "return_snapshot": false}),
        )
        .await;
    assert!(
        !fixture
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| r == "/blocked")
    );

    let timeout = fixture
        .call(
            "evaluate",
            json!({
                "session_id": popup_id, "expression": "new Promise(() => {})", "timeout_ms": 200
            }),
        )
        .await;
    assert_eq!(timeout["status"], "timeout");
    assert_eq!(timeout["page_closed"], true);
    assert_eq!(eval("document.title").await["value"], "Agena fixture");
    fixture.call("shutdown", json!({})).await;
    fixture.plugin.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "starts isolated Chrome processes and a local HTTP fixture"]
async fn rendered_fetch_and_interactive_browser_have_separate_lifetimes() {
    let fixture = Fixture::new().await;
    let page = fixture
        .call("open", json!({"url": "http://8.8.8.8/"}))
        .await["session_id"]
        .clone();
    fixture
        .call(
            "evaluate",
            json!({
                "session_id": page, "expression": "document.title = 'Interactive page'"
            }),
        )
        .await;

    let renderer = crate::tool::new_web_plugin();
    let workspace = fixture.workspace.path();
    Plugin::init(
        &renderer,
        InitContext {
            agena_version: "test".into(),
            workspace_root: workspace.to_owned(),
            plugin_id: crate::tool::web_plugin_id().parse().unwrap(),
            host_callback_url: None,
            host_callback_token: None,
            settings: json!({
                "browser": {
                    "enabled": true, "executable_path": workspace.join("chrome-fixture"),
                    "wait": {"for_network_idle": false},
                },
                "fetch": {"request": {"respect_robots_txt": false, "delay_ms": 1}},
            }),
            protocol_version: agena_plugin_sdk::rpc::PROTOCOL_VERSION,
        },
        Arc::new(FixtureHost),
    )
    .await
    .unwrap();
    let rendered = fixture
        .call_plugin(
            &renderer,
            19,
            "fetch",
            json!({"url": "http://8.8.8.8/", "render_js": true, "use_cache": false}),
            false,
        )
        .await
        .unwrap()
        .payload
        .unwrap();
    assert_eq!(rendered["rendered"], true);
    assert!(
        rendered["markdown"]
            .as_str()
            .unwrap()
            .contains("Created by JS")
    );
    assert_ne!(rendered["title"], "Interactive page");

    Plugin::shutdown(&renderer).await.unwrap();
    let title = fixture
        .call(
            "evaluate",
            json!({
                "session_id": page, "expression": "document.title",
            }),
        )
        .await;
    assert_eq!(title["value"], "Interactive page");
    Plugin::shutdown(&fixture.plugin).await.unwrap();
}
