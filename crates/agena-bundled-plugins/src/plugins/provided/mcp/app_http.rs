use agena_mcp_client::{McpError, McpHttpNetworkAuthorizer, McpResult, ServerSpec};
use agena_process::{sandbox::SandboxPolicy, sandbox_http::WorkerCancellation};
use std::{net::SocketAddr, path::Path, sync::Mutex};

use super::*;

struct AppSetupAuthority(Mutex<Option<Arc<ConnectionNetworkAuthority>>>);

impl AppSetupAuthority {
    fn current(&self) -> McpResult<Arc<ConnectionNetworkAuthority>> {
        self.0
            .lock()
            .ok()
            .and_then(|authority| authority.clone())
            .ok_or(McpError::PermissionDenied)
    }
}

#[async_trait::async_trait]
impl McpHttpNetworkAuthorizer for AppSetupAuthority {
    fn cancellation(&self) -> Option<WorkerCancellation> {
        self.current()
            .ok()
            .and_then(|authority| authority.cancellation())
    }

    async fn authorize(&self, target: &str) -> McpResult<()> {
        self.current()?.authorize(target).await
    }

    async fn authorize_resolved(&self, target: &str, address: SocketAddr) -> McpResult<bool> {
        self.current()?.authorize_resolved(target, address).await
    }

    async fn sandbox_policy(&self, target: &str) -> McpResult<SandboxPolicy> {
        self.current()?.sandbox_policy(target).await
    }

    async fn header_helper_policy(&self, cwd: &Path) -> McpResult<SandboxPolicy> {
        self.current()?.header_helper_policy(cwd).await
    }
}

pub(super) struct AppHttpSetup {
    pub manager: Arc<McpConnectionManager>,
    authority: Arc<AppSetupAuthority>,
}

impl Drop for AppHttpSetup {
    fn drop(&mut self) {
        self.authority
            .0
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take();
    }
}

impl McpPlugin {
    pub(super) async fn prepare_app_http(
        &self,
        context: &ToolInvokeContext<'_>,
        server: &str,
        spec: &ServerSpec,
    ) -> SdkResult<Option<AppHttpSetup>> {
        if context.session_id <= 0
            || context.call_id <= 0
            || !matches!(spec, ServerSpec::Http { .. })
        {
            return Ok(None);
        }
        self.open_app_http(context, server).await.map(Some)
    }

    pub(super) async fn open_app_http(
        &self,
        context: &ToolInvokeContext<'_>,
        server: &str,
    ) -> SdkResult<AppHttpSetup> {
        let original = self.connection_network_authority()?;
        let authority = Arc::new(AppSetupAuthority(Mutex::new(Some(original.clone()))));
        let workspace =
            (!context.workspace_root.is_empty()).then(|| Path::new(context.workspace_root));
        let manager = self
            .manager
            .for_app_http_scope_in_workspace(server, authority.clone(), workspace)
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        let setup = AppHttpSetup { manager, authority };
        self.refresh_http_roots(&setup.manager, &original).await?;
        original.finish(setup.manager.reconnect(server).await)?;
        Ok(setup)
    }
}
