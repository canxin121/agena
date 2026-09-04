#![cfg(unix)]

use std::{
    fs::OpenOptions,
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};

use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};
use tokio::time::{Instant, sleep};

struct TestServer {
    child: Child,
    base_url: String,
    _fixture: TempDir,
}

async fn login_ui(client: &Client, base_url: &str, password: &str) -> String {
    let (status, _, body) = response_json(
        client
            .post(format!("{base_url}/auth/session"))
            .json(&json!({"password": password}))
            .send()
            .await
            .expect("login to the Agena management API"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    body["token"]
        .as_str()
        .expect("management login returns a bearer token")
        .to_owned()
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn start_server() -> TestServer {
    start_server_with_options(None, &[]).await
}

async fn start_server_with_options(
    ui_password: Option<&str>,
    environment: &[(&str, &str)],
) -> TestServer {
    let fixture = tempdir().expect("create MCP HTTP fixture");
    let workspace = fixture.path().join("workspace");
    let server_data = fixture.path().join("server-data");
    let database = fixture.path().join("server.db");
    let log_path = fixture.path().join("server.log");
    std::fs::create_dir_all(&workspace).expect("create MCP HTTP workspace");
    std::fs::create_dir_all(&server_data).expect("create MCP HTTP server data");

    let (child, base_url) = spawn_server(
        &workspace,
        &server_data,
        &database,
        &log_path,
        ui_password,
        environment,
    )
    .await;

    TestServer {
        child,
        base_url,
        _fixture: fixture,
    }
}

async fn spawn_server(
    workspace: &Path,
    server_data: &Path,
    database: &Path,
    log_path: &Path,
    ui_password: Option<&str>,
    environment: &[(&str, &str)],
) -> (Child, String) {
    // Let the server bind port 0 itself. Reserving an ephemeral port in the
    // test process and releasing it before spawning the child leaves a TOCTOU
    // window where parallel tests (or another local process) can claim the
    // port. A unique endpoint record gives us the actual bound port without
    // that race.
    let record_path = server_data.join(format!(
        "mcp-http-server-{}.json",
        uuid::Uuid::new_v4().simple()
    ));

    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .expect("open MCP HTTP server log");
    let stderr = log.try_clone().expect("clone MCP HTTP server log");
    let mut command = Command::new(env!("CARGO_BIN_EXE_agena"));
    command
        .arg("--database-path")
        .arg(database)
        .arg("server")
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg("0")
        .arg("--workspace")
        .arg(workspace)
        .env("AGENA_SERVER_DATA_DIR", server_data)
        .env("AGENA_SERVER_RECORD", &record_path)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr));
    for name in [
        "AGENA_MCP_ENABLED",
        "AGENA_MCP_PUBLIC_URL",
        "AGENA_MCP_OAUTH_ISSUER_URL",
        "AGENA_MCP_AUTH_MODE",
        "AGENA_MCP_ANONYMOUS_ACCESS",
    ] {
        command.env_remove(name);
    }
    match ui_password {
        Some(password) => {
            command.env("AGENA_SERVER_UI_PASSWORD", password);
        }
        None => {
            command.env_remove("AGENA_SERVER_UI_PASSWORD");
        }
    }
    for (name, value) in environment {
        command.env(name, value);
    }
    let mut child = command.spawn().expect("spawn MCP HTTP server");

    let client = test_client();
    // A cold macOS GitHub runner may spend well over 20 seconds initializing
    // the server runtime. Keep a generous readiness deadline, but fail
    // immediately if the child exits instead of masking a real startup error
    // as a timeout.
    let deadline = Instant::now() + Duration::from_secs(60);
    let base_url = loop {
        if let Ok(bytes) = std::fs::read(&record_path)
            && let Ok(record) = serde_json::from_slice::<Value>(&bytes)
            && let Some(url) = record.get("url").and_then(Value::as_str)
            && let Ok(response) = client.get(format!("{url}/health")).send().await
            && response.status().is_success()
        {
            break url.to_owned();
        }
        if let Some(status) = child.try_wait().expect("inspect MCP HTTP server child") {
            panic!(
                "MCP HTTP server exited before becoming ready with status {status}; log:\n{}",
                std::fs::read_to_string(log_path).unwrap_or_default()
            );
        }
        assert!(
            Instant::now() < deadline,
            "MCP HTTP server did not become ready; log:\n{}",
            std::fs::read_to_string(log_path).unwrap_or_default()
        );
        sleep(Duration::from_millis(50)).await;
    };

    (child, base_url)
}

fn test_client() -> Client {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("build MCP HTTP test client")
}

fn tool_name_is_hidden_from_stateless_mcp(name: &str) -> bool {
    let compact = name.strip_prefix("agena.").unwrap_or(name);
    [
        "chatgpt.",
        "gemini.",
        "claude.",
        "openai.",
        "schema_lab.",
        "interaction.",
        "session.",
        "plan.",
        "tasks.",
        "cron.",
        "monitor.",
        "snapshot.",
        "report.",
        "settings.",
        "memory.",
        "skills.",
        "tools.",
        "mcp.",
        "web.browser_",
    ]
    .iter()
    .any(|prefix| compact.starts_with(prefix))
}

async fn response_json(
    response: reqwest::Response,
) -> (StatusCode, reqwest::header::HeaderMap, Value) {
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.bytes().await.expect("read MCP HTTP response");
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("decode MCP HTTP JSON response")
    };
    (status, headers, body)
}

