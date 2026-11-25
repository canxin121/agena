use aes::Aes256;
use base64::{Engine, engine::general_purpose::STANDARD};
use cbc::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use agena_assistant::{CHANNEL_WEBHOOK_MAX_BYTES, ChannelWebhookEnvelope, ChannelWebhookResponse};
use agena_plugin_sdk::{
    Result,
    integration::{ChannelEnvelope, ChannelIdentity, ConversationKind, ExternalEvent},
};

use super::config::{
    Connection, Feishu, Slack, WhatsApp, decimal_id, platform_id, secret, valid_slack_timestamp,
};

const SIGNATURE_TOLERANCE_SECONDS: u64 = 300;
const TEXT_LIMIT: usize = 8 * 1024;

pub(super) enum Decoded {
    Reply(ChannelWebhookResponse),
    Messages(Vec<ExternalEvent>),
}

impl From<ChannelWebhookResponse> for Decoded {
    fn from(reply: ChannelWebhookResponse) -> Self {
        Self::Reply(reply)
    }
}

pub(super) fn decode(
    connection: &Connection,
    connections: &[Connection],
    envelope: ChannelWebhookEnvelope,
    now_ms: i64,
) -> Result<Decoded> {
    use ChannelWebhookResponse::{Invalid, NotFound, Unauthenticated};

    let (body, timestamp, nonce, signature) = match envelope {
        ChannelWebhookEnvelope::Event {
            body,
            timestamp,
            nonce,
            signature,
        } => (body, timestamp, nonce, signature),
        ChannelWebhookEnvelope::Verification {
            mode,
            token,
            challenge,
        } => {
            let Connection::WhatsApp(config) = connection else {
                return Ok(NotFound.into());
            };
            let expected = secret(&config.verification_token_env)?;
            if mode == "subscribe"
                && constant_eq(expected.as_bytes(), token.as_bytes())
                && decimal_id(&challenge)
            {
                return Ok(ChannelWebhookResponse::TextChallenge { challenge }.into());
            }
            return Ok(Unauthenticated.into());
        }
    };
    if body.len() > CHANNEL_WEBHOOK_MAX_BYTES {
        return Ok(Invalid.into());
    }
    let fresh = timestamp.as_deref().filter(|value| {
        value
            .parse::<i64>()
            .is_ok_and(|timestamp| timestamp.abs_diff(now_ms / 1000) <= SIGNATURE_TOLERANCE_SECONDS)
    });
    let supplied = signature.as_deref().unwrap_or_default();
    let (value, signed) = match connection {
        Connection::Slack(config) => {
            let Some(timestamp) = fresh else {
                return Ok(Unauthenticated.into());
            };
            let secret = secret(&config.signing_secret_env)?;
            let valid = hmac_valid(
                &secret,
                &[b"v0:", timestamp.as_bytes(), b":", body.as_bytes()],
                supplied,
                "v0=",
            );
            if !valid {
                return Ok(Unauthenticated.into());
            }
            (serde_json::from_str(&body).ok(), true)
        }
        Connection::Feishu(config) => {
            let key = secret(&config.encrypt_key_env)?;
            let signed = fresh
                .zip(nonce.as_deref())
                .is_some_and(|(timestamp, nonce)| {
                    feishu_signature_valid(&key, timestamp, nonce, &body, supplied)
                });
            let raw: Option<Value> = serde_json::from_str(&body).ok();
            let value = raw.and_then(|value| match value.get("encrypt").and_then(Value::as_str) {
                Some(encoded) => decrypt_feishu(&key, encoded),
                None => Some(value),
            });
            (value, signed)
        }
        Connection::WhatsApp(config) => {
            let app_secret = secret(&config.app_secret_env)?;
            if !hmac_valid(&app_secret, &[body.as_bytes()], supplied, "sha256=") {
                return Ok(Unauthenticated.into());
            }
            let Ok(value @ Value::Object(_)) = serde_json::from_str(&body) else {
                return Ok(Invalid.into());
            };
            // A Meta app has one callback URL and can carry messages for
            // several business accounts or phone numbers in the same batch.
            let same_app = connections.iter().filter_map(|target| {
                let Connection::WhatsApp(peer) = target else {
                    return None;
                };
                (peer.name != config.name
                    && secret(&peer.app_secret_env)
                        .is_ok_and(|key| constant_eq(key.as_bytes(), app_secret.as_bytes())))
                .then_some((target, peer))
            });
            let messages = std::iter::once((connection, config))
                .chain(same_app)
                .flat_map(|(target, config)| {
                    whatsapp_messages(config, &target.identity(), &value, now_ms)
                })
                .collect();
            return Ok(Decoded::Messages(messages));
        }
        Connection::Discord(_) => return Ok(NotFound.into()),
    };
    let Some(value) = value.filter(Value::is_object) else {
        return Ok(if signed { Invalid } else { Unauthenticated }.into());
    };

    if let Connection::Feishu(config) = connection {
        let expected = secret(&config.verification_token_env)?;
        let is_challenge = field(&value, "type") == "url_verification";
        let token = if is_challenge {
            field(&value, "token")
        } else {
            field(&value["header"], "token")
        };
        if !constant_eq(expected.as_bytes(), token.as_bytes()) || (!signed && !is_challenge) {
            return Ok(Unauthenticated.into());
        }
        if is_challenge {
            return Ok(challenge(&value));
        }
        let header = &value["header"];
        if field(header, "app_id") != config.app_id
            || field(header, "tenant_key") != config.tenant_key
        {
            return Ok(Unauthenticated.into());
        }
    }

    let identity = connection.identity();
    let messages = match connection {
        Connection::Slack(config) => {
            if field(&value, "type") == "url_verification" {
                return Ok(challenge(&value));
            }
            if field(&value, "team_id") != config.team_id
                || field(&value, "api_app_id") != config.app_id
            {
                return Ok(Unauthenticated.into());
            }
            slack_message(config, &identity, &value["event"], now_ms)
                .into_iter()
                .collect()
        }
        Connection::Feishu(config) => {
            if field(&value["header"], "event_type") != "im.message.receive_v1" {
                return Ok(Decoded::Messages(Vec::new()));
            }
            feishu_message(config, &identity, &value["event"], now_ms)
                .into_iter()
                .collect()
        }
        Connection::WhatsApp(_) | Connection::Discord(_) => return Ok(NotFound.into()),
    };
    Ok(Decoded::Messages(messages))
}

