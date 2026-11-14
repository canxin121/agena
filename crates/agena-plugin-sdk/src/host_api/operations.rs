//! Typed conveniences over the single shared host operation transport.
use super::{HostClient, SessionOp, SessionOpResponse};
use crate::error::{PluginError, Result};
use async_trait::async_trait;
use serde::de::DeserializeOwned;

pub use agena_session_api::{commands::Command, queries::Query};

pub(super) fn response_value(
    response: SessionOpResponse,
    command: bool,
) -> Result<serde_json::Value> {
    match (response, command) {
        (SessionOpResponse::Query(value), false) | (SessionOpResponse::Command(value), true) => {
            Ok(value)
        }
        _ => Err(PluginError::internal(
            "Host returned the wrong operation response kind",
        )),
    }
}

fn decode<T: DeserializeOwned>(value: serde_json::Value) -> Result<T> {
    if value
        .get("result")
        .and_then(serde_json::Value::as_str)
        .is_none()
    {
        return Err(PluginError::internal(
            "Host operation response is missing its result envelope",
        ));
    }
    serde_json::from_value(
        value
            .get("data")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    )
    .map_err(PluginError::internal)
}

/// Available on concrete clients, Arc<dyn HostClient>, and &dyn HostClient.
#[async_trait]
pub trait HostClientExt: HostClient {
    async fn query_as<T: DeserializeOwned + Send>(
        &self,
        query: impl Into<Query> + Send,
    ) -> Result<T> {
        let response = self.session_op(SessionOp::Query(query.into())).await?;
        decode(response_value(response, false)?)
    }

    async fn command_as<T: DeserializeOwned + Send>(
        &self,
        command: impl Into<Command> + Send,
    ) -> Result<T> {
        let response = self.session_op(SessionOp::Command(command.into())).await?;
        decode(response_value(response, true)?)
    }
}
impl<T: HostClient + ?Sized> HostClientExt for T {}
