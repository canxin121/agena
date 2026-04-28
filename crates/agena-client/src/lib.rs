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
pub use agena_event;
pub use error::ClientError;
pub use http::AgenaClient;
pub use ws::{Subscription, WsClient};
