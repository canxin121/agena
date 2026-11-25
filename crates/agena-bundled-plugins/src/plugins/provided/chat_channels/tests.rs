use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use aes::Aes256;
use agena_assistant::ChannelWebhookEnvelope;
use agena_plugin_sdk::{
    ToolInvokeOutput,
    hooks::{EventEnvelope, EventFilter},
    host_api::{
        EventSubscription, HostStorageGetRequest, HostStorageGetResponse, HostStorageSetRequest,
        LogLevel, NoopHostClient,
    },
    integration::ConversationKind,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use cbc::cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;

const NOW: i64 = 1_800_000_000_000;
const FIXTURE_ENV: &str = "PATH";

#[derive(Default)]
struct Storage {
    records: Mutex<BTreeMap<String, String>>,
    pause_first_write: Option<PausedWrite>,
}

#[derive(Default)]
struct PausedWrite {
    entered: tokio::sync::Notify,
    resume: tokio::sync::Notify,
    started: AtomicBool,
}

#[async_trait::async_trait]
impl HostClient for Storage {
    async fn log(&self, _: LogLevel, _: String, _: Value) {}

    async fn publish_event(&self, event: EventEnvelope) -> Result<()> {
        NoopHostClient.publish_event(event).await
    }

    async fn subscribe_events(&self, filter: EventFilter) -> Result<EventSubscription> {
        NoopHostClient.subscribe_events(filter).await
    }

    async fn read_config(&self, path: Option<String>) -> Result<Value> {
        NoopHostClient.read_config(path).await
    }

    async fn invoke_tool(&self, name: String, input: Value) -> Result<ToolInvokeOutput> {
        NoopHostClient.invoke_tool(name, input).await
    }

    async fn storage_get(&self, req: HostStorageGetRequest) -> Result<HostStorageGetResponse> {
        assert_eq!(req.namespace, "channel-inbox");
        assert_eq!(
            req.visibility,
            agena_plugin_sdk::host_api::HostStorageVisibility::Private
        );
        Ok(HostStorageGetResponse {
            value: self.records.lock().unwrap().get(&req.key).cloned(),
        })
    }

    async fn storage_set(&self, req: HostStorageSetRequest) -> Result<()> {
        if let Some(pause) = &self.pause_first_write
            && !pause.started.swap(true, Ordering::Relaxed)
        {
            pause.entered.notify_one();
            pause.resume.notified().await;
        }
        self.records.lock().unwrap().insert(req.key, req.value);
        Ok(())
    }
}

fn slack() -> Connection {
    Connection::Slack(config::Slack {
        name: "main".into(),
        team_id: "T123".into(),
        app_id: "A123".into(),
        bot_user_id: "U123".into(),
        token_env: FIXTURE_ENV.into(),
        signing_secret_env: FIXTURE_ENV.into(),
    })
}

fn signed_mac(parts: &[&[u8]]) -> String {
    let secret = config::secret(FIXTURE_ENV).unwrap();
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(secret.as_bytes()).unwrap();
    for part in parts {
        mac.update(part);
    }
    hex::encode(mac.finalize().into_bytes())
}

fn slack_webhook(value: Value, now: i64) -> ChannelWebhookEnvelope {
    let body = value.to_string();
    let timestamp = (now / 1000).to_string();
    let signature = signed_mac(&[b"v0:", timestamp.as_bytes(), b":", body.as_bytes()]);
    ChannelWebhookEnvelope::Event {
        body,
        timestamp: Some(timestamp),
        nonce: None,
        signature: Some(format!("v0={signature}")),
    }
}

fn new_poll(identity: ChannelIdentity, now_ms: i64) -> AdapterPollRequest {
    AdapterPollRequest {
        binding_id: "binding".into(),
        config: json!({}),
        cursor: Value::Null,
        limit: 1,
        now_ms,
        not_before_ms: now_ms - 60_000,
        channel_identity: Some(identity),
        channel_kinds: Vec::new(),
    }
}

fn invoke(method: &str, input: Value) -> PluginServiceInvokeInput {
    PluginServiceInvokeInput {
        service: INTEGRATION_SERVICE.into(),
        api_version: INTEGRATION_API_VERSION,
        method: method.into(),
        input,
    }
}

#[test]
fn channel_manifests_publish_the_host_contract() {
    for platform in CHAT_PLATFORM_PLUGINS {
        let manifest = ChatChannel::new(platform).manifest();
        assert_eq!(manifest.name, platform.name());
        assert_eq!(manifest.services.exports.len(), 1);
        let export = &manifest.services.exports[0];
        manifest.services.validate().unwrap();
        assert_eq!(export.id, INTEGRATION_SERVICE);
        for method in ["describe", "poll", "send"] {
            assert!(export.methods.iter().any(|item| item.id == method));
        }
        assert_eq!(
            export.methods.iter().any(|item| item.id == "webhook"),
            platform != ChatPlatform::Discord
        );
        manifest
            .settings
            .unwrap()
            .validate_value(&json!({"accounts": []}))
            .unwrap();
    }
}

#[tokio::test]
async fn signed_slack_messages_survive_replay_and_poll_through_the_service() {
    let api = fixture(&[
        (
            "POST /api/auth.test ",
            200,
            json!({"ok":true,"team_id":"T123","user_id":"U123","bot_id":"B123"}),
        ),
        (
            "POST /api/chat.postMessage ",
            200,
            json!({"ok":true,"ts":"1800000000.123457"}),
        ),
    ])
    .await;
    let identity = slack().identity();
    let plugin = ChatChannel::new(ChatPlatform::Slack);
    plugin
        .state
        .set(ChannelState {
            host: Arc::new(Storage::default()),
            connections: vec![slack()],
            api: PlatformApi::new().unwrap().with_endpoint(api),
        })
        .ok()
        .unwrap();

    let now = chrono::Utc::now().timestamp_millis();
    let body = json!({
        "team_id": "T123", "api_app_id": "A123", "type": "event_callback",
        "event": {"type":"message","channel_type":"channel","channel":"C123","user":"U456",
            "ts": format!("{}.123456", now / 1000 - 1), "thread_ts":"1800000000.000001","text":"<@U123> hello"}
    });
    let mut forged = slack_webhook(body.clone(), now);
    if let ChannelWebhookEnvelope::Event { body, .. } = &mut forged {
        body.push(' ');
    }
    let result = plugin
        .service_invoke(invoke(
            "webhook",
            json!({"connection": "main", "envelope": forged}),
        ))
        .await
        .unwrap();
    assert_eq!(result["status"], "unauthenticated");
    for _ in 0..2 {
        let envelope = slack_webhook(body.clone(), now);
        let result = plugin
            .service_invoke(invoke(
                "webhook",
                json!({"connection": "main", "envelope": envelope}),
            ))
            .await
            .unwrap();
        assert_eq!(result["status"], "accepted");
    }
    let mut poll = new_poll(identity.clone(), now);
    let batch: AdapterBatch = decode(
        plugin
            .service_invoke(invoke("poll", json!(poll)))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(batch.events.len(), 1);
    let event = &batch.events[0];
    assert_eq!(event.actor, "U456");
    assert_eq!(event.resource, "C123");
    assert!(event.channel.as_ref().unwrap().addressed_to_bot);
    poll.cursor = batch.cursor;
    let next: AdapterBatch = decode(
        plugin
            .service_invoke(invoke("poll", json!(poll)))
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(next.events.is_empty());

    let delivery = ChannelDelivery {
        binding_id: "binding".into(),
        config: json!({}),
        delivery_id: "reply".into(),
        resource: event.resource.clone(),
        thread: event.thread.clone(),
        text: "fixture reply".into(),
        channel: Some(identity),
        kind: Some(ConversationKind::Group),
    };
    let reply: ChannelReceipt = decode(
        plugin
            .service_invoke(invoke("send", json!(delivery)))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(reply.disposition, SendDisposition::Sent);
}

#[test]
fn feishu_requires_a_signature_and_decrypts_platform_events() {
    let account = Connection::Feishu(config::Feishu {
        name: "main".into(),
        tenant_key: "t_fixture".into(),
        app_id: "cli_fixture".into(),
        bot_open_id: "ou_bot".into(),
        lark: false,
        app_secret_env: FIXTURE_ENV.into(),
        encrypt_key_env: FIXTURE_ENV.into(),
        verification_token_env: FIXTURE_ENV.into(),
    });
    let token = config::secret(FIXTURE_ENV).unwrap();
    let payload = json!({
        "schema": "2.0",
        "header": {"token": token, "app_id":"cli_fixture","tenant_key":"t_fixture","event_type":"im.message.receive_v1"},
        "event": {
            "sender":{"sender_type":"user","sender_id":{"open_id":"ou_human"}},
            "message":{"message_type":"text","message_id":"om_in","chat_id":"oc_room","chat_type":"group",
                "content":"{\"text\":\"hello\"}","root_id":"om_root","mentions":[{"id":{"open_id":"ou_bot"}}],
                "create_time":(NOW-1200).to_string()}
        }
    }).to_string();
    let key = Sha256::digest(token.as_bytes());
    let iv = [1u8; 16];
    let mut encrypted = vec![0; payload.len() + 16];
    encrypted[..payload.len()].copy_from_slice(payload.as_bytes());
    let cipher = cbc::Encryptor::<Aes256>::new_from_slices(&key, &iv)
        .unwrap()
        .encrypt_padded_mut::<Pkcs7>(&mut encrypted, payload.len())
        .unwrap();
    let data = json!({"encrypt": STANDARD.encode([&iv[..], cipher].concat())}).to_string();
    let timestamp = (NOW / 1000).to_string();
    let nonce = "fixture-nonce";
    let signature = hex::encode(Sha256::digest(
        format!("{timestamp}{nonce}{token}{data}").as_bytes(),
    ));
    let request = |signature| ChannelWebhookEnvelope::Event {
        body: data.clone(),
        timestamp: Some(timestamp.clone()),
        nonce: Some(nonce.into()),
        signature,
    };
    assert!(matches!(
        webhooks::decode(&account, &[], request(None), NOW).unwrap(),
        webhooks::Decoded::Reply(ChannelWebhookResponse::Unauthenticated)
    ));
    let webhooks::Decoded::Messages(found) =
        webhooks::decode(&account, &[], request(Some(signature)), NOW).unwrap()
    else {
        panic!("expected a Feishu message");
    };
    assert_eq!(found[0].thread.as_deref(), Some("om_root"));
    assert_eq!(found[0].actor, "ou_human");
    assert!(found[0].channel.as_ref().unwrap().addressed_to_bot);
}

#[test]
fn whatsapp_demultiplexes_a_signed_app_batch_only_to_its_own_phones() {
    let whatsapp = |name: &str, business: &str, phone: &str, app_secret_env: &str| {
        Connection::WhatsApp(config::WhatsApp {
            name: name.into(),
            business_account_id: business.into(),
            phone_number_id: phone.into(),
            token_env: FIXTURE_ENV.into(),
            app_secret_env: app_secret_env.into(),
            verification_token_env: FIXTURE_ENV.into(),
        })
    };
    let account = whatsapp("main", "110", "220", FIXTURE_ENV);
    let peers = [
        whatsapp("same-app", "110", "221", FIXTURE_ENV),
        whatsapp("other-app", "111", "220", "AGENA_FIXTURE_OTHER_WABA_SECRET"),
    ];
    let challenge = webhooks::decode(
        &account,
        &peers,
        ChannelWebhookEnvelope::Verification {
            mode: "subscribe".into(),
            token: config::secret(FIXTURE_ENV).unwrap(),
            challenge: "123456".into(),
        },
        NOW,
    )
    .unwrap();
    assert!(matches!(
        challenge,
        webhooks::Decoded::Reply(ChannelWebhookResponse::TextChallenge { .. })
    ));
    let change = |phone: &str| {
        json!({"field": "messages", "value": {
            "metadata": {"phone_number_id":phone},
            "messages":[{"type":"text","id":"wamid.fixture","from":"1234567890","text":{"body":"hi"},"timestamp":(NOW / 1000).to_string()}],
        }})
    };
    let body = json!({
        "object":"whatsapp_business_account",
        "entry":[{"id":"111","changes":[change("220")]},{"id":"110","changes":[change("221"),change("220")]}],
    }).to_string();
    let signature = signed_mac(&[body.as_bytes()]);
    let decoded = webhooks::decode(
        &account,
        &peers,
        ChannelWebhookEnvelope::Event {
            body,
            timestamp: None,
            nonce: None,
            signature: Some(format!("sha256={signature}")),
        },
        NOW,
    )
    .unwrap();
    let webhooks::Decoded::Messages(messages) = decoded else {
        panic!("expected a WhatsApp message")
    };
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].resource, "1234567890");
    assert_eq!(
        messages[0].channel.as_ref().unwrap().identity,
        account.identity()
    );
    assert_eq!(
        messages[1].channel.as_ref().unwrap().identity,
        peers[0].identity()
    );
}

#[tokio::test]
async fn dropped_acknowledgement_keeps_the_writer_serialized() {
    let store = Arc::new(Storage {
        pause_first_write: Some(PausedWrite::default()),
        ..Default::default()
    });
    let pause = store.pause_first_write.as_ref().unwrap();
    let connection = slack();
    let identity = connection.identity();
    let now = chrono::Utc::now().timestamp_millis();
    let message = |id: &str| {
        vec![
            webhooks::to_event(
                &identity,
                webhooks::Message {
                    id: id.into(),
                    resource: "C123",
                    actor: "U456",
                    kind: ConversationKind::Group,
                    thread: None,
                    text: "fixture message",
                    addressed: true,
                    occurred_at_ms: now,
                },
                now,
            )
            .unwrap(),
        ]
    };
    let first = tokio::spawn(inbox::append(store.clone(), message("first")));
    pause.entered.notified().await;
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    let second = tokio::spawn(inbox::append(store.clone(), message("second")));
    tokio::task::yield_now().await;
    assert!(!second.is_finished());
    pause.resume.notify_one();
    second.await.unwrap().unwrap();
    let mut request = new_poll(identity, now);
    request.limit = 2;
    let batch = inbox::poll(&*store, &connection, &request).await.unwrap();
    assert_eq!(
        batch
            .events
            .iter()
            .map(|event| event.id.as_str())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
}

#[tokio::test]
async fn discord_verifies_identity_and_preserves_cursor_on_rate_limit() {
    let config = config::Discord {
        name: "main".into(),
        bot_user_id: "123".into(),
        token_env: FIXTURE_ENV.into(),
        rooms: vec![config::DiscordRoom {
            id: "456".into(),
            kind: ConversationKind::Group,
        }],
    };
    let new_message_id = (((NOW - 1_420_070_400_000 - 1000) as u64) << 22) + 123;
    let api = fixture(&[
        ("GET /users/@me ", 200, json!({"id":"123","bot":true})),
        ("GET /channels/456 ", 200, json!({"id":"456","type":0})),
        (
            "GET /channels/456/messages?",
            200,
            json!([{
            "id":new_message_id.to_string(),"type":0,"channel_id":"456","author":{"id":"300"},
            "mentions":[{"id":"123"}], "content":"hello"}]),
        ),
        ("GET /channels/456 ", 200, json!({"id":"456","type":0})),
        ("GET /channels/456/messages?", 429, json!({"retry_after":2})),
        ("POST /channels/456/messages ", 200, json!({"id":"111"})),
    ])
    .await;
    let account = Connection::Discord(config);
    let Connection::Discord(config) = &account else {
        unreachable!()
    };
    let api = PlatformApi::new().unwrap().with_endpoint(api);
    let mut poll = new_poll(account.identity(), NOW);
    let batch = api.poll_discord(&account, config, &poll).await.unwrap();
    assert_eq!(batch.events.len(), 1);
    assert!(batch.events[0].channel.as_ref().unwrap().addressed_to_bot);
    poll.cursor = batch.cursor;
    let limited = api.poll_discord(&account, config, &poll).await.unwrap();
    assert_eq!(limited.cursor, poll.cursor);
    assert_eq!(limited.retry_after_seconds, Some(2));
    let reply = api
        .deliver(
            &account,
            &ChannelDelivery {
                binding_id: "binding".into(),
                config: json!({}),
                delivery_id: "reply".into(),
                resource: "456".into(),
                thread: None,
                text: "fixture reply".into(),
                channel: Some(account.identity()),
                kind: Some(ConversationKind::Group),
            },
        )
        .await
        .unwrap();
    assert_eq!(reply.disposition, SendDisposition::Sent);
}

async fn fixture(routes: &[(&'static str, u16, Value)]) -> String {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let routes = routes.to_vec();
    tokio::spawn(async move {
        for (prefix, code, value) in routes {
            let (mut stream, _) = tokio::time::timeout(Duration::from_secs(8), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0; 2048];
                let read = stream.read(&mut chunk).await.unwrap();
                assert_ne!(read, 0);
                request.extend_from_slice(&chunk[..read]);
                let Some(header) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..header]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if request.len() >= header + 4 + length {
                    break;
                }
            }
            assert!(
                request.starts_with(prefix.as_bytes()),
                "unexpected fixture request route"
            );
            let body = value.to_string();
            let response = format!(
                "HTTP/1.1 {code} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nRetry-After: 2\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        }
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn feishu_and_whatsapp_replies_use_the_verified_scope() {
    let feishu = Connection::Feishu(config::Feishu {
        name: "fixture".into(),
        tenant_key: "t_fixture".into(),
        app_id: "cli_fixture".into(),
        bot_open_id: "ou_bot".into(),
        lark: false,
        app_secret_env: FIXTURE_ENV.into(),
        encrypt_key_env: FIXTURE_ENV.into(),
        verification_token_env: FIXTURE_ENV.into(),
    });
    let endpoint = fixture(&[
        (
            "POST /open-apis/auth/v3/tenant_access_token/internal ",
            200,
            json!({"code":0,"tenant_access_token":"fixture-token","expire":7200}),
        ),
        (
            "GET /open-apis/bot/v3/info ",
            200,
            json!({"code":0,"bot":{"open_id":"ou_bot"}}),
        ),
        (
            "GET /open-apis/im/v1/messages/om_wrong ",
            200,
            json!({"code":0,"data":{"items":[{"chat_id":"oc_other"}]}}),
        ),
        (
            "GET /open-apis/im/v1/messages/om_room ",
            200,
            json!({"code":0,"data":{"items":[{"chat_id":"oc_room"}]}}),
        ),
        (
            "POST /open-apis/im/v1/messages/om_room/reply ",
            200,
            json!({"code":0,"data":{"message_id":"om_reply"}}),
        ),
    ])
    .await;
    let api = PlatformApi::new().unwrap().with_endpoint(endpoint);
    let mut delivery = ChannelDelivery {
        binding_id: "binding".into(),
        config: json!({}),
        delivery_id: "reply".into(),
        resource: "oc_room".into(),
        thread: Some("om_wrong".into()),
        text: "fixture reply".into(),
        channel: Some(feishu.identity()),
        kind: Some(ConversationKind::Group),
    };
    assert_eq!(
        api.deliver(&feishu, &delivery).await.unwrap().disposition,
        SendDisposition::Rejected
    );
    delivery.thread = Some("om_room".into());
    assert_eq!(
        api.deliver(&feishu, &delivery)
            .await
            .unwrap()
            .remote_id
            .as_deref(),
        Some("om_reply")
    );

    let whatsapp = Connection::WhatsApp(config::WhatsApp {
        name: "fixture".into(),
        business_account_id: "110".into(),
        phone_number_id: "220".into(),
        token_env: FIXTURE_ENV.into(),
        app_secret_env: FIXTURE_ENV.into(),
        verification_token_env: FIXTURE_ENV.into(),
    });
    let endpoint = fixture(&[
        ("GET /220?fields=id ", 200, json!({"id":"220"})),
        (
            "POST /220/messages ",
            200,
            json!({"messages":[{"id":"wamid.fixture"}]}),
        ),
    ])
    .await;
    let api = PlatformApi::new().unwrap().with_endpoint(endpoint);
    delivery.resource = "1234567890".into();
    delivery.kind = Some(ConversationKind::Direct);
    delivery.channel = Some(whatsapp.identity());
    delivery.thread = None;
    assert_eq!(
        api.deliver(&whatsapp, &delivery)
            .await
            .unwrap()
            .remote_id
            .as_deref(),
        Some("wamid.fixture")
    );
}
