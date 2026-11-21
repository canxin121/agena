use super::*;
use agena_plugin_host::sdk::{
    InitContext, Plugin, ToolInvokeInput,
    hooks::{EventEnvelope, EventFilter},
    host_api::{
        EventSubscription, HostCallbackContext, LogLevel, PathPermissionQuery, PermissionQuery,
        run_in_isolated_host_callback_context,
    },
};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::sync::Mutex as SyncMutex;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

struct FixtureHost;

#[async_trait]
impl HostClient for FixtureHost {
    async fn log(&self, _: LogLevel, _: String, _: Value) {}
    async fn publish_event(&self, _: EventEnvelope) -> SdkResult<()> {
        Ok(())
    }
    async fn publish_activity(&self, _: BackgroundActivity) -> SdkResult<()> {
        Ok(())
    }
    async fn register_activity_source(
        &self,
        _: BackgroundActivityKind,
        _: Arc<dyn ActivitySourceAdapter>,
    ) -> SdkResult<()> {
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
        if matches!(url.host_str(), Some("8.8.8.8" | "1.1.1.1")) {
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
                    workspace_root: self.workspace.path().to_str().unwrap().to_owned(),
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

impl Drop for Fixture {
    fn drop(&mut self) {
        self.proxy.abort();
    }
}

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
