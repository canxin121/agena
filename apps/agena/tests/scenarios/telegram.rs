//! The bundled `agena.telegram` adapter against a local Bot API double.
//!
//! Telegram itself is never contacted: `api_base` points at a loopback Axum
//! fixture that answers `getMe`, `getUpdates` and `sendMessage`, so the whole
//! path — binding grant, long-poll checkpoint, host authorization, model turn,
//! outbound delivery — runs against the real product process.
use super::fixture;
use agena_api::integrations::*;
use agena_testkit::{Reply, RequestMatch, SettingsLayer, TestResult};
use anyhow::{Context, Result, ensure};
use axum::{Json, Router, extract::Path as RoutePath, extract::State, routing::post};
use chrono::Utc;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

const BOT_ID: i64 = 4242;
const BOT_USERNAME: &str = "agena_agent_bot";
const CHAT_ID: i64 = -1_001_234_567_890;
const MEMBER_ID: i64 = 77;
/// The instance operator's own Telegram id, kept so a departure never leaves
/// the policy without an owner.
const OWNER_ID: i64 = 7;
const TOKEN: &str = "4242:local-fixture-secret";

/// What the fake Bot API has been asked to do.
#[derive(Default)]
struct BotApi {
    /// Queued updates, drained by the next `getUpdates`.
    updates: Mutex<Vec<Value>>,
    /// Every `sendMessage` body, in order.
    sent: Mutex<Vec<Value>>,
}

async fn bot_api(
    State(api): State<Arc<BotApi>>,
    RoutePath(rest): RoutePath<String>,
    Json(body): Json<Value>,
) -> Json<Value> {
    match rest.rsplit('/').next().unwrap_or_default() {
        "getMe" => Json(json!({
            "ok": true,
            "result": {"id": BOT_ID, "is_bot": true, "username": BOT_USERNAME, "first_name": "Agena"}
        })),
        // The adapter sends `offset`; a real server would drop everything
        // before it. The fixture only ever holds one batch, so draining is the
        // same thing and keeps the acknowledgement honest.
        "getUpdates" => {
            let drained = std::mem::take(&mut *api.updates.lock().unwrap());
            Json(json!({"ok": true, "result": drained}))
        }
        "sendMessage" => {
            let mut sent = api.sent.lock().unwrap();
            sent.push(body);
            Json(json!({"ok": true, "result": {"message_id": 900 + sent.len() as i64}}))
        }
        other => Json(json!({
            "ok": false,
            "error_code": 404,
            "description": format!("Not Found: {other}")
        })),
    }
}

fn group_message(message_id: i64, text: &str, entities: Value) -> Value {
    json!({
        "message_id": message_id,
        // A second ahead of activation, so the adapter cannot mistake this
        // for pre-grant history it is supposed to checkpoint instead.
        "date": Utc::now().timestamp() + 1,
        "chat": {"id": CHAT_ID, "type": "supergroup", "title": "Team"},
        "from": {"id": MEMBER_ID, "is_bot": false, "first_name": "Ann"},
        "text": text,
        "entities": entities,
    })
}

fn update(update_id: i64, message: Value) -> Value {
    json!({"update_id": update_id, "message": message})
}

fn channel_policy() -> ChannelAccessPolicy {
    ChannelAccessPolicy {
        identity: ChannelIdentity {
            tenant: "telegram".into(),
            // The adapter lowercases the verified bot username.
            account: BOT_USERNAME.into(),
        },
        kinds: [ConversationKind::Group].into_iter().collect(),
        session_mode: ChannelSessionMode::PerConversation,
        thread_mode: ChannelThreadMode::PerThread,
        trigger: ChannelTrigger::DirectOrMention,
        members: vec![
            // The instance operator keeps ownership, so removing an ordinary
            // member is a revocation rather than a policy with no owner left.
            ChannelMember {
                actor: OWNER_ID.to_string(),
                role: ChannelRole::Owner,
                expires_at_ms: None,
            },
            ChannelMember {
                actor: MEMBER_ID.to_string(),
                role: ChannelRole::Member,
                expires_at_ms: None,
            },
        ],
        allow_unlisted_chat: false,
        grant: Default::default(),
        per_user_daily_limit: 10,
        allow_automated_reports: false,
        service_grant: Default::default(),
    }
}

