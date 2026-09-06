//! Replayable initialization data. Callback authority is minted for each
//! invocation; no authority token from an earlier call is retained here.

use std::path::PathBuf;
use std::sync::Weak;
use std::time::Duration;

use super::PluginTransport;
use crate::error::TransportError;
use crate::host::HostHandle;
use crate::sdk::host_api::HostCallbackContext;
use crate::sdk::rpc::method;
use crate::sdk::{InitContext, InitOutcome, PluginKey, PluginManifest};

#[derive(Clone)]
pub struct PluginInitialization {
    pub host: Weak<HostHandle>,
    pub plugin_id: PluginKey,
    pub manifest: PluginManifest,
    pub agena_version: String,
    pub workspace_root: PathBuf,
    pub settings: serde_json::Value,
}

impl PluginInitialization {
    pub async fn initialize<T: PluginTransport + ?Sized>(
        &self,
        transport: &T,
    ) -> Result<InitOutcome, TransportError> {
        let host = self.host.upgrade().ok_or_else(|| {
            TransportError::disconnected("plugin initialization host was dropped")
        })?;
        let context = InitContext {
            agena_version: self.agena_version.clone(),
            workspace_root: self.workspace_root.clone(),
            plugin_id: self.plugin_id.clone(),
            host_callback_url: host.callback_url(&self.plugin_id.to_string()),
            host_callback_token: host.callback_token(&self.plugin_id.to_string()).await,
            settings: self.settings.clone(),
            protocol_version: crate::sdk::rpc::PROTOCOL_VERSION,
        };
        let request = host.run_in_authorized_callback_context(
            &self.plugin_id,
            HostCallbackContext {
                workspace_root: Some(self.workspace_root.to_string_lossy().into_owned()),
                ..Default::default()
            },
            transport.dispatch(method::META_INIT, serde_json::to_value(context)?),
        );
        let result = tokio::time::timeout(Duration::from_secs(30), request)
            .await
            .map_err(|error| {
                TransportError::timeout_error("meta/init timed out after 30 seconds", &error)
            })??;
        let outcome: InitOutcome = serde_json::from_value(result)?;
        if outcome.manifest != self.manifest {
            return Err(TransportError::Io("plugin manifest changed between `meta/manifest` and `meta/init`; manifests must be immutable during initialization".into()));
        }
        if outcome.protocol_version != crate::sdk::rpc::PROTOCOL_VERSION {
            return Err(TransportError::Io(format!(
                "meta/init returned protocol version {}, expected {}",
                outcome.protocol_version,
                crate::sdk::rpc::PROTOCOL_VERSION
            )));
        }
        Ok(outcome)
    }

    pub(crate) async fn reinitialize<T: PluginTransport + ?Sized>(
        &self,
        transport: &T,
    ) -> Result<InitOutcome, TransportError> {
        let manifest = tokio::time::timeout(
            Duration::from_secs(30),
            transport.dispatch(method::META_MANIFEST, serde_json::json!({})),
        )
        .await
        .map_err(|error| {
            TransportError::timeout_error("meta/manifest timed out during plugin restart", &error)
        })??;
        let manifest: PluginManifest = serde_json::from_value(manifest)?;
        if manifest != self.manifest {
            return Err(TransportError::Io("plugin manifest changed after restart; reload the plugin to validate its new contract".into()));
        }
        self.initialize(transport).await
    }
}
