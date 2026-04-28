//! HTTP driver. Plugin author serves an axum router on their own port; one
//! POST endpoint receives JSON-RPC envelopes and forwards them to dispatch.

use std::sync::Arc;

use axum::{Json, Router, extract::State, routing::post};

use crate::drivers::dispatch::PluginDispatcher;
use crate::host_api::HostClient;
use crate::plugin::Plugin;
use crate::rpc::{ErrorObject, JsonRpcVersion, Request, Response, ResponsePayload, codes};

pub fn router<P: Plugin>(plugin: P, host: Arc<dyn HostClient>) -> Router {
    let dispatcher = Arc::new(PluginDispatcher::new(plugin));
    let dispatcher_clone = Arc::clone(&dispatcher);
    tokio::spawn(async move {
        dispatcher_clone.set_host(host).await;
    });
    Router::new()
        .route("/rpc", post(handle_rpc::<P>))
        .with_state(dispatcher)
}

async fn handle_rpc<P: Plugin>(
    State(dispatcher): State<Arc<PluginDispatcher<P>>>,
    Json(req): Json<Request>,
) -> Json<Response> {
    let id = req.id.clone();
    let params = req.params.unwrap_or(serde_json::Value::Null);
    match dispatcher.dispatch(&req.method, params).await {
        Ok(v) => Json(Response {
            jsonrpc: JsonRpcVersion,
            id,
            payload: ResponsePayload::Ok { result: v },
        }),
        Err(e) => Json(Response {
            jsonrpc: JsonRpcVersion,
            id,
            payload: ResponsePayload::Err {
                error: ErrorObject {
                    code: codes::PLUGIN_GENERIC,
                    message: e.message.clone(),
                    data: serde_json::to_value(&e).ok(),
                },
            },
        }),
    }
}

#[macro_export]
macro_rules! export_http {
    ($plugin_expr:expr, $bind_addr:expr) => {
        fn main() -> std::io::Result<()> {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            rt.block_on(async move {
                let host: ::std::sync::Arc<dyn $crate::host_api::HostClient> =
                    ::std::sync::Arc::new($crate::host_api::NoopHostClient);
                let app = $crate::drivers::http::router($plugin_expr, host);
                let listener = tokio::net::TcpListener::bind($bind_addr).await?;
                axum::serve(listener, app).await
            })
        }
    };
}
