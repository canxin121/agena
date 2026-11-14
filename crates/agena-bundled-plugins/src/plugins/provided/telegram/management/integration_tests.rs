use super::*;
use agena_assistant::bot::{BotList, BotPersona, BotProfile, BotSession};
use agena_assistant::{
    BindingState, IntegrationBinding, IntegrationControlAction, IntegrationDetail,
    IntegrationsSnapshot,
};
use agena_plugin_sdk::host_api::{EventSubscription, LogLevel};
use agena_plugin_sdk::operations::business::{BusinessCommand, BusinessQuery};
use agena_plugin_sdk::operations::{Command, Query};
use agena_plugin_sdk::{EventEnvelope, EventFilter, ToolInvokeOutput};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

fn ai_id(bot: &str) -> Uuid {
    Uuid::from_u128(if bot == "home" { 2 } else { 1 })
}
fn ai_source(id: Uuid) -> i64 {
    if id == ai_id("home") { 102 } else { 101 }
}
fn ai_profile(id: Uuid) -> BotProfile {
    BotProfile {
        id,
        name: if id == ai_id("home") {
            "Home AI"
        } else {
            "Work AI"
        }
        .into(),
        persona: BotPersona::default(),
        revision: 1,
        archived: false,
        created_at_ms: 0,
        updated_at_ms: 0,
        local_session_id: Some(ai_source(id)),
    }
}
fn binding_uuid(seed: &str) -> Uuid {
    let digest = Sha256::digest(seed.as_bytes());
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    Uuid::from_bytes(bytes)
}

