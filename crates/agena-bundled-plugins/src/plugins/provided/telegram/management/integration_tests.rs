use super::*;
use agena_plugin_sdk::host_api::{EventSubscription, LogLevel};
use agena_plugin_sdk::{EventEnvelope, EventFilter, ToolInvokeOutput};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Default)]
struct MemoryHost {
    records: Mutex<BTreeMap<String, String>>,
    secrets: Mutex<BTreeMap<String, String>>,
    fail_writes: AtomicBool,
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
            value["from"] = json!({"id":77,"first_name":"Member","is_bot":false});
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
        connection: TelegramConfig {
            account: account.into(),
            api_base: api.base.clone(),
            allow_loopback_http: true,
            ..Default::default()
        },
    }
}
fn poll_request(
    id: &str,
    username: &str,
    binding: &str,
    kind: ConversationKind,
) -> AdapterPollRequest {
    AdapterPollRequest {
        binding_id: binding.into(),
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
        binding_id: "d".into(),
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
    plugin
        .save_managed_bot(1, config, Some("456:fixture-home".into()))
        .await
        .unwrap();
    assert!(plugin.verify_bot(selection.clone()).await.is_err());
    let loaded = plugin
        .inspect_bot(selection, false)
        .await
        .unwrap()
        .output
        .unwrap();
    assert!(loaded["verification"].is_null());
    assert!(loaded["last_error"].as_str().unwrap().contains("home_bot"));
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
