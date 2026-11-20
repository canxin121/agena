//! Signed HTTP channel fixture for authorization and participant scenarios.
//! Incoming signed messages still pass through the production integration
//! service, channel principal, model execution and outbound delivery paths.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use agena_plugin_sdk::integration::{
    AdapterBatch, AdapterDescription, AdapterKind, AdapterPollRequest, ChannelDelivery,
    ChannelReceipt, ExternalEvent, SendDisposition,
};
use agena_plugin_sdk::prelude::*;
use agena_testkit::{ModelOptions, SettingsLayer, TestAgenaBuilder};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use chrono::Utc;

use super::{Replies, sign};

struct ChannelState {
    events: Mutex<Vec<ExternalEvent>>,
    replies: Replies,
}

struct HttpChannel {
    state: Arc<ChannelState>,
}

#[agena_plugin(
    namespace = "example",
    name = "webhook_chat",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Signed HTTP channel fixture for native MCP authorization."
)]
impl HttpChannel {
    #[hook(
        init,
        summary = "Initialize the channel fixture",
        help = "Acknowledges a real SDK HTTP plugin instance."
    )]
    async fn initialize(
        &self,
        _ctx: InitContext,
        _host: Arc<dyn HostClient>,
    ) -> Result<InitOutcome> {
        Ok(InitOutcome::ack(self.manifest()))
    }

    #[service("agena.integration", version = 1, method = "describe")]
    async fn describe(&self) -> Result<AdapterDescription> {
        Ok(AdapterDescription {
            kind: AdapterKind::Channel,
            title: "Signed MCP test channel".into(),
            description: "Authenticated events and observable, idempotent replies.".into(),
            config_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["conversation"],
                "properties": {
                    "conversation": {"type": "string", "minLength": 1, "maxLength": 256}
                }
            }),
            minimum_poll_seconds: 1,
            idempotent_send: true,
            channel_context_version: Some(1),
        })
    }

    #[service("agena.integration", version = 1, method = "poll")]
    async fn poll(&self, request: &AdapterPollRequest) -> Result<AdapterBatch> {
        let identity = request.channel_identity.as_ref().ok_or_else(|| {
            PluginError::invalid_params("host must pin the channel tenant and account")
        })?;
        let conversation = request.config["conversation"]
            .as_str()
            .ok_or_else(|| PluginError::invalid_params("conversation is required"))?;
        let after = if request.cursor.is_null() {
            0
        } else {
            if request.cursor["identity"] != json!(identity)
                || request.cursor["conversation"] != conversation
            {
                return Err(PluginError::invalid_params(
                    "cursor belongs to another channel namespace",
                ));
            }
            request.cursor["sequence"]
                .as_u64()
                .ok_or_else(|| PluginError::invalid_params("invalid channel cursor"))?
        };
        let events = self.state.events.lock().unwrap();
        let limit = request.limit.clamp(1, 32) as usize;
        let batch = events
            .iter()
            .enumerate()
            .filter(|(index, event)| {
                (*index as u64 + 1) > after
                    && event.resource == conversation
                    && event.channel.as_ref().is_some_and(|channel| {
                        &channel.identity == identity
                            && (request.channel_kinds.is_empty()
                                || request.channel_kinds.contains(&channel.kind))
                    })
            })
            .take(limit)
            .map(|(index, event)| (index as u64 + 1, event.clone()))
            .collect::<Vec<_>>();
        let sequence = if batch.len() == limit {
            batch.last().unwrap().0
        } else {
            after.max(events.len() as u64)
        };
        Ok(AdapterBatch {
            events: batch.into_iter().map(|(_, event)| event).collect(),
            cursor: json!({
                "identity": identity,
                "conversation": conversation,
                "sequence": sequence
            }),
            retry_after_seconds: Some(1),
        })
    }

    #[service("agena.integration", version = 1, method = "send")]
    async fn send(&self, request: &ChannelDelivery) -> Result<ChannelReceipt> {
        if request.config["conversation"] != request.resource
            || request.channel.is_none()
            || request.kind.is_none()
        {
            return Err(PluginError::invalid_params(
                "reply requires the configured conversation and channel identity",
            ));
        }
        let payload = json!({
            "channel": request.channel,
            "kind": request.kind,
            "delivery_id": request.delivery_id,
            "conversation": request.resource,
            "thread": request.thread,
            "text": request.text
        });
        let mut replies = self.state.replies.lock().unwrap();
        if let Some(prior) = replies
            .iter()
            .find(|prior| prior["delivery_id"] == request.delivery_id)
        {
            if prior != &payload {
                return Err(PluginError::invalid_params(
                    "delivery identity reused with different content",
                ));
            }
        } else {
            replies.push(payload);
        }
        Ok(ChannelReceipt {
            disposition: SendDisposition::Sent,
            remote_id: Some(request.delivery_id.clone()),
            retry_after_seconds: None,
        })
    }
}

