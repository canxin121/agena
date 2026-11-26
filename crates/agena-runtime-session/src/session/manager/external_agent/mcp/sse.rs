use std::{collections::HashMap, future::ready};

use axum::{
    Json,
    extract::Query,
    response::{
        Sse,
        sse::{Event, KeepAlive},
    },
    routing::{get, post},
};
use futures_util::{StreamExt, stream};
use parking_lot::Mutex;
use rmcp::model::ClientJsonRpcMessage;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_util::sync::PollSender;
use uuid::Uuid;

use super::*;

const MAX_CONNECTIONS: usize = 16;
const MESSAGE_QUEUE: usize = 8;

type Sessions = Arc<Mutex<HashMap<Uuid, mpsc::Sender<ClientJsonRpcMessage>>>>;

#[derive(Clone)]
struct Server {
    backend: Backend,
    closing: CancellationToken,
    sessions: Sessions,
}

struct Connection {
    id: Uuid,
    sessions: Sessions,
    closing: CancellationToken,
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.sessions.lock().remove(&self.id);
        self.closing.cancel();
    }
}

pub(super) fn router(backend: Backend, closing: CancellationToken) -> Router {
    Router::new()
        .route("/sse", get(connect))
        .route("/messages", post(receive))
        .with_state(Server {
            backend,
            closing,
            sessions: Default::default(),
        })
}

async fn connect(State(server): State<Server>) -> Result<Response, StatusCode> {
    let id = Uuid::new_v4();
    let (incoming, mut input) = mpsc::channel(MESSAGE_QUEUE);
    let (output, outgoing) = mpsc::channel(MESSAGE_QUEUE);
    {
        let mut sessions = server.sessions.lock();
        if sessions.len() >= MAX_CONNECTIONS {
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
        sessions.insert(id, incoming);
    }
    let connection = Connection {
        id,
        sessions: server.sessions,
        closing: server.closing.child_token(),
    };
    let transport = (
        PollSender::new(output),
        stream::poll_fn(move |cx| input.poll_recv(cx)),
    );
    let closing = connection.closing.clone();
    tokio::spawn(async move {
        if let Err(error) =
            agena_mcp_server::serve_tools_transport(server.backend, transport, closing).await
        {
            tracing::debug!(%error, "ACP SSE MCP connection closed");
        }
    });

    let endpoint = Event::default()
        .event("endpoint")
        .data(format!("/messages?sessionId={id}"));
    let frames = stream::unfold(
        (outgoing, connection),
        |(mut outgoing, connection)| async move {
            let message = tokio::select! {
                biased;
                _ = connection.closing.cancelled() => return None,
                message = outgoing.recv() => message?,
            };
            Some((
                Event::default().event("message").json_data(message),
                (outgoing, connection),
            ))
        },
    );
    Ok(Sse::new(stream::once(ready(Ok(endpoint))).chain(frames))
        .keep_alive(KeepAlive::default())
        .into_response())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionId {
    session_id: Uuid,
}

async fn receive(
    State(server): State<Server>,
    Query(query): Query<SessionId>,
    Json(message): Json<ClientJsonRpcMessage>,
) -> StatusCode {
    let Some(sender) = server.sessions.lock().get(&query.session_id).cloned() else {
        return StatusCode::NOT_FOUND;
    };
    match sender.try_send(message) {
        Ok(()) => StatusCode::ACCEPTED,
        Err(mpsc::error::TrySendError::Full(_)) => StatusCode::TOO_MANY_REQUESTS,
        Err(mpsc::error::TrySendError::Closed(_)) => StatusCode::NOT_FOUND,
    }
}
