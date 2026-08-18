#![cfg(unix)]

use std::{
    fs::OpenOptions,
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::{Client, StatusCode, header::LOCATION};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::{TempDir, tempdir};
use tokio::time::{Instant, sleep};
use url::Url;

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
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("reserve MCP HTTP port");
    let port = listener
        .local_addr()
        .expect("read reserved MCP HTTP port")
        .port();
    drop(listener);

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
        .arg(port.to_string())
        .arg("--workspace")
        .arg(workspace)
        .env("AGENA_SERVER_DATA_DIR", server_data)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr));
    for name in [
        "AGENA_MCP_ENABLED",
        "AGENA_MCP_PUBLIC_URL",
        "AGENA_MCP_OAUTH_ISSUER_URL",
        "AGENA_MCP_AUTH_MODE",
        "AGENA_MCP_ANONYMOUS_ACCESS",
        "AGENA_MCP_TOOL_EXPOSURE",
        "AGENA_MCP_CLIENT_REGISTRATION",
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
    let child = command.spawn().expect("spawn MCP HTTP server");

    let base_url = format!("http://127.0.0.1:{port}");
    let client = test_client();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(response) = client.get(format!("{base_url}/health")).send().await
            && response.status().is_success()
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "MCP HTTP server did not become ready; log:\n{}",
            std::fs::read_to_string(log_path).unwrap_or_default()
        );
        sleep(Duration::from_millis(50)).await;
    }

    (child, base_url)
}