#[derive(Default)]
struct MemoryHost {
    records: Mutex<BTreeMap<String, String>>,
    secrets: Mutex<BTreeMap<String, String>>,
    fail_writes: AtomicBool,
    bindings: Mutex<BTreeMap<Uuid, IntegrationBinding>>,
    canonical_sources: Mutex<BTreeMap<Uuid, i64>>,
    deleted_sources: Mutex<std::collections::BTreeSet<i64>>,
    fail_session_reads: AtomicBool,
}
fn source_owner(source: i64) -> Uuid {
    if source % 1000 == 102 {
        ai_id("home")
    } else {
        ai_id("work")
    }
}
fn missing_session() -> PluginError {
    let mut error = invalid("Session not found");
    error.failure.category = agena_failure::FailureCategory::NotFound;
    error.failure.code = agena_failure::FailureCode::new("resource.not_found");
    error
}
impl MemoryHost {
    fn profile(&self, id: Uuid) -> BotProfile {
        let mut profile = ai_profile(id);
        let source = self
            .canonical_sources
            .lock()
            .unwrap()
            .get(&id)
            .copied()
            .unwrap_or(ai_source(id));
        profile.local_session_id =
            (!self.deleted_sources.lock().unwrap().contains(&source)).then_some(source);
        profile
    }
}
#[async_trait::async_trait]
impl HostClient for MemoryHost {
    async fn log(&self, _: LogLevel, _: String, _: Value) {}
    async fn publish_event(&self, _: EventEnvelope) -> Result<()> {
        Ok(())
    }
    async fn subscribe_events(&self, _: EventFilter) -> Result<EventSubscription> {
        Err(PluginError::internal("unused"))
    }
    async fn read_config(&self, _: Option<String>) -> Result<Value> {
        Ok(json!({}))
    }
    async fn invoke_tool(&self, _: String, _: Value) -> Result<ToolInvokeOutput> {
        Err(PluginError::internal("unused"))
    }
    async fn session_op(
        &self,
        op: agena_plugin_sdk::host_api::SessionOp,
    ) -> Result<agena_plugin_sdk::host_api::SessionOpResponse> {
        use agena_plugin_sdk::host_api::{SessionOp, SessionOpResponse};
        match op {
            SessionOp::Query(query) => self.query(query).await.map(SessionOpResponse::Query),
            SessionOp::Command(command) => {
                self.command(command).await.map(SessionOpResponse::Command)
            }
        }
    }
    async fn query(&self, query: Query) -> Result<Value> {
        let data = match query {
            Query::Business(BusinessQuery::ListBots(_)) => json!(BotList {
                items: vec![self.profile(ai_id("work")), self.profile(ai_id("home"))],
                default_bot_id: Some(ai_id("work")),
            }),
            Query::Business(BusinessQuery::GetBot { bot_id }) => {
                if bot_id != ai_id("work") && bot_id != ai_id("home") {
                    return Err(invalid("AI Bot not found"));
                }
                json!(self.profile(bot_id))
            }
            Query::GetSession(params) => {
                if self.fail_session_reads.load(Ordering::SeqCst) {
                    return Err(PluginError::internal("injected session storage failure"));
                }
                if self
                    .deleted_sources
                    .lock()
                    .unwrap()
                    .contains(&params.session_id)
                {
                    return Err(missing_session());
                }
                json!({ "id": params.session_id })
            }
            Query::Business(BusinessQuery::ListIntegrations { session_id }) => {
                if self.deleted_sources.lock().unwrap().contains(&session_id) {
                    // Matches the assistant service's legacy error mapping:
                    // a missing source is not reported as typed NotFound here.
                    return Err(PluginError::internal("integration source unavailable"));
                }
                json!(IntegrationsSnapshot {
                    source_session_id: session_id,
                    workspace_id: None,
                    providers: vec![],
                    bindings: self
                        .bindings
                        .lock()
                        .unwrap()
                        .values()
                        .filter(|binding| binding.owner_bot_id == Some(source_owner(session_id)))
                        .cloned()
                        .collect(),
                })
            }
            Query::Business(BusinessQuery::GetIntegration {
                session_id,
                binding_id,
            }) => {
                let binding = self
                    .bindings
                    .lock()
                    .unwrap()
                    .get(&binding_id)
                    .cloned()
                    .filter(|binding| binding.owner_bot_id == Some(source_owner(session_id)))
                    .ok_or_else(|| invalid("Binding not found in this AI Bot"))?;
                json!(IntegrationDetail {
                    binding,
                    events: vec![],
                    deliveries: vec![],
                    channel_sessions: vec![],
                    access_audit: vec![]
                })
            }
            _ => return Err(invalid("Unexpected fixture query")),
        };
        Ok(json!({ "result": "data", "data": data }))
    }
    async fn command(&self, command: Command) -> Result<Value> {
        let data = match command {
            Command::Business(BusinessCommand::EnsureBotSession { bot_id }) => {
                let mut sources = self.canonical_sources.lock().unwrap();
                let source = sources.entry(bot_id).or_insert(ai_source(bot_id));
                if self.deleted_sources.lock().unwrap().contains(source) {
                    *source += 1000;
                }
                json!(BotSession::new(*source, &ai_profile(bot_id)))
            }
            Command::Business(BusinessCommand::CreateIntegration {
                session_id,
                request,
            }) => {
                let mut bindings = self.bindings.lock().unwrap();
                if let Some(binding) = bindings.values().find(|binding| {
                    binding.source_session_id == session_id
                        && binding.request_key == request.request_key
                }) {
                    json!(binding)
                } else {
                    let owner = source_owner(session_id);
                    let binding = IntegrationBinding::new(
                        None,
                        Some(owner),
                        session_id,
                        request.request_key,
                        request.spec,
                        "fixture".into(),
                        chrono::Utc::now().timestamp_millis(),
                    )
                    .map_err(|error| invalid(error.to_string()))?;
                    bindings.insert(binding.id, binding.clone());
                    json!(binding)
                }
            }
            Command::Business(BusinessCommand::ControlIntegration {
                session_id,
                binding_id,
                request,
            }) => {
                let mut bindings = self.bindings.lock().unwrap();
                let binding = bindings
                    .get_mut(&binding_id)
                    .filter(|binding| binding.owner_bot_id == Some(source_owner(session_id)))
                    .ok_or_else(|| invalid("Binding not found"))?;
                if binding.revision != request.expected_revision {
                    return Err(invalid("Binding revision conflict"));
                }
                match request.action {
                    IntegrationControlAction::Enable => {
                        binding.state = BindingState::Enabled;
                        binding.activated_at_ms = request
                            .not_before_ms
                            .or(binding.activated_at_ms)
                            .or_else(|| Some(chrono::Utc::now().timestamp_millis()));
                    }
                    IntegrationControlAction::Pause => binding.state = BindingState::Paused,
                    IntegrationControlAction::PollNow => {}
                }
                binding.revision += 1;
                binding.epoch += 1;
                json!(binding)
            }
            _ => return Err(invalid("Unexpected fixture command")),
        };
        Ok(json!({ "result": "data", "data": data }))
    }
    async fn storage_get(
        &self,
        request: HostStorageGetRequest,
    ) -> Result<agena_plugin_sdk::host_api::HostStorageGetResponse> {
        assert_eq!(request.scope, HostStorageScope::Global);
        assert_eq!(request.visibility, HostStorageVisibility::Private);
        Ok(agena_plugin_sdk::host_api::HostStorageGetResponse {
            value: self.records.lock().unwrap().get(&request.key).cloned(),
        })
    }
    async fn storage_set(&self, request: HostStorageSetRequest) -> Result<()> {
        if self.fail_writes.load(Ordering::SeqCst) {
            return Err(PluginError::internal("injected storage failure"));
        }
        self.records
            .lock()
            .unwrap()
            .insert(request.key, request.value);
        Ok(())
    }
    async fn secret_get(
        &self,
        request: HostSecretGetRequest,
    ) -> Result<agena_plugin_sdk::host_api::HostSecretGetResponse> {
        Ok(agena_plugin_sdk::host_api::HostSecretGetResponse {
            value: self.secrets.lock().unwrap().get(&request.name).cloned(),
        })
    }
    async fn secret_set(&self, request: HostSecretSetRequest) -> Result<()> {
        self.secrets
            .lock()
            .unwrap()
            .insert(request.name, request.value);
        Ok(())
    }
    async fn secret_delete(&self, request: HostSecretDeleteRequest) -> Result<()> {
        self.secrets.lock().unwrap().remove(&request.name);
        Ok(())
    }
}
#[derive(Default)]
struct Remote {
    updates: BTreeMap<i64, Vec<Value>>,
    offsets: BTreeMap<i64, Vec<Option<i64>>>,
    sent: Vec<(i64, Value)>,
}
struct BotApi {
    base: String,
    state: Arc<Mutex<Remote>>,
    job: tokio::task::JoinHandle<()>,
}
impl Drop for BotApi {
    fn drop(&mut self) {
        self.job.abort();
    }
}
impl BotApi {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(Remote::default()));
        let shared = state.clone();
        let job = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let shared = shared.clone();
                tokio::spawn(async move {
                    let mut bytes = Vec::new();
                    let mut block = [0u8; 4096];
                    let header_end = loop {
                        let count = socket.read(&mut block).await.unwrap();
                        if count == 0 {
                            return;
                        };
                        bytes.extend_from_slice(&block[..count]);
                        if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                            break index + 4;
                        }
                        if bytes.len() > 65536 {
                            return;
                        }
                    };
                    let header = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
                    let length = header
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    while bytes.len() < header_end + length {
                        let count = socket.read(&mut block).await.unwrap();
                        if count == 0 {
                            return;
                        };
                        bytes.extend_from_slice(&block[..count]);
                    }
                    let path = header
                        .lines()
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .nth(1)
                        .unwrap();
                    let parts = path.split('/').collect::<Vec<_>>();
                    let account = parts[1]
                        .trim_start_matches("bot")
                        .split(':')
                        .next()
                        .unwrap()
                        .parse::<i64>()
                        .unwrap();
                    let method = parts[2];
                    let body: Value =
                        serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
                    let result = {
                        let mut state = shared.lock().unwrap();
                        match method {
                            "getMe" => {
                                json!({"id":account,"is_bot":true,"username":if account==123{"work_bot"}else{"home_bot"}})
                            }
                            "getWebhookInfo" => json!({"url":"","pending_update_count":0}),
                            "getUpdates" => {
                                let offset = body.get("offset").and_then(Value::as_i64);
                                state.offsets.entry(account).or_default().push(offset);
                                let rows = state.updates.entry(account).or_default();
                                if let Some(offset) = offset {
                                    rows.retain(|v| v["update_id"].as_i64().unwrap() >= offset)
                                };
                                json!(rows.clone())
                            }
                            "sendMessage" => {
                                state.sent.push((account, body));
                                json!({"message_id":99})
                            }
                            _ => json!({}),
                        }
                    };
                    let data = serde_json::to_vec(&json!({"ok":true,"result":result})).unwrap();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        data.len()
                    );
                    socket.write_all(response.as_bytes()).await.unwrap();
                    socket.write_all(&data).await.unwrap();
                });
            }
        });
        Self { base, state, job }
    }
    fn put(&self, account: i64, id: i64, chat: i64, kind: &str, text: &str) {
        let key = if kind == "channel" {
            "channel_post"
        } else {
            "message"
        };
        let mut value = json!({"message_id":id,"date":chrono::Utc::now().timestamp(),"chat":{"id":chat,"type":kind,"title":format!("Room {chat}")},"text":text});
        if kind != "channel" {
            value["from"] = json!({"id":if kind == "private" { chat } else { 77 },"first_name":"Member","is_bot":false});
        }
        self.state
            .lock()
            .unwrap()
            .updates
            .entry(account)
            .or_default()
            .push(json!({"update_id":id,key:value}));
    }
}
fn plugin(host: Arc<MemoryHost>) -> TelegramPlugin {
    let plugin = TelegramPlugin::new();
    plugin
        .settings
        .set(TelegramConfig::default(), "test")
        .unwrap();
    assert!(plugin.host.set(host).is_ok());
    plugin
}
fn configuration(api: &BotApi, id: &str, account: &str) -> BotConfiguration {
    BotConfiguration {
        id: id.into(),
        name: id.into(),
        enabled: true,
        credential_kind: "vault".into(),
        ai_bot_id: Some(ai_id(id)),
        chat_access: ChatAccess {
            mode: routing::ChatAccessMode::Blacklist,
            private_enabled: true,
            group_enabled: true,
            channel_enabled: true,
            ..Default::default()
        },
        connection: TelegramConfig {
            account: account.into(),
            api_base: api.base.clone(),
            allow_loopback_http: true,
            ..Default::default()
        },
    }
}
fn register_binding(
    host: &MemoryHost,
    config: &BotConfiguration,
    seed: &str,
    chat_id: &str,
    kind: ConversationKind,
) {
    let resolved = ResolvedBot {
        id: config.id.clone(),
        enabled: config.enabled,
        connection: config.connection.clone(),
        ai_bot_id: config.ai_bot_id,
        source_session_id: Some(ai_source(config.ai_bot_id.unwrap())),
        chat_access: config.chat_access.clone(),
        access_since_ms: 0,
        chat_since_ms: BTreeMap::new(),
    };
    let room = Room {
        chat_id: chat_id.into(),
        title: chat_id.into(),
        kind: kind.as_str().into(),
        ..Default::default()
    };
    let request = routing::default_binding(
        &resolved,
        &room,
        kind,
        chrono::Utc::now().timestamp_millis(),
    )
    .unwrap();
    let mut binding = IntegrationBinding::new(
        None,
        config.ai_bot_id,
        resolved.source_session_id.unwrap(),
        format!("fixture-{seed}"),
        request.spec,
        "fixture".into(),
        chrono::Utc::now().timestamp_millis(),
    )
    .unwrap();
    binding.id = binding_uuid(seed);
    binding.state = BindingState::Enabled;
    binding.activated_at_ms = Some(0);
    binding.epoch = 1;
    host.bindings.lock().unwrap().insert(binding.id, binding);
}