async fn post_mcp(client: &Client, url: &str, request: Value) -> (StatusCode, Value) {
    post_mcp_with_headers(client, url, request, &[]).await
}

async fn post_mcp_with_headers(
    client: &Client,
    url: &str,
    request: Value,
    headers: &[(&str, &str)],
) -> (StatusCode, Value) {
    let mut request_builder = client
        .post(url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream");
    for (name, value) in headers {
        request_builder = request_builder.header(*name, *value);
    }
    let response = request_builder
        .json(&request)
        .send()
        .await
        .expect("send MCP request");
    let (status, _, body) = response_json(response).await;
    (status, body)
}

async fn post_mcp_raw(client: &Client, url: &str, body: &[u8]) -> (StatusCode, Value) {
    let response = client
        .post(url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(body.to_owned())
        .send()
        .await
        .expect("send raw MCP request");
    let (status, _, body) = response_json(response).await;
    (status, body)
}

fn initialize_request(id: i64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "agena-http-test", "version": "1"}
        }
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn anonymous_mcp_is_stateless_and_hides_interactive_tools() {
    let server = start_server().await;
    let client = test_client();
    let mcp_url = format!("{}/mcp", server.base_url);

    let (status, _, control) = response_json(
        client
            .get(format!("{}/api/v1/server/mcp", server.base_url))
            .send()
            .await
            .expect("read default MCP control state"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(control["ready"], false);
    assert!(
        control["warnings"]
            .as_array()
            .is_some_and(|warnings| warnings.iter().any(|warning| warning
                .as_str()
                .is_some_and(|warning| warning.contains("remote HTTPS"))))
    );

    let (status, _, error) = response_json(
        client
            .put(format!("{}/api/v1/server/mcp", server.base_url))
            .json(&json!({
                "enabled": true,
                "authMode": "oauth",
                "publicUrl": "https://mcp.example.test/mcp"
            }))
            .send()
            .await
            .expect("reject public OAuth without a management password"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        error["error"]
            .as_str()
            .is_some_and(|message| message.contains("AGENA_SERVER_UI_PASSWORD"))
    );

    let metadata = client
        .get(format!(
            "{}/.well-known/oauth-protected-resource",
            server.base_url
        ))
        .send()
        .await
        .expect("request anonymous OAuth metadata");
    assert_eq!(metadata.status(), StatusCode::NOT_FOUND);
    assert!(
        metadata
            .bytes()
            .await
            .expect("read metadata body")
            .is_empty()
    );

    let untrusted_host = client
        .post(&mcp_url)
        .header("host", "attacker.example")
        .header("content-type", "application/json")
        .json(&initialize_request(0))
        .send()
        .await
        .expect("send MCP request with untrusted Host");
    assert_eq!(untrusted_host.status(), StatusCode::BAD_REQUEST);

    let untrusted_origin = client
        .post(&mcp_url)
        .header("origin", "https://attacker.example")
        .header("content-type", "application/json")
        .json(&initialize_request(0))
        .send()
        .await
        .expect("send MCP request with untrusted Origin");
    assert_eq!(untrusted_origin.status(), StatusCode::FORBIDDEN);

    // Secure MCP Tunnel forwards each JSON-RPC request independently. Some
    // connector probes can therefore ask for tools before initialize and
    // without a session ID. The old rmcp stateful dispatcher returned HTTP
    // 422 for this exact sequence; the public stateless adapter must answer
    // normally instead.
    let (status, pre_initialize_tools) = post_mcp(
        &client,
        &mcp_url,
        json!({"jsonrpc":"2.0","id":"pre-init","method":"tools/list","params":{}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !pre_initialize_tools["result"]["tools"]
            .as_array()
            .expect("pre-initialize tools/list returns an array")
            .is_empty()
    );
    assert!(
        pre_initialize_tools["result"]["tools"]
            .as_array()
            .expect("pre-initialize tools/list returns an array")
            .iter()
            .all(|tool| !tool_name_is_hidden_from_stateless_mcp(
                tool["name"].as_str().unwrap_or_default()
            ))
    );

    let (status, discover) = post_mcp_with_headers(
        &client,
        &mcp_url,
        json!({
            "jsonrpc": "2.0",
            "id": "discover",
            "method": "server/discover",
            "params": {
                "_meta": {
                    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                    "io.modelcontextprotocol/clientInfo": {
                        "name": "ChatGPT",
                        "version": "test"
                    },
                    "io.modelcontextprotocol/clientCapabilities": {}
                }
            }
        }),
        &[
            ("MCP-Protocol-Version", "2026-07-28"),
            ("Mcp-Method", "server/discover"),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(discover["result"]["resultType"], "complete");
    assert_eq!(discover["result"]["supportedVersions"][0], "2026-07-28");
    assert_eq!(discover["result"]["capabilities"]["tools"], json!({}));
    assert_eq!(
        discover["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "agena"
    );
    assert!(discover["result"]["ttlMs"].as_u64().is_some());
    assert_eq!(discover["result"]["cacheScope"], "private");

    let (status, modern_tools) = post_mcp_with_headers(
        &client,
        &mcp_url,
        json!({
            "jsonrpc": "2.0",
            "id": "modern-tools",
            "method": "tools/list",
            "params": {
                "_meta": {
                    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                    "io.modelcontextprotocol/clientInfo": {
                        "name": "ChatGPT",
                        "version": "test"
                    },
                    "io.modelcontextprotocol/clientCapabilities": {}
                }
            }
        }),
        &[
            ("MCP-Protocol-Version", "2026-07-28"),
            ("Mcp-Method", "tools/list"),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(modern_tools["result"]["resultType"], "complete");
    assert_eq!(modern_tools["result"]["cacheScope"], "private");
    assert_eq!(modern_tools["result"]["ttlMs"], 30_000);
    let modern_tool_list = modern_tools["result"]["tools"]
        .as_array()
        .expect("modern tools/list returns an array");
    assert!(
        modern_tool_list
            .iter()
            .any(|tool| tool["name"] == "shell.run")
    );
    assert!(modern_tool_list.iter().all(|tool| {
        !tool_name_is_hidden_from_stateless_mcp(tool["name"].as_str().unwrap_or_default())
    }));

    let (status, initialize) = post_mcp(&client, &mcp_url, initialize_request(1)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(initialize["result"]["protocolVersion"], "2025-06-18");

    let (status, tools) = post_mcp(
        &client,
        &mcp_url,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let listed = tools["result"]["tools"]
        .as_array()
        .expect("tools/list returns an array");
    assert!(!listed.is_empty());
    assert!(
        listed
            .iter()
            .all(|tool| { tool["securitySchemes"] == json!([{"type":"noauth"}]) })
    );
    assert!(listed.iter().all(|tool| {
        let name = tool["name"].as_str().unwrap_or_default();
        !name.contains("interactive") && !tool_name_is_hidden_from_stateless_mcp(name)
    }));

    // The compatibility seam is deliberately tools-only. Other legacy-era
    // requests without per-request protocol metadata are rejected by rmcp's
    // modern stateless header validator rather than being silently accepted.
    let (status, unsupported) = post_mcp(
        &client,
        &mcp_url,
        json!({"jsonrpc":"2.0","id":20,"method":"resources/list","params":{}}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(unsupported["error"]["code"], -32020);

    let (status, unknown) = post_mcp(
        &client,
        &mcp_url,
        json!({"jsonrpc":"2.0","id":21,"method":"agena/unknown","params":{}}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(unknown["error"]["code"], -32020);

    let (status, parse_error) = post_mcp_raw(&client, &mcp_url, br"{not-json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(parse_error["error"]["code"], -32700);

    let (status, batch) = post_mcp(
        &client,
        &mcp_url,
        json!([
            initialize_request(22),
            {"jsonrpc":"2.0","id":23,"method":"tools/list","params":{}}
        ]),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(batch.as_array().expect("JSON-RPC batch response").len(), 2);
    assert!(batch[1]["result"]["tools"].is_array());

    let (status, hidden_call) = post_mcp(
        &client,
        &mcp_url,
        json!({
            "jsonrpc":"2.0",
            "id":3,
            "method":"tools/call",
            "params":{"name":"interaction.ask","arguments":{}}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(hidden_call["error"].is_object());

    let (status, hidden_session_call) = post_mcp(
        &client,
        &mcp_url,
        json!({
            "jsonrpc":"2.0",
            "id":4,
            "method":"tools/call",
            "params":{"name":"session.model","arguments":{}}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(hidden_session_call["error"].is_object());

    for (id, name) in [(5, "plan.get"), (6, "settings.get"), (7, "mcp.tools.call")] {
        let (status, hidden_internal_call) = post_mcp(
            &client,
            &mcp_url,
            json!({
                "jsonrpc":"2.0",
                "id":id,
                "method":"tools/call",
                "params":{"name":name,"arguments":{}}
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            hidden_internal_call["error"].is_object(),
            "{name} must not be callable through stateless MCP"
        );
    }

    let (status, _, _) = response_json(
        client
            .put(format!("{}/api/v1/server/mcp", server.base_url))
            .json(&json!({"enabled":false,"authMode":"none"}))
            .send()
            .await
            .expect("disable MCP server"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let disabled = client
        .post(&mcp_url)
        .json(&initialize_request(4))
        .send()
        .await
        .expect("request disabled MCP server");
    assert_eq!(disabled.status(), StatusCode::NOT_FOUND);
    assert!(
        disabled
            .bytes()
            .await
            .expect("read disabled MCP body")
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn oauth_discovery_is_cimd_only() {
    let management_password = "Agena-management-password-2026";
    let server = start_server_with_options(Some(management_password), &[]).await;
    let client = test_client();
    let management_token = login_ui(&client, &server.base_url, management_password).await;
    let resource = "https://mcp.example.test/mcp";
    let issuer = "https://auth.example.test";
    let password = "MCP-HTTP-test-password-2026";

    let (status, _, error) = response_json(
        client
            .put(format!("{}/api/v1/server/mcp", server.base_url))
            .bearer_auth(&management_token)
            .json(&json!({
                "enabled": true,
                "authMode": "oauth",
                "publicUrl": "https://tunnel.example/v1/mcp/tunnel_123"
            }))
            .send()
            .await
            .expect("reject routed OAuth resource without issuer"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        error["error"].as_str().is_some_and(
            |message| message.contains("explicit browser-reachable OAuth issuer origin")
        )
    );

    let (status, _, _) = response_json(
        client
            .put(format!("{}/api/v1/server/mcp", server.base_url))
            .bearer_auth(&management_token)
            .json(&json!({
                "enabled": true,
                "authMode": "none",
                "publicUrl": resource,
                "oauthIssuerUrl": issuer
            }))
            .send()
            .await
            .expect("configure MCP HTTPS resource"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = response_json(
        client
            .put(format!(
                "{}/api/v1/server/mcp/oauth/password",
                server.base_url
            ))
            .bearer_auth(&management_token)
            .json(&json!({"password": password}))
            .send()
            .await
            .expect("set MCP OAuth password"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let oauth_enable_response = client
        .put(format!("{}/api/v1/server/mcp", server.base_url))
        .bearer_auth(&management_token)
        .json(&json!({"enabled": true, "authMode": "oauth"}))
        .send()
        .await
        .expect("send OAuth enable request");
    let (status, _, control) = response_json(oauth_enable_response).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(control["ready"], true);
    assert_eq!(control["authMode"], "oauth");
    assert_eq!(control["oauthIssuerUrl"], issuer);
    assert_eq!(
        control["oauth"]["protectedResourceMetadata"],
        "https://mcp.example.test/.well-known/oauth-protected-resource/mcp"
    );
    assert_eq!(
        control["oauth"]["authorizationServerMetadata"],
        "https://auth.example.test/.well-known/oauth-authorization-server"
    );

    let (status, _, protected_resource) = response_json(
        client
            .get(format!(
                "{}/.well-known/oauth-protected-resource",
                server.base_url
            ))
            .send()
            .await
            .expect("read protected resource metadata"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(protected_resource["resource"], resource);
    assert_eq!(
        protected_resource["scopes_supported"],
        json!(["agena:tools", "offline_access"])
    );
    assert_eq!(protected_resource["authorization_servers"], json!([issuer]));

    let (status, _, authorization_server) = response_json(
        client
            .get(format!(
                "{}/.well-known/oauth-authorization-server",
                server.base_url
            ))
            .send()
            .await
            .expect("read authorization server metadata"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        authorization_server["token_endpoint_auth_methods_supported"],
        json!(["none"])
    );
    assert_eq!(
        authorization_server["code_challenge_methods_supported"],
        json!(["S256"])
    );
    assert_eq!(
        authorization_server["client_id_metadata_document_supported"],
        true
    );
    assert_eq!(
        authorization_server["authorization_response_iss_parameter_supported"],
        true
    );
    assert_eq!(authorization_server["issuer"], issuer);
    assert_eq!(
        authorization_server["scopes_supported"],
        json!(["agena:tools", "offline_access"])
    );
    assert!(authorization_server.get("registration_endpoint").is_none());

    let disabled_registration = client
        .post(format!("{}/oauth/register", server.base_url))
        .json(&json!({
            "redirect_uris": ["https://chatgpt.com/connector/oauth/disabled"],
            "token_endpoint_auth_method": "none"
        }))
        .send()
        .await
        .expect("probe disabled DCR endpoint");
    assert_eq!(disabled_registration.status(), StatusCode::NOT_FOUND);

    let malformed_token = client
        .post(format!("{}/oauth/token", server.base_url))
        .header("content-type", "application/json")
        .body(r#"{\"grant_type\":\"authorization_code\"}"#)
        .send()
        .await
        .expect("send malformed OAuth token request");
    assert_eq!(malformed_token.status(), StatusCode::BAD_REQUEST);

    let (status, _, _) = response_json(
        client
            .put(format!("{}/api/v1/server/mcp", server.base_url))
            .bearer_auth(&management_token)
            .json(&json!({"enabled":true,"authMode":"none"}))
            .send()
            .await
            .expect("disable OAuth surface"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let metadata_after_disable = client
        .get(format!(
            "{}/.well-known/oauth-protected-resource",
            server.base_url
        ))
        .send()
        .await
        .expect("request disabled OAuth metadata");
    assert_eq!(metadata_after_disable.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mixed_auth_defaults_closed_and_can_explicitly_allow_read_only_tools() {
    let management_password = "Agena-mixed-management-password-2026";
    let server = start_server_with_options(Some(management_password), &[]).await;
    let client = test_client();
    let management_token = login_ui(&client, &server.base_url, management_password).await;
    let mcp_url = format!("{}/mcp", server.base_url);
    let resource = "https://mcp.example.test/mcp";
    let issuer = "https://auth.example.test";

    let (status, _, _) = response_json(
        client
            .put(format!("{}/api/v1/server/mcp", server.base_url))
            .bearer_auth(&management_token)
            .json(&json!({
                "enabled": true,
                "authMode": "none",
                "publicUrl": resource,
                "oauthIssuerUrl": issuer
            }))
            .send()
            .await
            .expect("configure mixed-auth MCP surface"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = response_json(
        client
            .put(format!(
                "{}/api/v1/server/mcp/oauth/password",
                server.base_url
            ))
            .bearer_auth(&management_token)
            .json(&json!({"password": "mixed-auth-test-password-2026"}))
            .send()
            .await
            .expect("set mixed-auth password"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, control) = response_json(
        client
            .put(format!("{}/api/v1/server/mcp", server.base_url))
            .bearer_auth(&management_token)
            .json(&json!({"authMode": "mixed"}))
            .send()
            .await
            .expect("enable mixed auth"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(control["authMode"], "mixed");
    assert_eq!(control["anonymousAccess"], "none");

    let (status, initialize) = post_mcp(&client, &mcp_url, initialize_request(30)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(initialize["result"]["serverInfo"]["name"], "agena");

    let (status, protected_catalog) = post_mcp(
        &client,
        &mcp_url,
        json!({"jsonrpc":"2.0","id":31,"method":"tools/list","params":{}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let protected_catalog = protected_catalog["result"]["tools"]
        .as_array()
        .expect("default-closed mixed tools/list array");
    assert!(
        protected_catalog.iter().all(|tool| {
            tool["securitySchemes"] == json!([{"type":"oauth2","scopes":["agena:tools"]}])
        }),
        "mixed auth must not make private read-only data anonymous by default"
    );

    let (status, _, control) = response_json(
        client
            .put(format!("{}/api/v1/server/mcp", server.base_url))
            .bearer_auth(&management_token)
            .json(&json!({"anonymousAccess": "read_only"}))
            .send()
            .await
            .expect("explicitly allow anonymous read-only tools"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(control["anonymousAccess"], "read_only");

    let (status, tools) = post_mcp(
        &client,
        &mcp_url,
        json!({"jsonrpc":"2.0","id":35,"method":"tools/list","params":{}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let tools = tools["result"]["tools"]
        .as_array()
        .expect("opt-in mixed tools/list array");
    let public_tool = tools
        .iter()
        .find(|tool| tool["securitySchemes"] == json!([{"type":"noauth"}]))
        .and_then(|tool| tool["name"].as_str())
        .expect("explicit opt-in exposes a read-only tool anonymously")
        .to_owned();
    let protected_tool = tools
        .iter()
        .find(|tool| tool["securitySchemes"] == json!([{"type":"oauth2","scopes":["agena:tools"]}]))
        .and_then(|tool| tool["name"].as_str())
        .expect("mixed catalog contains a protected privileged tool")
        .to_owned();

    let (status, protected_call) = post_mcp(
        &client,
        &mcp_url,
        json!({
            "jsonrpc":"2.0",
            "id":32,
            "method":"tools/call",
            "params":{"name":protected_tool,"arguments":{}}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(protected_call["result"]["isError"], true);
    assert!(
        protected_call["result"]["_meta"]["mcp/www_authenticate"][0]
            .as_str()
            .is_some_and(|challenge| challenge
                .contains("https://mcp.example.test/.well-known/oauth-protected-resource/mcp"))
    );

    let (status, unknown_call) = post_mcp(
        &client,
        &mcp_url,
        json!({
            "jsonrpc":"2.0",
            "id":34,
            "method":"tools/call",
            "params":{"name":"unknown.guessed.tool","arguments":{}}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(unknown_call["result"]["isError"], true);
    assert!(unknown_call.to_string().contains("mcp/www_authenticate"));

    let (status, public_call) = post_mcp(
        &client,
        &mcp_url,
        json!({
            "jsonrpc":"2.0",
            "id":33,
            "method":"tools/call",
            "params":{"name":public_tool,"arguments":{}}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !public_call.to_string().contains("mcp/www_authenticate"),
        "a noauth tool must reach the tool implementation rather than the OAuth challenge"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn explicit_public_oauth_environment_overrides_persisted_control_on_restart() {
    let fixture = tempdir().expect("create restartable MCP HTTP fixture");
    let workspace = fixture.path().join("workspace");
    let server_data = fixture.path().join("server-data");
    let database = fixture.path().join("server.db");
    let log_path = fixture.path().join("server.log");
    std::fs::create_dir_all(&workspace).expect("create restartable workspace");
    std::fs::create_dir_all(&server_data).expect("create restartable server data");

    let ui_password = "Agena-restart-management-password-2026";
    let client = test_client();
    let (mut first_child, first_base_url) = spawn_server(
        &workspace,
        &server_data,
        &database,
        &log_path,
        Some(ui_password),
        &[],
    )
    .await;
    let first_token = login_ui(&client, &first_base_url, ui_password).await;
    let (status, _, persisted) = response_json(
        client
            .put(format!("{first_base_url}/api/v1/server/mcp"))
            .bearer_auth(&first_token)
            .json(&json!({
                "enabled": false,
                "authMode": "none",
                "publicUrl": "https://old.example.test/mcp",
                "oauthIssuerUrl": "https://old-auth.example.test",
                "anonymousAccess": "read_only"
            }))
            .send()
            .await
            .expect("persist the initial MCP control state"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(persisted["enabled"], false);
    assert_eq!(persisted["publicUrl"], "https://old.example.test/mcp");
    first_child.kill().expect("stop first MCP server");
    first_child.wait().expect("reap first MCP server");

    let environment = [
        ("AGENA_MCP_ENABLED", "true"),
        ("AGENA_MCP_PUBLIC_URL", "https://new.example.test/mcp"),
        ("AGENA_MCP_AUTH_MODE", "oauth"),
        ("AGENA_MCP_ANONYMOUS_ACCESS", "none"),
    ];
    let (mut second_child, second_base_url) = spawn_server(
        &workspace,
        &server_data,
        &database,
        &log_path,
        Some(ui_password),
        &environment,
    )
    .await;
    let second_token = login_ui(&client, &second_base_url, ui_password).await;
    let (status, _, control) = response_json(
        client
            .get(format!("{second_base_url}/api/v1/server/mcp"))
            .bearer_auth(&second_token)
            .send()
            .await
            .expect("read environment-overridden MCP control state"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(control["enabled"], true);
    assert_eq!(control["publicUrl"], "https://new.example.test/mcp");
    assert_eq!(control["resourceUrl"], "https://new.example.test/mcp");
    assert!(control["oauthIssuerUrl"].is_null());
    assert_eq!(control["oauth"]["issuer"], "https://new.example.test");
    assert_eq!(control["authMode"], "oauth");
    assert_eq!(control["anonymousAccess"], "none");
    assert_eq!(control["ready"], true);

    second_child.kill().expect("stop second MCP server");
    second_child.wait().expect("reap second MCP server");
}