fn test_client() -> Client {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("build MCP HTTP test client")
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
    assert!(
        modern_tools["result"]["tools"]
            .as_array()
            .expect("modern tools/list returns an array")
            .iter()
            .all(|tool| {
                tool["annotations"]["readOnlyHint"] == true
                    && tool["annotations"]["destructiveHint"] == false
                    && tool["annotations"]["openWorldHint"] == false
            })
    );

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
        !name.contains("interactive")
            && !name.contains("browser")
            && !name.contains("chatgpt")
            && !name.contains("gemini")
            && !name.contains("claude")
            && name != "plan.phase"
            && name != "plan.review"
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

    let (status, _, _) = response_json(
        client
            .put(format!("{}/api/v1/server/mcp", server.base_url))
            .json(&json!({"enabled":false,"authEnabled":false}))
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
async fn oauth_discovery_and_authorization_code_flow_are_chatgpt_compatible() {
    let management_password = "Agena-management-password-2026";
    let server = start_server_with_options(Some(management_password), &[]).await;
    let client = test_client();
    let management_token = login_ui(&client, &server.base_url, management_password).await;
    let mcp_url = format!("{}/mcp", server.base_url);
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
                "publicUrl": "https://tunnel.example/v1/mcp/tunnel_123",
                "toolExposure": "read_only"
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
                "oauthIssuerUrl": issuer,
                "toolExposure": "read_only",
                "clientRegistration": "cimd_only"
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
    assert_eq!(control["toolExposure"], "read_only");
    assert_eq!(control["authMode"], "oauth");
    assert_eq!(control["clientRegistration"], "cimd_only");
    assert_eq!(control["oauthIssuerUrl"], issuer);
    assert_eq!(
        control["oauth"]["protectedResourceMetadata"],
        "https://mcp.example.test/.well-known/oauth-protected-resource/mcp"
    );
    assert_eq!(
        control["oauth"]["authorizationServerMetadata"],
        "https://auth.example.test/.well-known/oauth-authorization-server"
    );
    assert!(control["oauth"].get("registrationEndpoint").is_none());

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

    let (status, _, control) = response_json(
        client
            .put(format!("{}/api/v1/server/mcp", server.base_url))
            .bearer_auth(&management_token)
            .json(&json!({"clientRegistration": "cimd_and_dcr"}))
            .send()
            .await
            .expect("enable OAuth DCR compatibility"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(control["clientRegistration"], "cimd_and_dcr");
    assert_eq!(
        control["oauth"]["registrationEndpoint"],
        "https://auth.example.test/oauth/register"
    );

    let (_, _, authorization_server) = response_json(
        client
            .get(format!(
                "{}/.well-known/oauth-authorization-server",
                server.base_url
            ))
            .send()
            .await
            .expect("read path-aware authorization server metadata"),
    )
    .await;
    assert_eq!(
        authorization_server["registration_endpoint"],
        "https://auth.example.test/oauth/register"
    );

    // Framework extractor failures must stay inside the OAuth error contract.
    // Axum's default JSON/form rejections can be HTTP 422, which the Secure
    // MCP Tunnel surfaces as a failed MCP target instead of an OAuth error.
    let malformed_registration = client
        .post(format!("{}/oauth/register", server.base_url))
        .header("content-type", "application/json")
        .body(r#"{"redirect_uris":"not-an-array"}"#)
        .send()
        .await
        .expect("send malformed OAuth registration");
    assert_eq!(malformed_registration.status(), StatusCode::BAD_REQUEST);

    let malformed_token = client
        .post(format!("{}/oauth/token", server.base_url))
        .header("content-type", "application/json")
        .body(r#"{"grant_type":"authorization_code"}"#)
        .send()
        .await
        .expect("send malformed OAuth token request");
    assert_eq!(malformed_token.status(), StatusCode::BAD_REQUEST);

    let redirect_uri = "https://chatgpt.com/connector/oauth/http-test";
    let registration = client
        .post(format!("{}/oauth/register", server.base_url))
        .json(&json!({
            "redirect_uris": [redirect_uri],
            "client_name": "ChatGPT HTTP test",
            "application_type": "web",
            "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"]
        }))
        .send()
        .await
        .expect("register OAuth client");
    assert_eq!(registration.status(), StatusCode::CREATED);
    let registration: Value = registration.json().await.expect("decode registration");
    let client_id = registration["client_id"]
        .as_str()
        .expect("registration returns client_id")
        .to_owned();
    assert_eq!(registration["application_type"], "web");

    let verifier =
        "AgenaHttpVerifier-0123456789-abcdefghijklmnopqrstuvwxyz-ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = "agena-http-state";
    let invalid_scope = client
        .get(format!("{}/oauth/authorize", server.base_url))
        .query(&[
            ("response_type", "code"),
            ("client_id", client_id.as_str()),
            ("redirect_uri", redirect_uri),
            ("state", state),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("resource", resource),
            ("scope", "agena:tools forbidden"),
        ])
        .send()
        .await
        .expect("request authorization with an invalid scope");
    assert_eq!(invalid_scope.status(), StatusCode::SEE_OTHER);
    assert_eq!(invalid_scope.headers()["cache-control"], "no-store");
    let invalid_scope_callback = Url::parse(
        invalid_scope
            .headers()
            .get(LOCATION)
            .expect("invalid scope returns callback")
            .to_str()
            .expect("invalid scope callback is UTF-8"),
    )
    .expect("parse invalid scope callback");
    assert_eq!(
        invalid_scope_callback
            .query_pairs()
            .find(|(key, _)| key == "error")
            .map(|(_, value)| value),
        Some("invalid_scope".into())
    );
    assert_eq!(
        invalid_scope_callback
            .query_pairs()
            .find(|(key, _)| key == "state")
            .map(|(_, value)| value),
        Some(state.into())
    );
    assert_eq!(
        invalid_scope_callback
            .query_pairs()
            .find(|(key, _)| key == "iss")
            .map(|(_, value)| value),
        Some(issuer.into())
    );

    let authorization_query = [
        ("response_type", "code"),
        ("client_id", client_id.as_str()),
        ("redirect_uri", redirect_uri),
        ("state", state),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("resource", resource),
        ("scope", "agena:tools offline_access"),
    ];
    let authorization_page = client
        .get(format!("{}/oauth/authorize", server.base_url))
        .query(&authorization_query)
        .send()
        .await
        .expect("open OAuth authorization page");
    assert_eq!(authorization_page.status(), StatusCode::OK);
    assert_eq!(authorization_page.headers()["cache-control"], "no-store");
    assert!(
        authorization_page
            .headers()
            .get("content-security-policy")
            .is_some()
    );

    let authorization_submit = client
        .post(format!("{}/oauth/authorize", server.base_url))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(form_body(&[
            ("response_type", "code"),
            ("client_id", client_id.as_str()),
            ("redirect_uri", redirect_uri),
            ("state", state),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("resource", resource),
            ("scope", "agena:tools offline_access"),
            ("password", password),
        ]))
        .send()
        .await
        .expect("submit OAuth authorization form");
    assert_eq!(authorization_submit.status(), StatusCode::SEE_OTHER);
    assert_eq!(authorization_submit.headers()["cache-control"], "no-store");
    assert_eq!(authorization_submit.headers()["pragma"], "no-cache");
    let callback = Url::parse(
        authorization_submit
            .headers()
            .get(LOCATION)
            .expect("authorization returns callback")
            .to_str()
            .expect("callback is valid UTF-8"),
    )
    .expect("parse OAuth callback");
    assert_eq!(
        callback
            .query_pairs()
            .find(|(key, _)| key == "state")
            .map(|(_, value)| value),
        Some(state.into())
    );
    assert_eq!(
        callback
            .query_pairs()
            .find(|(key, _)| key == "iss")
            .map(|(_, value)| value),
        Some(issuer.into())
    );
    let code = callback
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.into_owned())
        .expect("callback returns authorization code");

    let token_response = client
        .post(format!("{}/oauth/token", server.base_url))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(form_body(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", redirect_uri),
            ("client_id", client_id.as_str()),
            ("code_verifier", verifier),
            ("resource", resource),
        ]))
        .send()
        .await
        .expect("exchange OAuth authorization code");
    assert_eq!(token_response.status(), StatusCode::OK);
    let tokens: Value = token_response.json().await.expect("decode OAuth tokens");
    let access_token = tokens["access_token"].as_str().expect("access token");
    let refresh_token = tokens["refresh_token"].as_str().expect("refresh token");
    assert_eq!(tokens["token_type"], "Bearer");
    assert_eq!(tokens["scope"], "agena:tools offline_access");

    let (status, tools) = post_mcp_with_bearer(
        &client,
        &mcp_url,
        json!({"jsonrpc":"2.0","id":10,"method":"tools/list","params":{}}),
        access_token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        tools["result"]["tools"]
            .as_array()
            .expect("OAuth tools/list array")
            .iter()
            .all(|tool| tool["securitySchemes"]
                == json!([{"type":"oauth2","scopes":["agena:tools"]}]))
    );

    // Re-saving the already-effective control projection is not an identity
    // or policy change and must not revoke an established ChatGPT connection.
    let (status, _, _) = response_json(
        client
            .put(format!("{}/api/v1/server/mcp", server.base_url))
            .bearer_auth(&management_token)
            .json(&json!({
                "enabled": true,
                "authMode": "oauth",
                "anonymousAccess": "none",
                "publicUrl": resource,
                "oauthIssuerUrl": issuer,
                "toolExposure": "read_only",
                "clientRegistration": "cimd_and_dcr"
            }))
            .send()
            .await
            .expect("re-save unchanged MCP control state"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = post_mcp_with_bearer(
        &client,
        &mcp_url,
        json!({"jsonrpc":"2.0","id":101,"method":"tools/list","params":{}}),
        access_token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let invalid = client
        .post(&mcp_url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .bearer_auth("invalid-token")
        .json(&initialize_request(11))
        .send()
        .await
        .expect("request MCP with invalid bearer");
    assert_eq!(invalid.status(), StatusCode::UNAUTHORIZED);
    let challenge = invalid
        .headers()
        .get("www-authenticate")
        .expect("OAuth challenge")
        .to_str()
        .expect("challenge is UTF-8");
    assert!(challenge.contains(
        "resource_metadata=\"https://mcp.example.test/.well-known/oauth-protected-resource/mcp\""
    ));

    let refresh_body = form_body(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id.as_str()),
        ("resource", resource),
    ]);
    let first_refresh = client
        .post(format!("{}/oauth/token", server.base_url))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(refresh_body.clone())
        .send();
    let second_refresh = client
        .post(format!("{}/oauth/token", server.base_url))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(refresh_body)
        .send();
    let (first_refresh, second_refresh) = tokio::join!(first_refresh, second_refresh);
    let first_refresh = first_refresh.expect("send first concurrent refresh");
    let second_refresh = second_refresh.expect("send second concurrent refresh");
    let statuses = [first_refresh.status(), second_refresh.status()];
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::OK)
            .count(),
        1,
        "exactly one concurrent refresh may succeed"
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::BAD_REQUEST)
            .count(),
        1,
        "refresh replay must be rejected"
    );
    let (successful_refresh, rejected_refresh) = if first_refresh.status() == StatusCode::OK {
        (first_refresh, second_refresh)
    } else {
        (second_refresh, first_refresh)
    };
    let refreshed: Value = successful_refresh
        .json()
        .await
        .expect("decode successful concurrent refresh");
    assert!(refreshed["access_token"].as_str().is_some());
    assert!(refreshed["refresh_token"].as_str().is_some());
    assert_eq!(refreshed["scope"], "agena:tools offline_access");
    let rejected_refresh: Value = rejected_refresh
        .json()
        .await
        .expect("decode concurrent refresh replay error");
    assert_eq!(rejected_refresh["error"], "invalid_grant");

    let revoke_response = client
        .post(format!("{}/oauth/revoke", server.base_url))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(form_body(&[
            ("token", access_token),
            ("client_id", client_id.as_str()),
        ]))
        .send()
        .await
        .expect("revoke OAuth token");
    assert_eq!(revoke_response.status(), StatusCode::OK);

    let revoked_access = client
        .post(&mcp_url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .bearer_auth(access_token)
        .json(&initialize_request(12))
        .send()
        .await
        .expect("request MCP with revoked bearer");
    assert_eq!(revoked_access.status(), StatusCode::UNAUTHORIZED);

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
                "oauthIssuerUrl": issuer,
                "toolExposure": "all_non_interactive",
                "clientRegistration": "cimd_only"
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
    assert_eq!(control["authEnabled"], true);
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
                "anonymousAccess": "read_only",
                "toolExposure": "all_non_interactive",
                "clientRegistration": "cimd_and_dcr"
            }))
            .send()
            .await
            .expect("persist the old MCP control state"),
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
        ("AGENA_MCP_TOOL_EXPOSURE", "read-only"),
        ("AGENA_MCP_CLIENT_REGISTRATION", "cimd-only"),
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
    assert_eq!(control["toolExposure"], "read_only");
    assert_eq!(control["clientRegistration"], "cimd_only");
    assert_eq!(control["ready"], true);

    second_child.kill().expect("stop second MCP server");
    second_child.wait().expect("reap second MCP server");
}

async fn post_mcp_with_bearer(
    client: &Client,
    url: &str,
    request: Value,
    token: &str,
) -> (StatusCode, Value) {
    let response = client
        .post(url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .bearer_auth(token)
        .json(&request)
        .send()
        .await
        .expect("send bearer MCP request");
    let (status, _, body) = response_json(response).await;
    (status, body)
}

fn form_body(values: &[(&str, &str)]) -> String {
    values
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                urlencoding::encode(key),
                urlencoding::encode(value)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}