fn poll_request(
    id: &str,
    username: &str,
    binding: &str,
    kind: ConversationKind,
) -> AdapterPollRequest {
    AdapterPollRequest {
        binding_id: binding_uuid(binding).to_string(),
        config: json!({"bot_id":id}),
        cursor: Value::Null,
        limit: 100,
        now_ms: chrono::Utc::now().timestamp_millis(),
        not_before_ms: 0,
        channel_identity: Some(ChannelIdentity {
            tenant: "telegram".into(),
            account: username.into(),
        }),
        channel_kinds: vec![kind],
    }
}
#[tokio::test]
async fn separate_bots_and_chat_consumers_survive_upstream_acknowledgement_and_restart() {
    let api = BotApi::start().await;
    let host = Arc::new(MemoryHost::default());
    let first = plugin(host.clone());
    first
        .save_managed_bot(
            0,
            configuration(&api, "work", "work_bot"),
            Some("123:fixture-work".into()),
        )
        .await
        .unwrap();
    first
        .save_managed_bot(
            1,
            configuration(&api, "home", "home_bot"),
            Some("456:fixture-home".into()),
        )
        .await
        .unwrap();
    register_binding(
        &host,
        &configuration(&api, "work", "work_bot"),
        "a",
        "11",
        ConversationKind::Direct,
    );
    register_binding(
        &host,
        &configuration(&api, "work", "work_bot"),
        "b",
        "-22",
        ConversationKind::Group,
    );
    register_binding(
        &host,
        &configuration(&api, "work", "work_bot"),
        "c",
        "-33",
        ConversationKind::Broadcast,
    );
    register_binding(
        &host,
        &configuration(&api, "home", "home_bot"),
        "d",
        "11",
        ConversationKind::Direct,
    );
    api.put(123, 1, 11, "private", "work-direct");
    api.put(123, 2, -22, "group", "work-group");
    api.put(123, 3, -33, "channel", "work-channel");
    api.put(456, 1, 11, "private", "home-direct");
    let mut a = poll_request("work", "work_bot", "a", ConversationKind::Direct);
    let mut b = poll_request("work", "work_bot", "b", ConversationKind::Group);
    let c = poll_request("work", "work_bot", "c", ConversationKind::Broadcast);
    let d = poll_request("home", "home_bot", "d", ConversationKind::Direct);
    let direct = first.poll_shared(&a).await.unwrap();
    assert_eq!(direct.events.len(), 1);
    assert_eq!(direct.events[0].text, "work-direct");
    a.cursor = direct.cursor;
    let group = first.poll_shared(&b).await.unwrap();
    assert_eq!(group.events.len(), 1);
    assert_eq!(group.events[0].text, "work-group");
    b.cursor = group.cursor;
    assert_eq!(
        first.poll_shared(&c).await.unwrap().events[0].text,
        "work-channel"
    );
    assert_eq!(
        first.poll_shared(&d).await.unwrap().events[0].text,
        "home-direct"
    );
    assert_eq!(
        api.state.lock().unwrap().offsets[&123].len(),
        1,
        "multiple bindings share one upstream pull"
    );
    tokio::time::sleep(Duration::from_millis(1050)).await;
    api.put(123, 4, -22, "group", "after-ack");
    let next = first.poll_shared(&a).await.unwrap();
    assert!(next.events.is_empty());
    assert_eq!(
        api.state.lock().unwrap().offsets[&123].last(),
        Some(&Some(4))
    );
    drop(first);
    let restarted = plugin(host.clone());
    let resumed = restarted.poll_shared(&b).await.unwrap();
    assert_eq!(resumed.events.len(), 1);
    assert_eq!(resumed.events[0].text, "after-ack");
    assert_eq!(
        resumed.events[0].channel.as_ref().unwrap().identity.account,
        "work_bot"
    );
    let mut delivery = ChannelDelivery {
        binding_id: binding_uuid("d").to_string(),
        config: json!({"bot_id":"home"}),
        delivery_id: "reply-1".into(),
        resource: "11".into(),
        thread: None,
        text: "home reply".into(),
        channel: Some(ChannelIdentity {
            tenant: "telegram".into(),
            account: "home_bot".into(),
        }),
        kind: Some(ConversationKind::Direct),
    };
    assert_eq!(
        restarted.send(&delivery).await.unwrap().disposition,
        SendDisposition::Sent
    );
    assert_eq!(
        api.state.lock().unwrap().sent[0].0,
        456,
        "outgoing replies use the selected bot credential"
    );
    delivery.channel.as_mut().unwrap().account = "work_bot".into();
    assert!(restarted.send(&delivery).await.is_err());
    assert_eq!(
        api.state.lock().unwrap().sent.len(),
        1,
        "mismatched identities never send"
    );
    let serialized = serde_json::to_string(&restarted.bot_catalog().await.unwrap()).unwrap();
    assert!(!serialized.contains("fixture-work"));
    assert!(!serialized.contains("fixture-home"));
    assert!(!host.records.lock().unwrap()["bots"].contains("fixture-work"));
}
#[tokio::test]
async fn registry_conflicts_disabled_bots_and_retired_identities_fail_closed() {
    let api = BotApi::start().await;
    let host = Arc::new(MemoryHost::default());
    let plugin = plugin(host.clone());
    let mut config = configuration(&api, "work", "work_bot");
    plugin
        .save_managed_bot(0, config.clone(), Some("123:fixture-work".into()))
        .await
        .unwrap();
    assert!(
        plugin
            .save_managed_bot(0, config.clone(), None)
            .await
            .is_err()
    );
    assert!(
        plugin
            .resolve_bot(
                &BotSelection {
                    bot_id: Some("work".into())
                },
                Some(&ChannelIdentity {
                    tenant: "telegram".into(),
                    account: "home_bot".into()
                })
            )
            .await
            .is_err()
    );
    config.enabled = false;
    plugin
        .save_managed_bot(1, config.clone(), None)
        .await
        .unwrap();
    assert!(
        plugin
            .poll_shared(&poll_request(
                "work",
                "work_bot",
                "a",
                ConversationKind::Direct
            ))
            .await
            .unwrap_err()
            .to_string()
            .contains("disabled")
    );
    plugin.retire_bot(2, "work".into()).await.unwrap();
    assert!(host.secrets.lock().unwrap().is_empty());
    assert!(
        plugin
            .save_managed_bot(3, config, Some("123:fixture-work".into()))
            .await
            .is_err()
    );
}
#[tokio::test]
async fn failed_persistence_does_not_acknowledge_new_updates_or_orphan_credentials() {
    let api = BotApi::start().await;
    let host = Arc::new(MemoryHost::default());
    let plugin = plugin(host.clone());
    host.fail_writes.store(true, Ordering::SeqCst);
    assert!(
        plugin
            .save_managed_bot(
                0,
                configuration(&api, "work", "work_bot"),
                Some("123:fixture-work".into())
            )
            .await
            .is_err()
    );
    assert!(host.secrets.lock().unwrap().is_empty());
    host.fail_writes.store(false, Ordering::SeqCst);
    plugin
        .save_managed_bot(
            0,
            configuration(&api, "work", "work_bot"),
            Some("123:fixture-work".into()),
        )
        .await
        .unwrap();
    register_binding(
        &host,
        &configuration(&api, "work", "work_bot"),
        "a",
        "11",
        ConversationKind::Direct,
    );
    api.put(123, 1, 11, "private", "must-not-be-lost");
    host.fail_writes.store(true, Ordering::SeqCst);
    let request = poll_request("work", "work_bot", "a", ConversationKind::Direct);
    assert!(plugin.poll_shared(&request).await.is_err());
    host.fail_writes.store(false, Ordering::SeqCst);
    let retried = plugin.poll_shared(&request).await.unwrap();
    assert_eq!(retried.events[0].text, "must-not-be-lost");
    assert_eq!(api.state.lock().unwrap().offsets[&123], vec![None, None]);
}
#[tokio::test]
async fn credential_rotation_revalidates_identity_and_removes_stale_verified_status() {
    let api = BotApi::start().await;
    let host = Arc::new(MemoryHost::default());
    let plugin = plugin(host);
    let config = configuration(&api, "work", "work_bot");
    let selection = BotSelection {
        bot_id: Some("work".into()),
    };
    plugin
        .save_managed_bot(0, config.clone(), Some("123:fixture-work".into()))
        .await
        .unwrap();
    assert!(plugin.verify_bot(selection.clone()).await.is_ok());
    assert!(
        plugin
            .save_managed_bot(1, config, Some("456:fixture-home".into()))
            .await
            .is_err()
    );
    assert!(plugin.verify_bot(selection.clone()).await.is_ok());
    let loaded = plugin
        .inspect_bot(selection, false)
        .await
        .unwrap()
        .output
        .unwrap();
    assert_eq!(loaded["verification"]["username"], "work_bot");
    assert!(loaded["last_error"].is_null());
}

