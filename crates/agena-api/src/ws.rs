//! WebSocket framing: duplex JSON-line envelope used by `agena-api-server`'s
//! `/api/v1/ws` endpoint.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::commands::{Command, CommandResult};
use crate::error::ApiError;
use crate::notifications::Notification;
use crate::queries::{Query, QueryResult};
use crate::subscribe::{SubscribeRequest, SubscriptionId};

/// Client-supplied request id, echoed in the matching response. Allows the
/// client to correlate replies for in-flight commands/queries.
pub type RequestId = SmolStr;

/// Messages flowing client → server over the WS.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Send a command. Server responds with [`ServerMessage::CommandResult`]
    /// or [`ServerMessage::Error`] keyed by `id`.
    Command {
        id: RequestId,
        #[serde(flatten)]
        command: Command,
    },
    /// Run a query. Server responds with [`ServerMessage::QueryResult`].
    Query {
        id: RequestId,
        #[serde(flatten)]
        query: Query,
    },
    /// Open a new subscription. Server responds with
    /// [`ServerMessage::Subscribed`] (or [`ServerMessage::Error`]) and then
    /// streams [`Notification`]s under that `subscription` id.
    Subscribe {
        id: SubscriptionId,
        #[serde(flatten)]
        request: SubscribeRequest,
    },
    /// Close an existing subscription. Server responds with
    /// [`ServerMessage::Unsubscribed`].
    Unsubscribe { id: SubscriptionId },
    /// Heartbeat. Server replies with [`ServerMessage::Pong`].
    Ping {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        nonce: Option<SmolStr>,
    },
}

/// Messages flowing server → client over the WS.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Greeting sent immediately after upgrade. Carries the protocol version
    /// the server speaks; clients downgrade their expectations if it differs.
    Hello { protocol_version: u32 },
    /// Reply to a [`ClientMessage::Command`].
    CommandResult {
        id: RequestId,
        #[serde(flatten)]
        result: CommandResult,
    },
    /// Reply to a [`ClientMessage::Query`].
    QueryResult {
        id: RequestId,
        #[serde(flatten)]
        result: QueryResult,
    },
    /// Acknowledgement of a successful subscribe.
    Subscribed { id: SubscriptionId },
    /// Acknowledgement of a successful unsubscribe.
    Unsubscribed { id: SubscriptionId },
    /// Asynchronous push from a subscription.
    Notification(Notification),
    /// Generic per-request error. `id` matches the originating request, or is
    /// absent for transport-level / unsolicited errors.
    Error {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<RequestId>,
        #[serde(flatten)]
        error: ApiError,
    },
    /// Heartbeat reply.
    Pong {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        nonce: Option<SmolStr>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CancelTurnParams, Command};

    #[test]
    fn client_command_round_trip() {
        let msg = ClientMessage::Command {
            id: "cmd-1".into(),
            command: Command::CancelTurn(CancelTurnParams { session_id: 7 }),
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["type"], "command");
        assert_eq!(json["method"], "cancel_turn");
        assert_eq!(json["id"], "cmd-1");
        let back: ClientMessage = serde_json::from_value(json).unwrap();
        let ClientMessage::Command { id, command } = back else {
            panic!("expected command");
        };
        assert_eq!(id, "cmd-1");
        assert!(matches!(command, Command::CancelTurn(_)));
    }
}
