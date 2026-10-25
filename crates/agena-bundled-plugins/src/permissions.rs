//! Permission enforcement for plugins that own their I/O. Host-owned storage,
//! immutable artifacts and configured discovery are separate host capabilities;
//! user-selected paths and outbound requests must be authorized before use.

use std::sync::{Arc, OnceLock};

use agena_domain::PathKind;
use agena_plugin_sdk::{
    PluginError, Result,
    host_api::{HostClient, PathPermissionQuery},
};

#[derive(Default)]
pub(crate) struct PluginPermissions(OnceLock<Arc<dyn HostClient>>);

impl PluginPermissions {
    pub(crate) fn read_filter(
        &self,
        interactive: bool,
    ) -> Result<agena_tool::path_filter::PathFilter<PluginError>> {
        read_filter(
            self.0.get().cloned().ok_or_else(|| {
                PluginError::internal("plugin permission host is not initialized")
            })?,
            interactive,
        )
    }
    pub(crate) fn initialize(&self, host: Arc<dyn HostClient>) -> Result<()> {
        self.0
            .set(host)
            .map_err(|_| PluginError::internal("plugin permission host initialized more than once"))
    }

    pub(crate) async fn path(&self, kind: PathKind, path: &str) -> Result<()> {
        let host = self.0.get().ok_or_else(|| {
            PluginError::from_kind(
                agena_plugin_sdk::PluginErrorKind::HostUnavailable,
                "plugin permission host is not initialized",
            )
        })?;
        require_path(host.as_ref(), kind, path).await
    }
}

/// Construct on the async caller, then use only inside a blocking worker.
/// Task-local callback authority must be restored for every concrete child.
pub(crate) fn read_filter(
    host: Arc<dyn HostClient>,
    interactive: bool,
) -> Result<agena_tool::path_filter::PathFilter<PluginError>> {
    let runtime =
        tokio::runtime::Handle::try_current().map_err(|e| PluginError::internal_error(&e))?;
    let context = agena_plugin_sdk::host_api::current_host_callback_context().unwrap_or_default();
    Ok(agena_tool::path_filter::PathFilter::new(move |path| {
        let host = host.clone();
        let request = PathPermissionQuery::read(path.to_string_lossy().into_owned());
        runtime.block_on(
            agena_plugin_sdk::host_api::run_in_isolated_host_callback_context(
                context.clone(),
                async move {
                    if interactive {
                        host.require_path_permission(request).await
                    } else if host.check_path_permission(request).await?.is_allowed() {
                        Ok(())
                    } else {
                        Err(PluginError::from_kind(
                            agena_plugin_sdk::PluginErrorKind::PolicyDenied,
                            "memory recall requires ungranted child path access",
                        ))
                    }
                },
            ),
        )
    }))
}

pub(crate) async fn require_path(host: &dyn HostClient, kind: PathKind, path: &str) -> Result<()> {
    host.require_path_permission(PathPermissionQuery {
        kind,
        path: path.to_owned(),
    })
    .await
}