#[tokio::test]
async fn maximum_bot_count_includes_the_configured_legacy_connection() {
    let api = BotApi::start().await;
    let host = Arc::new(MemoryHost::default());
    let plugin = TelegramPlugin::new();
    plugin
        .settings
        .set(
            TelegramConfig {
                account: "legacy_bot".into(),
                ..Default::default()
            },
            "test",
        )
        .unwrap();
    assert!(plugin.host.set(host).is_ok());
    let mut registry = Registry::default();
    for index in 0..31 {
        let id = format!("bot_{index}");
        let mut config = configuration(&api, &id, &format!("account_{index}_bot"));
        config.credential_kind = "environment".into();
        registry.bots.insert(
            id,
            StoredBot {
                config,
                secret_name: None,
                updated_at_ms: 0,
                source_session_id: Some(101),
                ai_bot: Some(ai_profile(ai_id("work")).into()),
                access_since_ms: 0,
                chat_since_ms: BTreeMap::new(),
            },
        );
    }
    plugin.write_record("bots", &registry).await.unwrap();
    let failure = plugin
        .save_managed_bot(
            0,
            configuration(&api, "another", "another_bot"),
            Some("123:test".into()),
        )
        .await
        .unwrap_err();
    assert!(failure.to_string().contains("32"));
    assert_eq!(
        plugin.bot_catalog().await.unwrap().output.unwrap()["bots"]
            .as_array()
            .unwrap()
            .len(),
        32
    );
    let mut replacement = registry.bots["bot_0"].config.clone();
    replacement.name = "Updated display name".into();
    assert!(
        plugin.save_managed_bot(0, replacement, None).await.is_ok(),
        "existing bots remain editable at capacity"
    );
}

