use agena_assistant::{
    CHANNEL_WEBHOOK_MAX_BYTES, ChannelWebhookEnvelope, ChannelWebhookRequest,
    ChannelWebhookResponse,
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use serde::Deserialize;

use crate::AppState;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/hooks/chat/{platform}/{connection}",
            post(event).get(verify),
        )
        .layer(DefaultBodyLimit::max(CHANNEL_WEBHOOK_MAX_BYTES))
}

async fn event(
    State(state): State<AppState>,
    Path((platform, connection)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let names = match platform.as_str() {
        "slack" => (Some("x-slack-request-timestamp"), None, "x-slack-signature"),
        "feishu" => (
            Some("x-lark-request-timestamp"),
            Some("x-lark-request-nonce"),
            "x-lark-signature",
        ),
        "whatsapp" => (None, None, "x-hub-signature-256"),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let Ok(body) = std::str::from_utf8(&body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let header = |name| {
        let value = headers.get(name)?;
        value
            .to_str()
            .ok()
            .filter(|value| value.len() <= 128)
            .map(str::to_owned)
    };
    let envelope = ChannelWebhookEnvelope::Event {
        body: body.into(),
        timestamp: names.0.and_then(header).filter(|value| value.len() <= 64),
        nonce: names.1.and_then(header),
        signature: header(names.2),
    };
    dispatch(&state, &platform, connection, envelope).await
}

#[derive(Deserialize)]
struct Verification {
    #[serde(rename = "hub.mode")]
    mode: String,
    #[serde(rename = "hub.verify_token")]
    token: String,
    #[serde(rename = "hub.challenge")]
    challenge: String,
}

async fn verify(
    State(state): State<AppState>,
    Path((platform, connection)): Path<(String, String)>,
    Query(query): Query<Verification>,
) -> Response {
    if platform != "whatsapp" {
        return StatusCode::NOT_FOUND.into_response();
    }
    if query.mode.len() > 32 || query.token.len() > 256 || query.challenge.len() > 256 {
        return StatusCode::BAD_REQUEST.into_response();
    }
    dispatch(
        &state,
        &platform,
        connection,
        ChannelWebhookEnvelope::Verification {
            mode: query.mode,
            token: query.token,
            challenge: query.challenge,
        },
    )
    .await
}

async fn dispatch(
    state: &AppState,
    platform: &str,
    connection: String,
    envelope: ChannelWebhookEnvelope,
) -> Response {
    if connection.is_empty()
        || connection.len() > 48
        || !connection
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Ok(service) = state.assistant_service() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match service
        .channel_webhook(
            platform,
            ChannelWebhookRequest {
                connection,
                envelope,
            },
        )
        .await
    {
        Ok(ChannelWebhookResponse::Accepted) => StatusCode::OK.into_response(),
        Ok(ChannelWebhookResponse::JsonChallenge { challenge }) => {
            Json(serde_json::json!({"challenge": challenge})).into_response()
        }
        Ok(ChannelWebhookResponse::TextChallenge { challenge }) => challenge.into_response(),
        Ok(ChannelWebhookResponse::Unauthenticated) => StatusCode::UNAUTHORIZED.into_response(),
        Ok(ChannelWebhookResponse::Invalid) => StatusCode::BAD_REQUEST.into_response(),
        Ok(ChannelWebhookResponse::NotFound) | Err(agena_assistant::Error::NotFound) => {
            StatusCode::NOT_FOUND.into_response()
        }
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}
