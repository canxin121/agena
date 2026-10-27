//! End-to-end external-client fixtures. Python 3 is required by repository CI.
#![cfg(unix)]

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agena_mcp_client::{
    ContentBlock, FileOAuthCredentialStore, FileTokenStore, HttpAuth, McpConnectionManager,
    McpError, McpOAuthNetworkAuthorizer, McpOAuthRegistration, McpOAuthSessionManager,
    McpToolPolicy, OAuthCredentialState, ReconnectPolicy, ServerSpec, credential_store_name,
    oauth_credential_binding,
};
use agena_secret_store::{SecretStore, SecretStoreError};
use rmcp::transport::CredentialStore;
use serde_json::Value;
use tempfile::TempDir;
use url::Url;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/server.py")
}

fn stdio_spec(root: &Path, protocol: &str, capabilities: &str, notifications: bool) -> ServerSpec {
    let mut args = vec![
        fixture_path().to_string_lossy().into_owned(),
        "--protocol".into(),
        protocol.into(),
        "--capabilities".into(),
        capabilities.into(),
        "--log".into(),
        root.join("rpc.jsonl").to_string_lossy().into_owned(),
    ];
    if notifications {
        args.push("--notifications".into());
    }
    ServerSpec::Stdio {
        command: "python3".into(),
        args,
        env: HashMap::new(),
        cwd: Some(root.to_path_buf()),
        tool_policy: McpToolPolicy::default(),
    }
}

fn manager() -> McpConnectionManager {
    McpConnectionManager::new("fixture-client", "1.0")
        .with_timeouts(Duration::from_secs(4), Duration::from_secs(2))
}