fn binding_for(host: &MemoryHost, source: i64, chat: &str) -> IntegrationBinding {
    host.bindings
        .lock()
        .unwrap()
        .values()
        .find(|binding| {
            binding.source_session_id == source && binding.spec.resource.single() == Some(chat)
        })
        .unwrap()
        .clone()
}
async fn permit_next_pull(plugin: &TelegramPlugin, id: &str) {
    let resolved = plugin
        .resolve_bot(
            &BotSelection {
                bot_id: Some(id.into()),
            },
            None,
        )
        .await
        .unwrap();
    let key = stream_key(&resolved);
    let mut stream: Stream = plugin.read_record(&key).await.unwrap();
    stream.last_poll_ms = 0;
    plugin.write_record(&key, &stream).await.unwrap();
}

#[tokio::test]
async fn project_and_global_plugin_instances_share_one_durable_collector() {
    let bot_id = "shared-hosts";
    let api = BotApi::start().await;
    let host = Arc::new(MemoryHost::default());
    let project = plugin(host.clone());
    let global = plugin(host.clone());
    project
        .save_managed_bot(
            0,
            configuration(&api, bot_id, "work_bot"),
            Some("123:fixture-work".into()),
        )
        .await
        .unwrap();
    api.put(123, 1, 77, "private", "one account-wide update");

    let (project_result, global_result) = tokio::join!(
        project.synchronize_managed_bot(bot_id, true),
        global.synchronize_managed_bot(bot_id, true),
    );
    project_result.unwrap();
    global_result.unwrap();
    assert_eq!(
        api.state.lock().unwrap().offsets[&123].len(),
        1,
        "both hosts must reuse the same persisted poll instead of competing getUpdates requests"
    );
    let resolved = project
        .resolve_bot(&BotSelection { bot_id: Some(bot_id.into()) }, None)
        .await
        .unwrap();
    let stream: Stream = project.read_record(&stream_key(&resolved)).await.unwrap();
    assert_eq!(stream.offset, Some(2));
    assert_eq!(stream.updates.len(), 1);
    assert_eq!(host.bindings.lock().unwrap().len(), 1);
    assert_eq!(binding_for(&host, 101, "77").state, BindingState::Enabled);
}

