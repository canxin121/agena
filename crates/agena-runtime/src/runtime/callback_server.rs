//! Runtime-owned loopback listener, available before any plugin initialization.
//! Callback bearers authenticate here independently of the application's UI
//! session authentication. The listener and its weak routes own no runtime.

use std::sync::Arc;

use agena_plugin_host::{PluginCallbackDispatcher, PluginCallbackRpcError};
use agena_plugin_sdk::rpc::{
    ErrorObject, JsonRpcVersion, Request, Response, ResponsePayload, codes,
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    routing::post,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct CallbackState {
    dispatcher: PluginCallbackDispatcher,
    stop: CancellationToken,
}

pub(super) struct PluginCallbackServer {
    pub(super) dispatcher: PluginCallbackDispatcher,
    pub(super) base_url: String,
    stop: CancellationToken,
}

impl PluginCallbackServer {
    pub(super) async fn start() -> Result<Arc<Self>, std::io::Error> {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let base_url = format!("http://{}", listener.local_addr()?);
        let dispatcher = PluginCallbackDispatcher::default();
        let stop = CancellationToken::new();
        let router = Router::new()
            .route("/plugin-rpc/{plugin_id}", post(callback))
            .with_state(CallbackState {
                dispatcher: dispatcher.clone(),
                stop: stop.clone(),
            });
        let shutdown = stop.clone();
        tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, router)
                .with_graceful_shutdown(shutdown.cancelled_owned())
                .await
            {
                tracing::error!(%error, "plugin callback listener stopped");
            }
        });
        Ok(Arc::new(Self {
            dispatcher,
            base_url,
            stop,
        }))
    }
}

impl Drop for PluginCallbackServer {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        return None;
    }
    let mut parts = value.split_whitespace();
    let scheme = parts.next()?;
    let token = parts.next()?;
    (scheme.eq_ignore_ascii_case("bearer") && parts.next().is_none()).then_some(token)
}

async fn callback(
    State(state): State<CallbackState>,
    Path(plugin): Path<String>,
    headers: HeaderMap,
    Json(request): Json<Request>,
) -> (StatusCode, Json<Response>) {
    let id = request.id.clone();
    let result = tokio::select! {
        biased;
        _ = state.stop.cancelled() => {
            return (StatusCode::SERVICE_UNAVAILABLE, Json(Response {
                jsonrpc: JsonRpcVersion,
                id,
                payload: ResponsePayload::Err { error: ErrorObject {
                    code: codes::PLUGIN_GENERIC,
                    message: "plugin callback listener is shutting down".into(), data: None,
                } },
            }));
        }
        result = state.dispatcher.dispatch(&plugin, bearer_token(&headers), request) => result,
    };
    match result {
        Ok(response) => (StatusCode::OK, Json(response)),
        Err(error) => {
            let (status, code) = match error {
                PluginCallbackRpcError::InvalidCallbackToken => {
                    (StatusCode::UNAUTHORIZED, codes::INVALID_REQUEST)
                }
                PluginCallbackRpcError::MissingCallbackContext => {
                    (StatusCode::BAD_REQUEST, codes::INVALID_PARAMS)
                }
            };
            (
                status,
                Json(Response {
                    jsonrpc: JsonRpcVersion,
                    id,
                    payload: ResponsePayload::Err {
                        error: ErrorObject {
                            code,
                            message: error.to_string(),
                            data: None,
                        },
                    },
                }),
            )
        }
    }
}