async fn signed_message(
    State(state): State<Arc<ChannelState>>,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, Json<Value>) {
    let Some(timestamp) = headers
        .get("X-Agena-Timestamp")
        .and_then(|value| value.to_str().ok())
    else {
        return rejected(StatusCode::UNAUTHORIZED);
    };
    let Some(supplied) = headers
        .get("X-Agena-Signature")
        .and_then(|value| value.to_str().ok())
    else {
        return rejected(StatusCode::UNAUTHORIZED);
    };
    let Ok(time) = timestamp.parse::<i64>() else {
        return rejected(StatusCode::UNAUTHORIZED);
    };
    let expected = sign(timestamp, &body);
    if Utc::now().timestamp_millis().abs_diff(time) > 300_000
        || expected.len() != supplied.len()
        || expected
            .bytes()
            .zip(supplied.bytes())
            .fold(0u8, |different, (a, b)| different | (a ^ b))
            != 0
    {
        return rejected(StatusCode::UNAUTHORIZED);
    }
    let Ok(event) = serde_json::from_slice::<ExternalEvent>(&body) else {
        return rejected(StatusCode::BAD_REQUEST);
    };
    let Some(channel) = event.channel.as_ref() else {
        return rejected(StatusCode::BAD_REQUEST);
    };
    if event.validate().is_err()
        || !matches!(event.event_type.as_str(), "message" | "membership.revoked")
        || matches!(
            event.actor.as_str(),
            "@service" | "@shared" | "@unattributed" | "*"
        )
        || event.actor.chars().any(char::is_whitespace)
        || [&channel.identity.tenant, &channel.identity.account]
            .iter()
            .any(|value| {
                value.is_empty()
                    || value.len() > 256
                    || value.chars().any(|c| c.is_control() || c.is_whitespace())
            })
        || (event.event_type == "membership.revoked"
            && (!channel.actor_verified || channel.is_member))
    {
        return rejected(StatusCode::BAD_REQUEST);
    }
    let mut events = state.events.lock().unwrap();
    if let Some(prior) = events.iter().find(|prior| {
        prior.id == event.id
            && prior.resource == event.resource
            && prior
                .channel
                .as_ref()
                .is_some_and(|old| old.identity == channel.identity && old.kind == channel.kind)
    }) {
        return if prior == &event {
            (
                StatusCode::OK,
                Json(json!({"accepted": true, "replayed": true})),
            )
        } else {
            rejected(StatusCode::CONFLICT)
        };
    }
    events.push(event);
    (StatusCode::ACCEPTED, Json(json!({"accepted": true})))
}

fn rejected(status: StatusCode) -> (StatusCode, Json<Value>) {
    (status, Json(json!({"accepted": false})))
}

pub(super) fn fixture(replies: Replies) -> TestAgenaBuilder {
    super::super::fixture()
        .case_timeout(Duration::from_secs(240))
        .model_options(ModelOptions {
            default_timeout: Duration::from_secs(45),
            ..Default::default()
        })
        .before_start(move |case| async move {
            let state = Arc::new(ChannelState {
                events: Mutex::new(Vec::new()),
                replies,
            });
            let plugin_state = state.clone();
            let rpc = agena_plugin_sdk::drivers::http::router(
                move || HttpChannel {
                    state: plugin_state.clone(),
                },
                Arc::new(NoopHostClient),
            );
            let ingress = Router::new()
                .route("/messages", post(signed_message))
                .layer(DefaultBodyLimit::max(64 * 1024))
                .with_state(state);
            let service = case.serve_http(rpc.merge(ingress)).await?;
            // Reuse the existing signed-channel sender without changing the
            // shared stdio fixture used by the other channel scenarios.
            tokio::fs::write(
                case.workspace().join("rbac-status.json"),
                serde_json::to_vec(&json!({"port": service.address().port()}))?,
            )
            .await?;
            let mut config: Value =
                serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
            config["plugins"]["list"]["example.webhook_chat"] = json!({
                "package": {"kind": "http", "url": service.url("/rpc")}
            });
            case.write_config(SettingsLayer::Global, config).await
        })
}