#[tokio::test]
async fn token_and_ai_selection_supply_identity_without_manual_source_fields() {
    let api = BotApi::start().await;
    let host = Arc::new(MemoryHost::default());
    let plugin = plugin(host.clone());
    let mut config = configuration(&api, "work", "");
    config.name.clear();
    let saved = plugin
        .save_managed_bot(0, config, Some("123:fixture-work".into()))
        .await
        .unwrap()
        .output
        .unwrap();
    assert_eq!(saved["account"], "work_bot");
    assert_eq!(saved["source_session_id"], 101);
    let bots = plugin.local_ai_bots().await.unwrap().output.unwrap();
    assert_eq!(bots["bots"].as_array().unwrap().len(), 2);
    let registry: Registry = plugin.read_record("bots").await.unwrap();
    assert_eq!(registry.bots["work"].config.name, "work_bot");
    assert_eq!(
        registry.bots["work"].ai_bot.as_ref().unwrap().id,
        ai_id("work")
    );
    let catalog = plugin.bot_catalog().await.unwrap().output.unwrap();
    assert!(!catalog.to_string().contains("fixture-work"));
    assert!(host.bindings.lock().unwrap().is_empty());
}

#[tokio::test]
async fn discovered_chats_follow_live_policy_and_preserve_manual_pauses() {
    let api = BotApi::start().await;
    let host = Arc::new(MemoryHost::default());
    let plugin = plugin(host.clone());
    let mut config = configuration(&api, "work", "work_bot");
    config.chat_access.mode = routing::ChatAccessMode::Whitelist;
    plugin
        .save_managed_bot(0, config.clone(), Some("123:fixture-work".into()))
        .await
        .unwrap();
    api.put(123, 1, 77, "private", "before permission");
    api.put(123, 2, -8, "supergroup", "before group permission");
    api.put(123, 3, -9, "group", "unselected chat");
    plugin.synchronize_managed_bot("work", true).await.unwrap();
    let inventory = plugin
        .inspect_bot(
            BotSelection {
                bot_id: Some("work".into()),
            },
            false,
        )
        .await
        .unwrap()
        .output
        .unwrap();
    assert_eq!(inventory["rooms"].as_array().unwrap().len(), 3);
    assert_eq!(inventory["buffered_updates"], 0);
    assert!(host.bindings.lock().unwrap().is_empty());

    config.chat_access.chat_ids = ["77".into(), "-8".into()].into_iter().collect();
    plugin
        .save_managed_bot(1, config.clone(), None)
        .await
        .unwrap();
    plugin.synchronize_managed_bot("work", false).await.unwrap();
    let direct = binding_for(&host, 101, "77");
    let group = binding_for(&host, 101, "-8");
    assert_eq!(direct.state, BindingState::Enabled);
    assert_eq!(group.state, BindingState::Enabled);
    assert_eq!(group.spec.expires_at_ms, 0);
    assert_eq!(group.spec.total_limit, 0);
    let policy = group.spec.channel_policy.as_ref().unwrap();
    assert!(policy.allow_unlisted_chat);
    assert!(
        policy
            .members
            .iter()
            .all(|member| member.role != agena_assistant::ChannelRole::Owner
                || member.actor.starts_with("agena:bot:"))
    );
    assert!(policy.grant.tools.is_empty());

    api.put(123, 4, 77, "private", "first allowed direct message");
    api.put(123, 5, -8, "supergroup", "first allowed group message");
    permit_next_pull(&plugin, "work").await;
    plugin.synchronize_managed_bot("work", true).await.unwrap();
    let mut request = poll_request("work", "work_bot", "unused", ConversationKind::Direct);
    request.binding_id = direct.id.to_string();
    request.not_before_ms = direct.activated_at_ms.unwrap();
    let batch = plugin.poll_shared(&request).await.unwrap();
    assert_eq!(batch.events.len(), 1);
    assert_eq!(batch.events[0].text, "first allowed direct message");

    // Blacklist rules cover future chats, not just the discovery snapshot.
    config.chat_access.mode = routing::ChatAccessMode::Blacklist;
    config.chat_access.chat_ids = ["-8".into()].into_iter().collect();
    plugin
        .save_managed_bot(2, config.clone(), None)
        .await
        .unwrap();
    plugin.synchronize_managed_bot("work", false).await.unwrap();
    assert_eq!(binding_for(&host, 101, "-8").state, BindingState::Paused);
    let blocked = ChannelDelivery {
        binding_id: group.id.to_string(),
        config: json!({"bot_id":"work"}),
        delivery_id: "blocked".into(),
        resource: "-8".into(),
        thread: None,
        text: "blocked reply".into(),
        channel: Some(ChannelIdentity {
            tenant: "telegram".into(),
            account: "work_bot".into(),
        }),
        kind: Some(ConversationKind::Group),
    };
    assert_eq!(
        plugin.send(&blocked).await.unwrap().disposition,
        SendDisposition::Rejected
    );
    assert!(api.state.lock().unwrap().sent.is_empty());
    api.put(123, 6, -100, "group", "new room after policy change");
    permit_next_pull(&plugin, "work").await;
    plugin.synchronize_managed_bot("work", true).await.unwrap();
    assert_eq!(binding_for(&host, 101, "-100").state, BindingState::Enabled);
    config.chat_access.chat_ids.clear();
    plugin
        .save_managed_bot(3, config.clone(), None)
        .await
        .unwrap();
    plugin.synchronize_managed_bot("work", false).await.unwrap();
    assert_eq!(binding_for(&host, 101, "-8").state, BindingState::Enabled);

    // An operator pause survives subsequent policy toggles.
    {
        let mut bindings = host.bindings.lock().unwrap();
        let binding = bindings.get_mut(&group.id).unwrap();
        binding.state = BindingState::Paused;
        binding.revision += 1;
    }
    config.chat_access.group_enabled = false;
    plugin
        .save_managed_bot(4, config.clone(), None)
        .await
        .unwrap();
    plugin.synchronize_managed_bot("work", false).await.unwrap();
    config.chat_access.group_enabled = true;
    plugin
        .save_managed_bot(5, config.clone(), None)
        .await
        .unwrap();
    plugin.synchronize_managed_bot("work", false).await.unwrap();
    assert_eq!(binding_for(&host, 101, "-8").state, BindingState::Paused);

    // An old source binding cannot consume through the newly selected AI Bot.
    config.ai_bot_id = Some(ai_id("home"));
    plugin
        .save_managed_bot(6, config.clone(), None)
        .await
        .unwrap();
    plugin.synchronize_managed_bot("work", false).await.unwrap();
    assert_eq!(binding_for(&host, 101, "77").state, BindingState::Paused);
    assert_eq!(
        binding_for(&host, 102, "77").owner_bot_id,
        Some(ai_id("home"))
    );
    assert!(plugin.poll_shared(&request).await.is_err());
    config.ai_bot_id = Some(ai_id("work"));
    plugin.save_managed_bot(7, config, None).await.unwrap();
    plugin.synchronize_managed_bot("work", false).await.unwrap();
    assert_eq!(binding_for(&host, 101, "77").state, BindingState::Enabled);
    assert_eq!(binding_for(&host, 101, "-8").state, BindingState::Paused);
}

