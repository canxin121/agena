#![cfg(unix)]

use std::{
    fs::OpenOptions,
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

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn start_server() -> TestServer {
    let fixture = tempdir().expect("create MCP HTTP fixture");
    let workspace = fixture.path().join("workspace");
    let server_data = fixture.path().join("server-data");
    let database = fixture.path().join("server.db");
    let log_path = fixture.path().join("server.log");
    std::fs::create_dir_all(&workspace).expect("create MCP HTTP workspace");
    std::fs::create_dir_all(&server_data).expect("create MCP HTTP server data");

    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("reserve MCP HTTP port");
    let port = listener
        .local_addr()
        .expect("read reserved MCP HTTP port")
        .port();
    drop(listener);

    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .expect("open MCP HTTP server log");
    let stderr = log.try_clone().expect("clone MCP HTTP server log");
    let child = Command::new(env!("CARGO_BIN_EXE_agena"))
        .arg("--database-path")
        .arg(&database)
        .arg("server")
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--workspace")
        .arg(&workspace)
        .env("AGENA_SERVER_DATA_DIR", &server_data)
        .env_remove("AGENA_SERVER_UI_PASSWORD")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr))
        .spawn()
        .expect("spawn MCP HTTP server");

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
            std::fs::read_to_string(&log_path).unwrap_or_default()
        );
        sleep(Duration::from_millis(50)).await;
    }

    TestServer {
        child,
        base_url,
        _fixture: fixture,
    }
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
    let response = client
        .post(url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
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

    let (status, discover) = post_mcp(
        &client,
        &mcp_url,
        json!({
            "jsonrpc": "2.0",
            "id": "discover",
            "method": "server/discover",
            "params": {}
        }),
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
    assert_eq!(discover["result"]["cacheScope"], "public");

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

    // Unsupported MCP methods must stay at the JSON-RPC layer. In
    // particular, they must not fall through to rmcp's stateful dispatcher,
    // which reports a pre-initialize request as HTTP 422.
    let (status, unsupported) = post_mcp(
        &client,
        &mcp_url,
        json!({"jsonrpc":"2.0","id":20,"method":"resources/list","params":{}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(unsupported["error"]["code"], -32601);

    let (status, unknown) = post_mcp(
        &client,
        &mcp_url,
        json!({"jsonrpc":"2.0","id":21,"method":"agena/unknown","params":{}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(unknown["error"]["code"], -32601);

    let (status, parse_error) = post_mcp_raw(&client, &mcp_url, br"{not-json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(parse_error["error"]["code"], -32700);

    let (status, batch) = post_mcp(
        &client,
        &mcp_url,
        json!([
            initialize_request(22),
            {"jsonrpc":"2.0","id":23,"method":"agena/unknown","params":{}}
        ]),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(batch.as_array().expect("JSON-RPC batch response").len(), 2);
    assert_eq!(batch[1]["error"]["code"], -32601);

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
    let server = start_server().await;
    let client = test_client();
    let mcp_url = format!("{}/mcp", server.base_url);
    let resource = "https://mcp.example.test/mcp";
    let password = "MCP-HTTP-test-password-2026";

    let (status, _, _) = response_json(
        client
            .put(format!("{}/api/v1/server/mcp", server.base_url))
            .json(&json!({
                "enabled": true,
                "authEnabled": false,
                "publicUrl": resource
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
            .json(&json!({"password": password}))
            .send()
            .await
            .expect("set MCP OAuth password"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let oauth_enable_response = client
        .put(format!("{}/api/v1/server/mcp", server.base_url))
        .json(&json!({"enabled": true, "authEnabled": true}))
        .send()
        .await
        .expect("send OAuth enable request");
    let (status, _, control) = response_json(oauth_enable_response).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(control["ready"], true);

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
        json!(["agena:tools"])
    );

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

    let verifier =
        "AgenaHttpVerifier-0123456789-abcdefghijklmnopqrstuvwxyz-ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = "agena-http-state";
    let authorization_query = [
        ("response_type", "code"),
        ("client_id", client_id.as_str()),
        ("redirect_uri", redirect_uri),
        ("state", state),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("resource", resource),
        ("scope", "agena:tools"),
    ];
    let authorization_page = client
        .get(format!("{}/oauth/authorize", server.base_url))
        .query(&authorization_query)
        .send()
        .await
        .expect("open OAuth authorization page");
    assert_eq!(authorization_page.status(), StatusCode::OK);

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
            ("scope", "agena:tools"),
            ("password", password),
        ]))
        .send()
        .await
        .expect("submit OAuth authorization form");
    assert_eq!(authorization_submit.status(), StatusCode::SEE_OTHER);
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
    assert_eq!(tokens["scope"], "agena:tools");

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
    assert!(invalid.headers().get("www-authenticate").is_some());

    let refresh_response = client
        .post(format!("{}/oauth/token", server.base_url))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(form_body(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client_id.as_str()),
            ("resource", resource),
        ]))
        .send()
        .await
        .expect("refresh OAuth token");
    assert_eq!(refresh_response.status(), StatusCode::OK);
    let refreshed: Value = refresh_response
        .json()
        .await
        .expect("decode refreshed tokens");
    assert!(refreshed["access_token"].as_str().is_some());
    assert!(refreshed["refresh_token"].as_str().is_some());

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

    let (status, _, _) = response_json(
        client
            .put(format!("{}/api/v1/server/mcp", server.base_url))
            .json(&json!({"enabled":true,"authEnabled":false}))
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