fn spec() -> IntegrationSpec {
    IntegrationSpec {
        title: "Telegram group".into(),
        provider: "agena.telegram".into(),
        mode: IntegrationMode::Channel,
        // The endpoint, token variable and bot identity live in plugin
        // settings; a grant only selects rooms, so it cannot point the bot
        // somewhere else.
        config: json!({}),
        resource: CHAT_ID.to_string().into(),
        thread: None,
        event_types: vec!["message".into()],
        actors: vec![],
        instruction: String::new(),
        poll_seconds: 1,
        expires_at_ms: Utc::now().timestamp_millis() + 3_600_000,
        daily_limit: 20,
        total_limit: 100,
        max_execution_seconds: 120,
        use_source_permissions: true,
        channel_policy: Some(channel_policy()),
    }
}

async fn wait_for<F, Fut, T>(mut check: F, description: &str) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Option<T>>>,
{
    tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            if let Some(value) = check().await? {
                return Ok(value);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .with_context(|| format!("timed out: {description}"))?
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bundled_adapter_polls_a_group_answers_it_and_reports_a_platform_departure() -> TestResult {
    let api = Arc::new(BotApi::default());
    let requests = api.clone();
    fixture()
        .case_timeout(Duration::from_secs(180))
        .before_start(move |case| async move {
            let server = case
                .serve_http(Router::new().route("/{*rest}", post(bot_api)).with_state(requests))
                .await?;
            let mut config: Value =
                serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
            // A self-hosted or fixture endpoint is plain HTTP on loopback,
            // which the adapter refuses unless this flag is explicitly set.
            config["plugins"]["list"]["agena.telegram"] = json!({
                "enabled": true,
                "package": {"kind": "static"},
                "settings": {
                    "bot_token_env": "AGENA_TEST_TELEGRAM_TOKEN",
                    "account": BOT_USERNAME,
                    "api_base": server.base_url(),
                    "long_poll_seconds": 0,
                    "allow_loopback_http": true
                }
            });
            case.write_config(SettingsLayer::Global, config).await
        })
        .env("AGENA_TEST_TELEGRAM_TOKEN", TOKEN)
        .run(move |case| async move {
            let operator = case.user().new_session().await?;
            let client = operator.client();

            let catalog = client.integrations(operator.id()).await?;
            let provider = catalog
                .providers
                .iter()
                .find(|provider| provider.plugin_id == "agena.telegram")
                .context("the bundled Telegram adapter must advertise agena.integration v1")?;
            ensure!(provider.adapter.kind == AdapterKind::Channel);
            ensure!(provider.adapter.channel_context_version == Some(1));

            let binding = client
                .create_integration(operator.id(), &CreateIntegration {
                    request_key: "telegram-group".into(),
                    spec: spec(),
                })
                .await?;
            ensure!(binding.state == BindingState::Disabled);
            let binding = client
                .control_integration(operator.id(), binding.id, &IntegrationControl {
                    expected_revision: binding.revision,
                    action: IntegrationControlAction::Enable,
                })
                .await?;

            // One message merely talks in the room; only the second one names
            // the bot. The host must see that difference as adapter evidence,
            // not as text the model was asked to interpret.
            let mention = format!("@{BOT_USERNAME}");
            api.updates.lock().unwrap().extend([
                update(1, group_message(11, "unaddressed-chatter-proof", json!([]))),
                update(2, group_message(
                    12,
                    &format!("{mention} telegram-channel-proof"),
                    json!([{"type": "mention", "offset": 0, "length": mention.len()}]),
                )),
            ]);
            let answer = case
                .model()
                .expect(RequestMatch::user_text("telegram-channel-proof"))
                .reply(Reply::text("telegram-reply-proof: ship it"));
            client
                .control_integration(operator.id(), binding.id, &IntegrationControl {
                    expected_revision: binding.revision,
                    action: IntegrationControlAction::PollNow,
                })
                .await?;
            answer.finished().await?;

            let sent = wait_for(
                || async {
                    Ok(api
                        .sent
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|body| {
                            body["text"]
                                .as_str()
                                .is_some_and(|text| text.contains("telegram-reply-proof"))
                        })
                        .cloned())
                },
                "the agent's answer posted through the Bot API",
            )
            .await?;
            ensure!(
                sent["chat_id"] == json!(CHAT_ID.to_string()),
                "a reply must return to the conversation it came from: {sent}"
            );
            ensure!(sent["parse_mode"] == json!("HTML"));
            ensure!(sent.get("message_thread_id").is_none(), "a threadless group reply: {sent}");

            let detail = client.integration_detail(operator.id(), binding.id).await?;
            ensure!(detail.binding.admitted == 1, "one addressed message was admitted: {detail:?}");
            let admitted = detail
                .events
                .iter()
                .find(|event| event.state != "ignored")
                .context("the addressed message must be recorded")?;
            ensure!(admitted.event.resource == CHAT_ID.to_string());
            ensure!(admitted.event.actor == MEMBER_ID.to_string());
            ensure!(admitted.event.event_type == "message");
            let chatter = detail
                .events
                .iter()
                .find(|event| event.state == "ignored")
                .context("the unaddressed message must be recorded as ignored")?;
            ensure!(
                chatter.note.as_deref() == Some("message_not_addressed_to_assistant"),
                "the adapter reported evidence, and the host applied its own trigger: {chatter:?}"
            );
            wait_for(
                || async {
                    let detail = client.integration_detail(operator.id(), binding.id).await?;
                    Ok(detail
                        .deliveries
                        .iter()
                        .any(|delivery| delivery.state == "sent")
                        .then_some(()))
                },
                "durable sent receipt",
            )
            .await?;

            // Leaving the group is a platform fact. It may only remove
            // authority, and it must be applied without any chat budget.
            api.updates.lock().unwrap().push(json!({
                "update_id": 3,
                "chat_member": {
                    "chat": {"id": CHAT_ID, "type": "supergroup", "title": "Team"},
                    "date": Utc::now().timestamp() + 2,
                    "old_chat_member": {"status": "member", "user": {"id": MEMBER_ID, "is_bot": false, "first_name": "Ann"}},
                    "new_chat_member": {"status": "left", "user": {"id": MEMBER_ID, "is_bot": false, "first_name": "Ann"}}
                }
            }));
            let current = client.integration_detail(operator.id(), binding.id).await?;
            client
                .control_integration(operator.id(), binding.id, &IntegrationControl {
                    expected_revision: current.binding.revision,
                    action: IntegrationControlAction::PollNow,
                })
                .await?;
            let revoked = wait_for(
                || async {
                    let detail = client.integration_detail(operator.id(), binding.id).await?;
                    Ok(detail
                        .events
                        .iter()
                        .find(|event| event.event.event_type == "membership.revoked")
                        .cloned())
                },
                "the verified departure",
            )
            .await?;
            ensure!(revoked.state == "handled");
            ensure!(revoked.note.as_deref() == Some("verified_platform_revocation"));
            let after = client.integration_detail(operator.id(), binding.id).await?;
            let policy = after
                .binding
                .spec
                .channel_policy
                .as_ref()
                .context("the policy must survive the revocation")?;
            let member = policy
                .members
                .iter()
                .find(|member| member.actor == MEMBER_ID.to_string())
                .context("the departed member stays listed")?;
            ensure!(member.role == ChannelRole::Blocked, "departure blocks, never grants: {member:?}");
            ensure!(
                after
                    .access_audit
                    .iter()
                    .any(|entry| entry.actor == "platform:agena.telegram"),
                "the removal must be audited as a platform action, not an operator edit"
            );

            // A member who left cannot come back through message text.
            api.updates.lock().unwrap().push(update(4, group_message(
                13,
                &format!("{mention} back-again-proof"),
                json!([{"type": "mention", "offset": 0, "length": mention.len()}]),
            )));
            let before = after.binding.admitted;
            client
                .control_integration(operator.id(), binding.id, &IntegrationControl {
                    expected_revision: after.binding.revision,
                    action: IntegrationControlAction::PollNow,
                })
                .await?;
            let blocked = wait_for(
                || async {
                    let detail = client.integration_detail(operator.id(), binding.id).await?;
                    Ok(detail
                        .events
                        .iter()
                        .find(|event| event.event.id == format!("telegram:{CHAT_ID}:13"))
                        .cloned())
                },
                "the post-departure message",
            )
            .await?;
            ensure!(blocked.state == "ignored");
            ensure!(blocked.note.as_deref() == Some("member_blocked"));
            ensure!(
                client.integration_detail(operator.id(), binding.id).await?.binding.admitted == before,
                "a blocked sender must not consume the chat budget"
            );
            Ok(())
        })
        .await
}
