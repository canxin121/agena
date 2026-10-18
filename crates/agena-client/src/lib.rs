//! # agena-client
//!
//! Official Rust SDK for the v2 agena API. Uses REST for commands and queries,
//! bounded session/content streams for live reads, and raw JSONL streaming for
//! portable archives. WebSocket subscriptions are also available.
//!
//! ## Quick start
//!
//! ```no_run
//! use agena_client::{AgenaClient, ClientError, SubscriptionEvent};
//!
//! async fn observe_session(server: &str, session_id: i64) -> Result<(), ClientError> {
//!     let client = AgenaClient::connect_server(server, None, None).await?;
//!     let mut connection = client.connect_session(session_id).await?;
//!     while let Some(update) = connection.subscription.recv().await {
//!         match update? {
//!             SubscriptionEvent::Lagged(_) => {
//!                 connection = client.connect_session(session_id).await?;
//!             }
//!             update => println!("got update: {update:?}"),
//!         }
//!     }
//!     Ok(())
//! }
//! ```

pub mod error;
pub mod http;
pub mod ws;

pub use agena_api;
pub use error::ClientError;
pub use http::{
    AgenaClient, BtwSubscription, ContentSubscription, NotificationSubscription, SessionConnection,
};
pub use ws::{Subscription, SubscriptionEvent, WsClient};

#[cfg(test)]
mod protocol_contract_tests {
    use agena_api::{
        PROTOCOL_VERSION,
        commands::{Command, CreateWorkspaceParams},
    };

    #[test]
    fn client_uses_the_current_api_protocol_version() {
        assert_eq!(PROTOCOL_VERSION, 2);
    }

    #[test]
    fn command_json_is_shared_with_the_server_contract() {
        let command = Command::CreateWorkspace(CreateWorkspaceParams {
            path: "/tmp/example".to_owned(),
        });
        let value = serde_json::to_value(command).expect("serialize command");
        assert_eq!(
            value,
            serde_json::json!({
                "method": "create_workspace",
                "params": { "path": "/tmp/example" }
            })
        );
        let decoded: Command = serde_json::from_value(value).expect("deserialize command");
        let Command::CreateWorkspace(params) = decoded else {
            panic!("command must round-trip through the shared API contract");
        };
        assert_eq!(params.path, "/tmp/example");
    }
}
