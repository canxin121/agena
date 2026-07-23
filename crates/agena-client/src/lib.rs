//! # agena-client
//!
//! Official Rust SDK for the v2 agena API. Talks REST for one-shot
//! commands/queries and WebSocket for live event subscriptions over a single
//! multiplexed connection.
//!
//! ## Quick start
//!
//! ```ignore
//! use agena_client::AgenaClient;
//! use agena_api::{Scope, subscribe::SubscribeRequest};
//!
//! let client = AgenaClient::new("http://localhost:7878").await?;
//! let session = client.create_session("workspace-1", 1, "demo", None).await?;
//! let mut sub = client
//!     .subscribe(SubscribeRequest {
//!         scope: Scope::Session { session_id: session.id },
//!         kinds: None,
//!         since_seq_global: None,
//!     })
//!     .await?;
//! while let Some(event) = sub.recv().await {
//!     println!("got {}: {:?}", event.kind.tag_str(), event);
//! }
//! ```

pub mod error;
pub mod http;
pub mod ws;

pub use agena_api;
pub use error::ClientError;
pub use http::{AgenaClient, NotificationSubscription};
pub use ws::{Subscription, SubscriptionEvent, WsClient};

#[cfg(test)]
mod protocol_contract_tests {
    use agena_api::{
        PROTOCOL_VERSION,
        commands::{Command, CreateWorkspaceParams},
    };

    #[test]
    fn client_uses_the_current_api_protocol_version() {
        assert_eq!(PROTOCOL_VERSION, 1);
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