#[tokio::test]
async fn admission_keeps_same_second_first_message_and_rejects_prepermission_backlog() {
    let api = BotApi::start().await;
    let host = Arc::new(MemoryHost::default());
    let plugin = plugin(host.clone());
    let config = configuration(&api, "work", "work_bot");
    plugin
        .save_managed_bot(0, config.clone(), Some("123:fixture-work".into()))
        .await
        .unwrap();
    register_binding(&host, &config, "timing", "77", ConversationKind::Direct);
    let since = (chrono::Utc::now().timestamp() - 2) * 1000 + 500;
    let mut registry: Registry = plugin.read_record("bots").await.unwrap();
    registry.bots.get_mut("work").unwrap().access_since_ms = since;
    plugin.write_record("bots", &registry).await.unwrap();
    let resolved = plugin
        .resolve_bot(
            &BotSelection {
                bot_id: Some("work".into()),
            },
            None,
        )
        .await
        .unwrap();
    let mut stream = Stream::default();
    let message = |id, seconds, text| {
        json!({"update_id":id,"message":{
            "message_id":id,"date":seconds,"chat":{"id":77,"type":"private"},
            "from":{"id":77,"is_bot":false},"text":text
        }})
    };
    stream
        .append(
            &[message(1, since / 1000, "already buffered")],
            since - 1,
            None,
        )
        .unwrap();
    stream
        .append(
            &[message(2, since / 1000 - 1, "old upstream backlog")],
            since + 1,
            None,
        )
        .unwrap();
    stream
        .append(
            &[message(3, since / 1000, "first allowed message")],
            since + 1,
            None,
        )
        .unwrap();
    plugin
        .write_record(&stream_key(&resolved), &stream)
        .await
        .unwrap();
    let mut request = poll_request("work", "work_bot", "timing", ConversationKind::Direct);
    request.not_before_ms = since.div_euclid(1000) * 1000;
    let batch = plugin.poll_shared(&request).await.unwrap();
    assert_eq!(batch.events.len(), 1);
    assert_eq!(batch.events[0].text, "first allowed message");
    assert_eq!(
        batch.events[0].occurred_at_ms,
        since.div_euclid(1000) * 1000
    );
    assert_eq!(batch.cursor["sequence"], 3);
}