fn challenge(value: &Value) -> Decoded {
    let challenge = field(value, "challenge");
    if challenge.is_empty() || challenge.len() > 256 || challenge.chars().any(char::is_control) {
        return ChannelWebhookResponse::Invalid.into();
    }
    ChannelWebhookResponse::JsonChallenge {
        challenge: challenge.into(),
    }
    .into()
}

fn constant_eq(first: &[u8], second: &[u8]) -> bool {
    bool::from(first.ct_eq(second))
}

fn hmac_valid(secret: &str, fields: &[&[u8]], signature: &str, prefix: &str) -> bool {
    let Some(tag) = signature
        .strip_prefix(prefix)
        .and_then(|value| <[u8; 32]>::try_from(hex::decode(value).ok()?).ok())
    else {
        return false;
    };
    let Ok(mut mac) = <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    for field in fields {
        mac.update(field);
    }
    mac.verify_slice(&tag).is_ok()
}

fn feishu_signature_valid(
    key: &str,
    timestamp: &str,
    nonce: &str,
    body: &str,
    signature: &str,
) -> bool {
    let mut digest = Sha256::new();
    for field in [timestamp, nonce, key, body] {
        digest.update(field.as_bytes());
    }
    hex::decode(signature).is_ok_and(|tag| constant_eq(&digest.finalize(), &tag))
}

fn decrypt_feishu(key: &str, data: &str) -> Option<Value> {
    let mut cipher = STANDARD.decode(data).ok()?;
    if cipher.len() < 32 {
        return None;
    }
    let (iv, contents) = cipher.split_at_mut(16);
    let key = Sha256::digest(key.as_bytes());
    let plaintext = cbc::Decryptor::<Aes256>::new_from_slices(&key, iv)
        .ok()?
        .decrypt_padded_mut::<Pkcs7>(contents)
        .ok()?;
    serde_json::from_slice(plaintext).ok()
}

fn slack_message(
    config: &Slack,
    identity: &ChannelIdentity,
    event: &Value,
    now_ms: i64,
) -> Option<ExternalEvent> {
    if field(event, "type") != "message"
        || event.get("bot_id").is_some()
        || !matches!(
            field(event, "subtype"),
            "" | "file_share" | "thread_broadcast"
        )
    {
        return None;
    }
    let resource = field(event, "channel");
    let actor = field(event, "user");
    let ts = field(event, "ts");
    if !valid_slack_timestamp(ts) || actor == config.bot_user_id {
        return None;
    }
    let (seconds, micros) = ts.split_once('.')?;
    let occurred_at_ms =
        seconds.parse::<i64>().ok()?.checked_mul(1000)? + micros.parse::<i64>().ok()? / 1000;
    let kind = match field(event, "channel_type") {
        "im" => ConversationKind::Direct,
        "channel" | "group" | "mpim" => ConversationKind::Group,
        _ => return None,
    };
    let text = field(event, "text");
    let addressed = text.contains(&format!("<@{}>", config.bot_user_id))
        || text.contains(&format!("<@{}|", config.bot_user_id));
    let thread = event
        .get("thread_ts")
        .and_then(Value::as_str)
        .filter(|value| valid_slack_timestamp(value));
    to_event(
        identity,
        Message {
            id: format!("slack:{resource}:{ts}"),
            resource,
            actor,
            kind,
            thread,
            text,
            addressed,
            occurred_at_ms,
        },
        now_ms,
    )
}

