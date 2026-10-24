use std::{path::Path, sync::Arc};

use agena_storage::instance_state::InstanceStateRepository;
use serde::{Serialize, de::DeserializeOwned};

static STATE_CODECS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

fn database_diagnostic(
    context: impl AsRef<str>,
    error: &(dyn std::error::Error + 'static),
) -> String {
    agena_failure::diagnostic::format_error_chain_with_context(context, error)
}

pub(crate) const KV_KEY_TERMINAL_SESSION_REGISTRY: &str = "terminal.sessionRegistry";
pub(crate) const KV_KEY_WORKSPACE_PREVIEW_SERVER_STATE: &str = "workspacePreview.state.server";
pub(crate) const KV_KEY_MCP_SERVER_CONTROL: &str = "mcp.server.control";
pub(crate) const KV_KEY_MCP_OAUTH_RUNTIME: &str = "mcp.oauth.runtime";

/// Typed application state on the runtime's existing instance database.
/// The runtime owns schema initialization, connection policy and shutdown.
#[derive(Clone)]
pub(crate) struct ServerStateDb {
    store: Arc<dyn InstanceStateRepository>,
}

impl std::fmt::Debug for ServerStateDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerStateDb")
            .field("path", &self.path())
            .finish()
    }
}

impl ServerStateDb {
    pub(crate) async fn from_store(
        store: Arc<dyn InstanceStateRepository>,
    ) -> Result<Self, String> {
        Ok(Self { store })
    }

    pub(crate) fn path(&self) -> &Path {
        self.store.path()
    }

    pub(crate) async fn get_json<T: DeserializeOwned + Send + 'static>(
        &self,
        key: &str,
    ) -> Result<Option<T>, String> {
        let Some(payload) = self.store.get_payload(key).await.map_err(|error| {
            database_diagnostic("failed to read a server-state database value", &error)
        })?
        else {
            return Ok(None);
        };
        STATE_CODECS
            .run(move || serde_json::from_str::<T>(&payload))
            .await
            .map_err(|error| database_diagnostic("server-state decode worker failed", &error))?
            .map(Some)
            .map_err(|error| {
                database_diagnostic("failed to decode typed server-state data", &error)
            })
    }

    pub(crate) async fn set_json<T: Serialize + Send + 'static>(
        &self,
        key: &str,
        value: T,
    ) -> Result<(), String> {
        // Serialize before entering the shared writer queue. Large registries
        // must not hold up session commits while their JSON is being encoded.
        let payload = STATE_CODECS
            .run(move || serde_json::to_string(&value))
            .await
            .map_err(|error| database_diagnostic("server-state encode worker failed", &error))?
            .map_err(|error| {
                database_diagnostic("failed to encode typed server-state data", &error)
            })?;
        self.store.set_payload(key, payload).await.map_err(|error| {
            database_diagnostic("failed to write a server-state database value", &error)
        })
    }
}
