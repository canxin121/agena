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

mod discovery;
pub use discovery::resolve_server_url;

pub mod error;
pub mod http;
pub mod ws;

pub use agena_api;
pub use error::ClientError;
pub use http::{
    AgenaClient, BtwSubscription, ContentSubscription, NotificationSubscription, SessionConnection,
};
pub use ws::{Subscription, SubscriptionEvent, WsClient};