#[tokio::test]
async fn manual_chat_can_be_added_without_telegram_directory_or_owner_id() {
    let api = BotApi::start().await;
    let host = Arc::new(MemoryHost::default());
    let plugin = plugin(host.clone());
    let mut config = configuration(&api, "work", "work_bot");
    config.chat_access.mode = routing::ChatAccessMode::Whitelist;
    config.chat_access.chat_ids = ["-10088".into()].into_iter().collect();
    plugin
        .save_managed_bot(0, config, Some("123:fixture-work".into()))
        .await
        .unwrap();
    let result = plugin
        .add_manual_chat(
            "work".into(),
            "-10088".into(),
            ConversationKind::Group,
            Some("Planning".into()),
        )
        .await
        .unwrap()
        .output
        .unwrap();
    assert_eq!(result["room"]["title"], "Planning");
    assert!(api.state.lock().unwrap().offsets.is_empty());
    plugin.synchronize_managed_bot("work", false).await.unwrap();
    let binding = binding_for(&host, 101, "-10088");
    assert_eq!(binding.state, BindingState::Enabled);
    assert_eq!(binding.owner_bot_id, Some(ai_id("work")));
    assert!(
        plugin
            .add_manual_chat("work".into(), "44".into(), ConversationKind::Group, None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn deleted_canonical_source_rebinds_without_reusing_retired_routes() {
    let api = BotApi::start().await;
    let host = Arc::new(MemoryHost::default());
    let plugin = plugin(host.clone());
    plugin
        .save_managed_bot(
            0,
            configuration(&api, "work", "work_bot"),
            Some("123:fixture-work".into()),
        )
        .await
        .unwrap();
    plugin
        .add_manual_chat("work".into(), "77".into(), ConversationKind::Direct, None)
        .await
        .unwrap();
    plugin.synchronize_managed_bot("work", false).await.unwrap();
    let previous = binding_for(&host, 101, "77");
    host.deleted_sources.lock().unwrap().insert(101);

    // A storage failure is not mistaken for deletion or discarded.
    host.fail_session_reads.store(true, Ordering::SeqCst);
    assert!(
        plugin
            .synchronize_managed_bot("work", false)
            .await
            .unwrap_err()
            .to_string()
            .contains("injected session storage failure")
    );
    host.fail_session_reads.store(false, Ordering::SeqCst);
    plugin.synchronize_managed_bot("work", false).await.unwrap();
    let registry: Registry = plugin.read_record("bots").await.unwrap();
    assert_eq!(registry.bots["work"].source_session_id, Some(1101));
    assert_eq!(
        registry.revision, 1,
        "internal canonical refresh does not overwrite user settings"
    );
    let replacement = binding_for(&host, 1101, "77");
    assert_ne!(replacement.id, previous.id);
    assert_eq!(replacement.state, BindingState::Enabled);
    assert_eq!(binding_for(&host, 101, "77").state, BindingState::Paused);
    plugin.synchronize_managed_bot("work", false).await.unwrap();
    assert_eq!(
        host.bindings.lock().unwrap().len(),
        2,
        "reconciliation stays idempotent"
    );

    let mut old_request = poll_request("work", "work_bot", "unused", ConversationKind::Direct);
    old_request.binding_id = previous.id.to_string();
    assert!(plugin.poll_shared(&old_request).await.is_err());
    api.put(123, 1, 77, "private", "after canonical replacement");
    let mut request = old_request;
    request.binding_id = replacement.id.to_string();
    request.not_before_ms = replacement.activated_at_ms.unwrap();
    assert_eq!(
        plugin.poll_shared(&request).await.unwrap().events[0].text,
        "after canonical replacement"
    );

    // Saving can win the race with background canonical refresh too.
    let mut registry: Registry = plugin.read_record("bots").await.unwrap();
    registry.bots.get_mut("work").unwrap().access_since_ms = 1;
    plugin.write_record("bots", &registry).await.unwrap();
    host.deleted_sources.lock().unwrap().insert(1101);
    plugin
        .save_managed_bot(1, configuration(&api, "work", "work_bot"), None)
        .await
        .unwrap();
    let registry: Registry = plugin.read_record("bots").await.unwrap();
    assert_eq!(registry.bots["work"].source_session_id, Some(2101));
    assert!(registry.bots["work"].access_since_ms > 1);
    assert!(registry.bots["work"].chat_since_ms.is_empty());
}