fn records(root: &Path) -> Vec<Value> {
    std::fs::read_to_string(root.join("rpc.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn has_method(root: &Path, method: &str) -> bool {
    records(root).iter().any(|entry| entry["method"] == method)
}

fn text_content(result: agena_mcp_client::CallToolResult) -> String {
    result
        .content
        .into_iter()
        .find_map(|content| match content {
            ContentBlock::Text { text, .. } => Some(text),
            _ => None,
        })
        .expect("fixture returned text")
}

struct HttpFixture {
    child: Child,
    root: TempDir,
    endpoint: Url,
}

impl HttpFixture {
    async fn start(protocol: &str, oauth: bool) -> Self {
        Self::start_registration(protocol, oauth, "dcr").await
    }

    async fn start_registration(protocol: &str, oauth: bool, registration: &str) -> Self {
        Self::start_options(protocol, oauth, registration, None).await
    }

    async fn start_options(
        protocol: &str,
        oauth: bool,
        registration: &str,
        issuer: Option<&str>,
    ) -> Self {
        Self::start_authorization_endpoint(protocol, oauth, registration, issuer, None).await
    }

    async fn start_authorization_endpoint(
        protocol: &str,
        oauth: bool,
        registration: &str,
        issuer: Option<&str>,
        authorization_url: Option<&str>,
    ) -> Self {
        let root = tempfile::tempdir().unwrap();
        let port_file = root.path().join("port");
        let mut command = Command::new("python3");
        command
            .arg(fixture_path())
            .args([
                "--transport",
                "http",
                "--protocol",
                protocol,
                "--capabilities",
                "mixed",
            ])
            .arg("--log")
            .arg(root.path().join("rpc.jsonl"))
            .arg("--port-file")
            .arg(&port_file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        if oauth {
            command.arg("--oauth");
        }
        command.args(["--registration", registration]);
        if let Some(issuer) = issuer {
            command.args(["--issuer-base", issuer]);
        }
        if let Some(url) = authorization_url {
            command.args(["--authorization-url", url]);
        }
        let child = command
            .spawn()
            .expect("Python 3 is required for MCP fixtures");
        // Construct the guard immediately so a failed startup also terminates the child.
        let mut fixture = Self {
            child,
            root,
            endpoint: Url::parse("http://127.0.0.1/").unwrap(),
        };
        fixture.endpoint = tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                if let Ok(base) = std::fs::read_to_string(&port_file)
                    && let Ok(endpoint) = Url::parse(&format!("{base}/mcp"))
                {
                    break endpoint;
                }
                assert!(
                    fixture.child.try_wait().unwrap().is_none(),
                    "HTTP fixture exited before startup"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("HTTP fixture started within deadline");
        fixture
    }

    fn spec(&self, auth: Option<HttpAuth>) -> ServerSpec {
        ServerSpec::Http {
            url: self.endpoint.clone(),
            headers: HashMap::new(),
            auth,
            tool_policy: McpToolPolicy::default(),
        }
    }
}

impl Drop for HttpFixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[tokio::test]
async fn legacy_resource_and_prompt_only_server_connects_without_tools_list() {
    let root = tempfile::tempdir().unwrap();
    let manager = manager();
    manager
        .add_server(
            "notes",
            stdio_spec(root.path(), "legacy", "resources", false),
        )
        .await
        .unwrap();
    assert!(manager.all_tools().await.is_empty());
    assert!(manager.refresh_tools("notes").await.unwrap().is_empty());
    assert_eq!(
        manager
            .list_resources("notes", None)
            .await
            .unwrap()
            .resources
            .len(),
        1
    );
    assert_eq!(
        manager
            .list_resource_templates("notes", None)
            .await
            .unwrap()
            .resource_templates
            .len(),
        1
    );
    assert_eq!(
        manager
            .read_resource("notes", "fixture://notes")
            .await
            .unwrap()
            .contents[0]
            .text
            .as_deref(),
        Some("Fixture notes")
    );
    assert_eq!(
        manager
            .list_prompts("notes", None)
            .await
            .unwrap()
            .prompts
            .len(),
        1
    );
    assert_eq!(
        manager
            .get_prompt("notes", "review", None)
            .await
            .unwrap()
            .messages
            .len(),
        1
    );
    assert!(matches!(
        manager.call_tool("notes", "missing", None).await,
        Err(McpError::CapabilityUnavailable { .. })
    ));
    let status = manager.statuses().await.remove(0);
    assert!(status.connected);
    assert_eq!(status.protocol_version.as_deref(), Some("2025-11-25"));
    assert!(status.capabilities.unwrap().get("resources").is_some());
    assert!(!has_method(root.path(), "tools/list"));
    assert!(has_method(root.path(), "initialize"));
    manager.shutdown_all().await;
}

#[tokio::test]
async fn modern_stdio_discovers_and_receives_catalog_subscription_changes() {
    let root = tempfile::tempdir().unwrap();
    let manager = manager();
    manager
        .add_server("modern", stdio_spec(root.path(), "modern", "tools", true))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if manager
                .all_tools()
                .await
                .iter()
                .any(|(_, tool)| tool.name == "updated")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("subscribed tool-list change refreshed the catalog");
    let status = manager.statuses().await.remove(0);
    assert_eq!(status.protocol_version.as_deref(), Some("2026-07-28"));
    assert_eq!(status.server_info.unwrap()["name"], "fixture");
    assert!(status.tool_generation > 0);
    assert!(has_method(root.path(), "server/discover"));
    assert!(has_method(root.path(), "subscriptions/listen"));
    assert!(!has_method(root.path(), "initialize"));
    let listing = records(root.path())
        .into_iter()
        .find(|entry| entry["method"] == "tools/list")
        .unwrap();
    assert_eq!(listing["metadata"]["version"], "2026-07-28");
    assert!(listing["metadata"]["capabilities"].is_object());
    assert!(
        manager
            .list_resources("modern", None)
            .await
            .unwrap()
            .resources
            .is_empty()
    );
    assert!(!has_method(root.path(), "resources/list"));
    manager.shutdown_all().await;
}

#[tokio::test]
async fn closed_stdio_transport_is_hidden_and_supervisor_reconnects_it() {
    let root = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager());
    manager
        .add_server("restart", stdio_spec(root.path(), "legacy", "tools", false))
        .await
        .unwrap();
    let first = text_content(manager.call_tool("restart", "ping", None).await.unwrap());
    assert!(
        manager
            .call_tool("restart", "disconnect", None)
            .await
            .is_err()
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if !manager.statuses().await[0].connected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(manager.all_tools().await.is_empty());
    assert_eq!(manager.statuses().await[0].tool_count, 0);
    manager.start_reconnect_supervisor(ReconnectPolicy::new(
        Duration::from_millis(10),
        Duration::from_millis(40),
        Duration::from_millis(10),
    ));
    let second = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(result) = manager.call_tool("restart", "ping", None).await {
                break text_content(result);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("closed transport was reconnected");
    assert_ne!(first, second, "reconnect launched a fresh child process");
    manager.remove_server("restart").await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert!(manager.statuses().await.is_empty());
    assert!(manager.all_tools().await.is_empty());
    manager.shutdown_all().await;
}

#[tokio::test]
async fn per_connection_request_timeout_applies_to_calls() {
    let root = tempfile::tempdir().unwrap();
    let manager = manager();
    manager
        .add_server_with_timeouts(
            "slow",
            stdio_spec(root.path(), "legacy", "tools", false),
            Duration::from_secs(4),
            Duration::from_millis(40),
        )
        .await
        .unwrap();
    assert!(matches!(
        manager.call_tool("slow", "slow", None).await,
        Err(McpError::Timeout(_))
    ));
    manager.shutdown_all().await;
}

#[tokio::test]
async fn streamable_http_supports_both_lifecycles_and_redacts_status_urls() {
    for (protocol, expected) in [("modern", "2026-07-28"), ("legacy", "2025-11-25")] {
        let mut fixture = HttpFixture::start(protocol, false).await;
        fixture.endpoint.set_query(Some("token=private-query"));
        let manager = manager();
        manager
            .add_server("http", fixture.spec(None))
            .await
            .unwrap();
        assert!(!text_content(manager.call_tool("http", "ping", None).await.unwrap()).is_empty());
        assert_eq!(
            manager
                .list_resources("http", None)
                .await
                .unwrap()
                .resources
                .len(),
            1
        );
        let status = manager.statuses().await.remove(0);
        assert_eq!(status.protocol_version.as_deref(), Some(expected));
        assert!(!status.network_target.unwrap().contains("private-query"));
        assert!(
            manager.server_network_targets().await["http"].contains("private-query"),
            "permission checks retain the actual target"
        );
        if protocol == "modern" {
            assert!(!has_method(fixture.root.path(), "initialize"));
            let calls = records(fixture.root.path());
            let call = calls
                .iter()
                .find(|entry| entry["method"] == "tools/call")
                .unwrap();
            assert_eq!(call["metadata"]["version"], expected);
            assert_eq!(call["metadata"]["header_version"], expected);
        } else {
            assert!(has_method(fixture.root.path(), "initialize"));
        }
        manager.shutdown_all().await;
    }
}

#[derive(Default)]
struct MemorySecrets(Mutex<BTreeMap<String, String>>);
impl SecretStore for MemorySecrets {
    fn get_secret(&self, key: &str) -> Result<Option<String>, SecretStoreError> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn set_secret(&self, key: &str, value: &str) -> Result<(), SecretStoreError> {
        self.0.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn delete_secret(&self, key: &str) -> Result<(), SecretStoreError> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

#[tokio::test]
async fn stored_bearer_is_bound_to_endpoint_and_legacy_credentials_are_not_sent() {
    let fixture = HttpFixture::start("modern", false).await;
    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecrets::default());
    let store = FileTokenStore::with_secret_store(secrets);
    store.put_bearer("bound", "legacy-private").unwrap();
    let other = Url::parse("http://127.0.0.1:1/mcp").unwrap();
    store
        .put_bearer(&credential_store_name("bound", &other), "other-private")
        .unwrap();
    let mut manager = manager();
    manager.set_token_store(Arc::new(store.clone()));
    let error = manager
        .add_server("bound", fixture.spec(Some(HttpAuth::BearerFromStore)))
        .await
        .unwrap_err();
    assert!(matches!(error, McpError::Auth(_)));
    assert!(
        !has_method(fixture.root.path(), "server/discover"),
        "no request is sent with an unrelated stored token"
    );
    store
        .put_bearer(
            &credential_store_name("bound", &fixture.endpoint),
            "current-private",
        )
        .unwrap();
    manager.reconnect("bound").await.unwrap();
    assert!(has_method(fixture.root.path(), "server/discover"));
    manager.shutdown_all().await;
}

#[tokio::test]
async fn oauth_pkce_completion_checks_binding_and_logout_preserves_bearer() {
    let fixture = HttpFixture::start("modern", true).await;
    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecrets::default());
    let sessions = McpOAuthSessionManager::with_secret_store(Arc::clone(&secrets));
    let identity = credential_store_name("oauth", &fixture.endpoint);
    let bearer = FileTokenStore::with_secret_store(Arc::clone(&secrets));
    bearer.put_bearer(&identity, "manual-private").unwrap();
    let flow = sessions
        .begin(
            "oauth",
            fixture.endpoint.clone(),
            vec!["read".into()],
            "http://127.0.0.1:4020/settings/mcp-connections",
            "revision-one".into(),
        )
        .await
        .unwrap();
    assert!(chrono::DateTime::parse_from_rfc3339(&flow.expires_at).is_ok());
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let authorization = client.get(&flow.authorization_url).send().await.unwrap();
    let callback = authorization
        .headers()
        .get(reqwest::header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let error = sessions
        .complete(&flow.flow_id, &callback, "revision-two")
        .await
        .unwrap_err();
    assert!(!error.to_string().contains("fixture-code"));
    assert!(
        !has_method(fixture.root.path(), "POST /token"),
        "changed connection binding is rejected before code exchange"
    );
    sessions
        .complete(&flow.flow_id, &callback, "revision-one")
        .await
        .unwrap();
    assert!(has_method(fixture.root.path(), "POST /token"));
    let oauth =
        FileOAuthCredentialStore::with_secret_store(&identity, Arc::clone(&secrets)).unwrap();
    assert_eq!(
        oauth.health().credential_state,
        OAuthCredentialState::Configured
    );
    let bound = oauth.clone().with_binding(oauth_credential_binding(
        &["read".into()],
        &McpOAuthRegistration::default(),
    ));
    let retained_credentials = bound.load().await.unwrap().unwrap();
    let changed_scopes = oauth.clone().with_binding(oauth_credential_binding(
        &["write".into()],
        &McpOAuthRegistration::default(),
    ));
    assert!(changed_scopes.load().await.unwrap().is_none());
    assert_eq!(
        changed_scopes.health().credential_state,
        OAuthCredentialState::Missing
    );
    changed_scopes.clear().await.unwrap();
    assert!(
        bound.load().await.unwrap().is_some(),
        "a mismatched connection cannot clear another binding"
    );
    let changed_registration = oauth.clone().with_binding(oauth_credential_binding(
        &["read".into()],
        &McpOAuthRegistration {
            client_id: Some("different-client".into()),
            ..Default::default()
        },
    ));
    assert!(changed_registration.load().await.unwrap().is_none());
    assert!(
        sessions
            .complete(&flow.flow_id, &callback, "revision-one")
            .await
            .is_err(),
        "authorization callbacks are single use"
    );
    sessions.logout("oauth", &fixture.endpoint).await.unwrap();
    assert_eq!(
        oauth.health().credential_state,
        OAuthCredentialState::Missing
    );
    assert_eq!(
        bearer.bearer_result(&identity).unwrap().as_deref(),
        Some("manual-private")
    );
    let pending = sessions
        .begin(
            "oauth",
            fixture.endpoint.clone(),
            vec!["read".into()],
            "http://127.0.0.1:4020/settings/mcp-connections",
            "id:revision-one".into(),
        )
        .await
        .unwrap();
    assert!(
        sessions
            .cancel_checked(&pending.flow_id, "other-id:revision-one")
            .await
            .is_err()
    );
    assert!(
        sessions
            .cancel_checked(&pending.flow_id, "id:revision-one")
            .await
            .unwrap()
    );
    assert!(
        !sessions
            .cancel_checked(&pending.flow_id, "id:revision-one")
            .await
            .unwrap()
    );
    // Simulate an older unbound credential document at the endpoint key.
    oauth.save(retained_credentials).await.unwrap();
    assert!(oauth.load().await.unwrap().is_some());
    assert!(
        bound.load().await.unwrap().is_none(),
        "legacy documents require explicit reauthorization"
    );
    sessions.shutdown().await;
}

#[tokio::test]
async fn oauth_pre_registered_and_metadata_clients_work_without_dynamic_registration() {
    for mode in ["preregistered", "metadata"] {
        let fixture = HttpFixture::start_registration("modern", true, mode).await;
        let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecrets::default());
        let sessions = McpOAuthSessionManager::with_secret_store(Arc::clone(&secrets));
        let (registration, expected_client) = if mode == "preregistered" {
            (
                McpOAuthRegistration {
                    client_id: Some("pre-registered".into()),
                    client_secret: Some("registration-secret".into()),
                    client_metadata_url: None,
                },
                "pre-registered",
            )
        } else {
            (
                McpOAuthRegistration {
                    client_metadata_url: Some("https://client.example/metadata.json".into()),
                    ..Default::default()
                },
                "https://client.example/metadata.json",
            )
        };
        assert!(!format!("{registration:?}").contains("registration-secret"));
        let flow = sessions
            .begin_with_registration(
                "external",
                fixture.endpoint.clone(),
                vec!["read".into()],
                "http://127.0.0.1:4020/settings/mcp-connections",
                "connection:revision".into(),
                registration,
            )
            .await
            .unwrap();
        let url = Url::parse(&flow.authorization_url).unwrap();
        assert_eq!(
            url.query_pairs()
                .find(|(key, _)| key == "client_id")
                .unwrap()
                .1,
            expected_client
        );
        assert!(!flow.authorization_url.contains("registration-secret"));
        assert!(!has_method(fixture.root.path(), "POST /register"));
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let response = client.get(flow.authorization_url).send().await.unwrap();
        let callback = response
            .headers()
            .get(reqwest::header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap();
        sessions
            .complete(&flow.flow_id, callback, "connection:revision")
            .await
            .unwrap();
        let store = FileOAuthCredentialStore::with_secret_store(
            credential_store_name("external", &fixture.endpoint),
            secrets,
        )
        .unwrap();
        assert_eq!(
            store.health().credential_state,
            OAuthCredentialState::Configured
        );
        assert!(!has_method(fixture.root.path(), "POST /register"));
        sessions.shutdown().await;
    }
}

struct OAuthNetworkPolicy {
    allowed_origin: Option<url::Origin>,
    targets: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl McpOAuthNetworkAuthorizer for OAuthNetworkPolicy {
    async fn authorize(&self, target: &str) -> agena_mcp_client::McpResult<()> {
        self.targets.lock().unwrap().push(target.to_owned());
        if self
            .allowed_origin
            .as_ref()
            .is_some_and(|origin| *origin == Url::parse(target).unwrap().origin())
        {
            Ok(())
        } else {
            Err(McpError::PermissionDenied)
        }
    }
}

#[tokio::test]
async fn oauth_denied_cross_origin_issuer_performs_no_registration_or_token_request() {
    let issuer = HttpFixture::start("modern", true).await;
    let issuer_base = issuer.endpoint.origin().ascii_serialization();
    let resource = HttpFixture::start_options("modern", true, "dcr", Some(&issuer_base)).await;
    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecrets::default());
    let sessions = McpOAuthSessionManager::with_secret_store(secrets);
    let policy = Arc::new(OAuthNetworkPolicy {
        allowed_origin: Some(resource.endpoint.origin()),
        targets: Mutex::new(Vec::new()),
    });
    let error = sessions
        .begin_authorized(
            "guarded",
            resource.endpoint.clone(),
            vec!["read".into()],
            "http://127.0.0.1:4020/settings/mcp-connections",
            "id:revision".into(),
            McpOAuthRegistration::default(),
            policy.clone(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, McpError::PermissionDenied));
    assert!(
        policy
            .targets
            .lock()
            .unwrap()
            .iter()
            .any(|target| target.starts_with(&issuer_base))
    );
    assert!(
        records(issuer.root.path()).is_empty(),
        "denied issuer receives no request, including discovery"
    );
    assert!(!has_method(resource.root.path(), "POST /register"));
    assert!(!has_method(resource.root.path(), "POST /token"));
    assert_eq!(
        Arc::strong_count(&policy),
        1,
        "failed OAuth operation releases callback authority"
    );
    sessions.shutdown().await;
}

#[tokio::test]
async fn oauth_completion_uses_new_authority_and_pending_flow_retains_none() {
    let fixture = HttpFixture::start("modern", true).await;
    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecrets::default());
    let sessions = McpOAuthSessionManager::with_secret_store(secrets);
    let start_policy = Arc::new(OAuthNetworkPolicy {
        allowed_origin: Some(fixture.endpoint.origin()),
        targets: Mutex::new(Vec::new()),
    });
    let flow = sessions
        .begin_authorized(
            "guarded",
            fixture.endpoint.clone(),
            vec!["read".into()],
            "http://127.0.0.1:4020/settings/mcp-connections",
            "id:revision".into(),
            McpOAuthRegistration::default(),
            start_policy.clone(),
        )
        .await
        .unwrap();
    assert_eq!(
        Arc::strong_count(&start_policy),
        1,
        "pending PKCE state must not retain start-call authority"
    );
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let response = client.get(flow.authorization_url).send().await.unwrap();
    let callback = response
        .headers()
        .get(reqwest::header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    let complete_policy = Arc::new(OAuthNetworkPolicy {
        allowed_origin: None,
        targets: Mutex::new(Vec::new()),
    });
    let error = sessions
        .complete_authorized(
            &flow.flow_id,
            callback,
            "id:revision",
            complete_policy.clone(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, McpError::PermissionDenied));
    assert!(
        !has_method(fixture.root.path(), "POST /token"),
        "the current callback policy denied token exchange"
    );
    assert_eq!(Arc::strong_count(&complete_policy), 1);
    sessions.shutdown().await;
}

#[tokio::test]
async fn oauth_rejects_non_http_authorization_urls_before_returning_a_browser_link() {
    for endpoint in [
        "javascript:alert(1)",
        "data:text/html,unsafe",
        "ftp://provider.test/authorize",
    ] {
        let fixture =
            HttpFixture::start_authorization_endpoint("modern", true, "dcr", None, Some(endpoint))
                .await;
        let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecrets::default());
        let sessions = McpOAuthSessionManager::with_secret_store(secrets);
        let error = sessions
            .begin(
                "unsafe-link",
                fixture.endpoint.clone(),
                vec!["read".into()],
                "http://127.0.0.1:4020/settings/mcp-connections",
                "id:revision".into(),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, McpError::Auth(_)));
        assert!(
            !error.to_string().contains(endpoint),
            "provider URLs are not echoed in errors"
        );
        assert!(!has_method(fixture.root.path(), "POST /token"));
        sessions.shutdown().await;
    }
}