fn feishu_message(
    config: &Feishu,
    identity: &ChannelIdentity,
    event: &Value,
    now_ms: i64,
) -> Option<ExternalEvent> {
    let sender = &event["sender"];
    let message = &event["message"];
    if field(sender, "sender_type") != "user" || field(message, "message_type") != "text" {
        return None;
    }
    let actor = field(&sender["sender_id"], "open_id");
    let id = field(message, "message_id");
    let resource = field(message, "chat_id");
    let kind = match field(message, "chat_type") {
        "p2p" => ConversationKind::Direct,
        "group" => ConversationKind::Group,
        _ => return None,
    };
    if actor == config.bot_open_id || !platform_id(id) {
        return None;
    }
    let contents: Value = serde_json::from_str(field(message, "content")).ok()?;
    let addressed = message["mentions"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|mention| field(&mention["id"], "open_id") == config.bot_open_id);
    to_event(
        identity,
        Message {
            id: format!("feishu:{id}"),
            resource,
            actor,
            kind,
            thread: message["root_id"].as_str().filter(|root| platform_id(root)),
            text: field(&contents, "text"),
            addressed,
            occurred_at_ms: field(message, "create_time").parse().ok()?,
        },
        now_ms,
    )
}

fn whatsapp_messages(
    config: &WhatsApp,
    identity: &ChannelIdentity,
    value: &Value,
    now_ms: i64,
) -> Vec<ExternalEvent> {
    if field(value, "object") != "whatsapp_business_account" {
        return Vec::new();
    }
    value["entry"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| field(entry, "id") == config.business_account_id)
        .flat_map(|entry| entry["changes"].as_array().into_iter().flatten())
        .filter(|change| {
            field(change, "field") == "messages"
                && field(&change["value"]["metadata"], "phone_number_id") == config.phone_number_id
        })
        .flat_map(|change| change["value"]["messages"].as_array().into_iter().flatten())
        .filter_map(|message| {
            let actor = field(message, "from");
            let id = field(message, "id");
            if !decimal_id(actor) || field(message, "type") != "text" || !id.starts_with("wamid.") {
                return None;
            }
            to_event(
                identity,
                Message {
                    id: format!("whatsapp:{id}"),
                    resource: actor,
                    actor,
                    kind: ConversationKind::Direct,
                    thread: None,
                    text: field(&message["text"], "body"),
                    addressed: true,
                    occurred_at_ms: field(message, "timestamp")
                        .parse::<i64>()
                        .ok()?
                        .checked_mul(1000)?,
                },
                now_ms,
            )
        })
        .collect()
}

pub(super) fn field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

pub(super) struct Message<'a> {
    pub id: String,
    pub resource: &'a str,
    pub actor: &'a str,
    pub kind: ConversationKind,
    pub thread: Option<&'a str>,
    pub text: &'a str,
    pub addressed: bool,
    pub occurred_at_ms: i64,
}

pub(super) fn to_event(
    identity: &ChannelIdentity,
    message: Message<'_>,
    now_ms: i64,
) -> Option<ExternalEvent> {
    if !platform_id(message.actor)
        || !platform_id(message.resource)
        || message.occurred_at_ms <= 0
        || message.occurred_at_ms > now_ms.saturating_add(60_000)
    {
        return None;
    }
    let mut text = message.text.trim();
    if text.len() > TEXT_LIMIT {
        text = &text[..text.floor_char_boundary(TEXT_LIMIT)];
    }
    let event = ExternalEvent {
        id: message.id,
        event_type: "message".into(),
        resource: message.resource.into(),
        actor: message.actor.into(),
        actor_display_name: None,
        thread: message.thread.map(str::to_owned),
        bot: false,
        occurred_at_ms: message.occurred_at_ms,
        text: text.into(),
        data: json!({}),
        channel: Some(ChannelEnvelope {
            identity: identity.clone(),
            kind: message.kind,
            actor_verified: true,
            is_member: true,
            addressed_to_bot: message.addressed,
            verified_at_ms: now_ms,
        }),
    };
    event.validate().ok()?;
    Some(event)
}
