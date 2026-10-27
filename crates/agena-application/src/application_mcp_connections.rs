//! External MCP connection management for authenticated application clients.

use crate::{Application, ApplicationError};
use agena_api::mcp_connections::*;
use agena_mcp_client::{FileTokenStore, McpConnectionManager, management};
use agena_runtime::mcp_connections::{
    McpConnectionStore, McpManagementError, McpManagementErrorKind, validate_connection,
};
use std::sync::Arc;

impl Application {
    fn mcp_connection_store(&self) -> Result<McpConnectionStore, ApplicationError> {
        let config = self
            .runtime_configuration()
            .runtime_configuration()
            .map_err(|e| ApplicationError::internal_error(&e))?;
        Ok(McpConnectionStore::new(
            config.config_path,
            config.project_config_path,
            self.workspace_root().to_path_buf(),
        )
        .with_active_settings(
            config
                .effective_config
                .pointer("/plugins/list/agena.mcp/settings")
                .cloned()
                .unwrap_or_default(),
        ))
    }

    fn mcp_live_manager(&self) -> Result<Arc<McpConnectionManager>, ApplicationError> {
        self.runtime_control()
            .mcp_connection_manager()
            .ok_or_else(|| {
                ApplicationError::service_unavailable(
                    "Enable the built-in MCP bridge before using a live connection.",
                )
            })
    }

    pub async fn mcp_connections(&self) -> Result<McpConnectionCatalog, ApplicationError> {
        let store = self.mcp_connection_store()?;
        let mut catalog = self
            .run_blocking(move |_| store.list())
            .await?
            .map_err(mcp_management_error)?;
        management::project_catalog(
            &mut catalog,
            self.runtime_control().mcp_connection_manager().as_ref(),
        )
        .await;
        Ok(catalog)
    }

    pub async fn mcp_connection(
        &self,
        id: String,
    ) -> Result<McpConnectionDetail, ApplicationError> {
        let store = self.mcp_connection_store()?;
        let mut detail = self
            .run_blocking(move |_| store.get(&id))
            .await?
            .map_err(mcp_management_error)?;
        management::project_detail(
            &mut detail,
            self.runtime_control().mcp_connection_manager().as_ref(),
        )
        .await;
        Ok(detail)
    }

    async fn mcp_saved(&self, id: Option<String>) -> Result<McpMutationResult, ApplicationError> {
        let mut warning = self.runtime_control().reload().await.err().map(|_| "Configuration was saved, but runtime reload failed. Check diagnostics and reload before using the connection.".to_owned());
        let connection = if let Some(id) = id {
            match self.mcp_connection(id).await {
                Ok(mut detail) => {
                    if warning.is_some() && detail.summary.effective && detail.summary.enabled {
                        detail.summary.state = "pending_reload".into();
                        detail.summary.error = warning.clone();
                    }
                    Some(detail)
                }
                Err(_) => {
                    warning = Some("Configuration was saved, but its current details could not be loaded. Refresh before making another change.".into());
                    None
                }
            }
        } else {
            None
        };
        Ok(McpMutationResult {
            saved: true,
            connection,
            warning,
            reload_task_id: None,
        })
    }

    pub async fn create_mcp_connection(
        &self,
        input: McpConnectionCreateRequest,
    ) -> Result<McpMutationResult, ApplicationError> {
        let store = self.mcp_connection_store()?;
        let id = self
            .run_blocking(move |_| store.create(input))
            .await?
            .map_err(mcp_management_error)?;
        self.mcp_saved(Some(id)).await
    }

    pub async fn update_mcp_connection(
        &self,
        id: String,
        input: McpConnectionUpdateRequest,
    ) -> Result<McpMutationResult, ApplicationError> {
        let store = self.mcp_connection_store()?;
        let target = id.clone();
        self.run_blocking(move |_| store.update(&target, input))
            .await?
            .map_err(mcp_management_error)?;
        self.mcp_saved(Some(id)).await
    }

    pub async fn remove_mcp_connection(
        &self,
        id: String,
        expected_revision: String,
    ) -> Result<McpMutationResult, ApplicationError> {
        let store = self.mcp_connection_store()?;
        self.run_blocking(move |_| store.remove(&id, &expected_revision))
            .await?
            .map_err(mcp_management_error)?;
        self.mcp_saved(None).await
    }

    pub async fn set_mcp_connection_enabled(
        &self,
        id: String,
        input: McpConnectionEnabledRequest,
    ) -> Result<McpMutationResult, ApplicationError> {
        let store = self.mcp_connection_store()?;
        let target = id.clone();
        self.run_blocking(move |_| store.set_enabled(&target, input))
            .await?
            .map_err(mcp_management_error)?;
        self.mcp_saved(Some(id)).await
    }

    pub async fn set_mcp_bridge_enabled(
        &self,
        input: McpBridgeEnabledRequest,
    ) -> Result<McpConnectionCatalog, ApplicationError> {
        let store = self.mcp_connection_store()?;
        self.run_blocking(move |_| store.set_bridge_enabled(input))
            .await?
            .map_err(mcp_management_error)?;
        self.runtime_control().reload().await.map_err(|_| {
            ApplicationError::internal(
                "MCP bridge setting was saved, but runtime reload failed. Reload from Diagnostics.",
            )
        })?;
        self.mcp_connections().await
    }

    pub async fn preview_mcp_import(
        &self,
        input: McpImportPreviewRequest,
    ) -> Result<McpImportPreview, ApplicationError> {
        let store = self.mcp_connection_store()?;
        self.run_blocking(move |_| store.preview_import(&input))
            .await?
            .map_err(mcp_management_error)
    }

    pub async fn import_mcp_connections(
        &self,
        input: McpImportRequest,
    ) -> Result<McpImportResult, ApplicationError> {
        let store = self.mcp_connection_store()?;
        let ids = self
            .run_blocking(move |_| store.import(input))
            .await?
            .map_err(mcp_management_error)?;
        let result = self.mcp_saved(None).await?;
        let catalog = self.mcp_connections().await?;
        Ok(McpImportResult {
            installed: catalog
                .connections
                .into_iter()
                .filter(|c| ids.contains(&c.id))
                .collect(),
            warning: result.warning,
            reload_task_id: None,
        })
    }

    async fn mcp_configured_connection(
        &self,
        id: String,
        require_active: bool,
    ) -> Result<(String, agena_runtime::McpServerConfig), ApplicationError> {
        let store = self.mcp_connection_store()?;
        self.run_blocking(move |_| {
            if require_active {
                let detail = store.get(&id)?;
                if !detail.summary.enabled {
                    return Err(McpManagementError::invalid(
                        "Enable this connection before using it.",
                    ));
                }
                if matches!(
                    detail.summary.state.as_str(),
                    "pending_reload" | "bridge_disabled"
                ) {
                    return Err(McpManagementError::invalid(
                        "Reload configuration before using this changed connection.",
                    ));
                }
                store.resolved(&id)
            } else {
                let (_, name, raw) = store.raw(&id)?;
                Ok((name.clone(), validate_connection(&name, raw)?))
            }
        })
        .await?
        .map_err(mcp_management_error)
    }

    pub async fn reconnect_mcp_connection(
        &self,
        id: String,
    ) -> Result<McpConnectionDetail, ApplicationError> {
        let (name, _) = self.mcp_configured_connection(id.clone(), true).await?;
        self.mcp_live_manager()?
            .reconnect(&name)
            .await
            .map_err(mcp_transport_error)?;
        self.mcp_connection(id).await
    }

    pub async fn test_mcp_connection(&self, id: String) -> Result<McpTestResult, ApplicationError> {
        let (name, config) = self.mcp_configured_connection(id, false).await?;
        let workspace = self.workspace_root().to_path_buf();
        let spec =
            agena_runtime::mcp_server_spec(&name, &config, &workspace).map_err(mcp_input_error)?;
        let timeouts = config.timeouts();
        let store = self.mcp_connection_store()?;
        let token_store_enabled = self
            .run_blocking(move |_| store.token_store_enabled())
            .await?
            .map_err(mcp_management_error)?;
        Ok(management::test_connection(
            &name,
            spec,
            workspace,
            timeouts.connect_ms,
            timeouts.request_ms,
            token_store_enabled,
        )
        .await)
    }

    pub async fn mcp_connection_capabilities(
        &self,
        id: String,
        kind: String,
        cursor: Option<String>,
    ) -> Result<McpCapabilityPage, ApplicationError> {
        let (name, _) = self.mcp_configured_connection(id, true).await?;
        management::capabilities(self.mcp_live_manager()?.as_ref(), &name, &kind, cursor)
            .await
            .map_err(mcp_transport_error)
    }

    pub async fn read_mcp_connection_resource(
        &self,
        id: String,
        input: McpResourceReadRequest,
    ) -> Result<serde_json::Value, ApplicationError> {
        let (name, _) = self.mcp_configured_connection(id, true).await?;
        let result = self
            .mcp_live_manager()?
            .read_resource(&name, &input.uri)
            .await
            .map_err(mcp_transport_error)?;
        serde_json::to_value(result).map_err(|e| ApplicationError::internal_error(&e))
    }

    pub async fn get_mcp_connection_prompt(
        &self,
        id: String,
        input: McpPromptGetRequest,
    ) -> Result<serde_json::Value, ApplicationError> {
        let (name, _) = self.mcp_configured_connection(id, true).await?;
        let arguments = management::prompt_arguments(input.arguments).map_err(mcp_input_error)?;
        let result = self
            .mcp_live_manager()?
            .get_prompt(&name, &input.name, Some(arguments))
            .await
            .map_err(mcp_transport_error)?;
        serde_json::to_value(result).map_err(|e| ApplicationError::internal_error(&e))
    }

    async fn mcp_http_connection(
        &self,
        id: String,
    ) -> Result<(String, url::Url, agena_runtime::McpServerConfig), ApplicationError> {
        let (name, config) = self.mcp_configured_connection(id, false).await?;
        let agena_runtime::McpServerConfig::Http { endpoint, .. } = &config else {
            return Err(ApplicationError::bad_request(
                "Authentication is available for HTTP connections.",
            ));
        };
        let url = url::Url::parse(&endpoint.url)
            .map_err(|_| ApplicationError::bad_request("Invalid HTTP endpoint."))?;
        Ok((name, url, config))
    }

    pub async fn set_mcp_connection_credential(
        &self,
        id: String,
        token: Option<String>,
    ) -> Result<McpConnectionDetail, ApplicationError> {
        let (name, endpoint, config) = self.mcp_http_connection(id.clone()).await?;
        if !matches!(
            config,
            agena_runtime::McpServerConfig::Http {
                auth: Some(agena_runtime::McpHttpAuthConfig::BearerFromStore),
                ..
            }
        ) {
            return Err(ApplicationError::bad_request(
                "Select stored bearer authentication before changing its credential.",
            ));
        }
        if token.as_ref().is_some_and(|t| t.trim().is_empty()) {
            return Err(ApplicationError::bad_request(
                "Bearer token must not be empty.",
            ));
        }
        let key = agena_mcp_client::credential_store_name(&name, &endpoint);
        self.run_blocking(move |_| {
            let store = FileTokenStore::new();
            match token {
                Some(token) => store.put_bearer(&key, &token),
                None => store.delete(&key),
            }
        })
        .await?
        .map_err(|_| ApplicationError::internal("Could not update the stored MCP credential."))?;
        let detail = self.mcp_connection(id.clone()).await?;
        if detail.summary.effective
            && detail.summary.enabled
            && !matches!(
                detail.summary.state.as_str(),
                "pending_reload" | "bridge_disabled" | "invalid"
            )
            && let Some(manager) = self.runtime_control().mcp_connection_manager()
        {
            let _ = manager.reconnect(&name).await;
        }
        self.mcp_connection(id).await
    }

    pub async fn start_mcp_connection_auth(
        &self,
        id: String,
        input: McpOAuthStartRequest,
    ) -> Result<McpOAuthStart, ApplicationError> {
        let store = self.mcp_connection_store()?;
        let (detail, raw) = self
            .run_blocking(move |_| store.snapshot(&id))
            .await?
            .map_err(mcp_management_error)?;
        let name = detail.summary.name.clone();
        let config = validate_connection(&name, raw).map_err(mcp_management_error)?;
        let agena_runtime::McpServerConfig::Http {
            endpoint,
            auth:
                Some(agena_runtime::McpHttpAuthConfig::OAuth {
                    scopes,
                    client_id,
                    client_secret,
                    client_metadata_url,
                }),
            ..
        } = config
        else {
            return Err(ApplicationError::bad_request(
                "Select OAuth authentication before signing in.",
            ));
        };
        let endpoint = url::Url::parse(&endpoint.url)
            .map_err(|_| ApplicationError::bad_request("Invalid HTTP endpoint."))?;
        let flow = self
            .mcp_live_manager()?
            .oauth_sessions()
            .begin_with_registration(
                &name,
                endpoint,
                scopes,
                &input.redirect_uri,
                format!("{}:{}", detail.summary.id, detail.summary.revision),
                agena_mcp_client::McpOAuthRegistration {
                    client_id,
                    client_secret,
                    client_metadata_url,
                },
            )
            .await
            .map_err(mcp_transport_error)?;
        Ok(McpOAuthStart {
            flow_id: flow.flow_id,
            authorization_url: flow.authorization_url,
            expires_at: flow.expires_at,
        })
    }

    pub async fn complete_mcp_connection_auth(
        &self,
        id: String,
        input: McpOAuthCompleteRequest,
    ) -> Result<McpConnectionDetail, ApplicationError> {
        let detail = self.mcp_connection(id.clone()).await?;
        let manager = self.mcp_live_manager()?;
        manager
            .oauth_sessions()
            .complete(
                &input.flow_id,
                &input.callback_url,
                &format!("{}:{}", detail.summary.id, detail.summary.revision),
            )
            .await
            .map_err(mcp_transport_error)?;
        if detail.summary.effective
            && detail.summary.enabled
            && !matches!(
                detail.summary.state.as_str(),
                "pending_reload" | "bridge_disabled" | "invalid"
            )
        {
            let _ = manager.reconnect(&detail.summary.name).await;
        }
        self.mcp_connection(id).await
    }

    pub async fn clear_mcp_connection_auth(
        &self,
        id: String,
        flow_id: Option<String>,
    ) -> Result<McpConnectionDetail, ApplicationError> {
        let (name, endpoint, _) = self.mcp_http_connection(id.clone()).await?;
        let manager = self.mcp_live_manager()?;
        let detail = self.mcp_connection(id.clone()).await?;
        if let Some(flow_id) = flow_id {
            manager
                .oauth_sessions()
                .cancel_checked(
                    &flow_id,
                    &format!("{}:{}", detail.summary.id, detail.summary.revision),
                )
                .await
                .map_err(mcp_transport_error)?;
        } else {
            manager
                .oauth_sessions()
                .logout(&name, &endpoint)
                .await
                .map_err(mcp_transport_error)?;
            if detail.summary.effective
                && detail.summary.enabled
                && !matches!(
                    detail.summary.state.as_str(),
                    "pending_reload" | "bridge_disabled" | "invalid"
                )
            {
                let _ = manager.reconnect(&name).await;
            }
        }
        self.mcp_connection(id).await
    }
}

fn mcp_management_error(error: McpManagementError) -> ApplicationError {
    let mut result = match error.kind {
        McpManagementErrorKind::Invalid => {
            ApplicationError::bad_request("Invalid MCP connection configuration.")
        }
        McpManagementErrorKind::NotFound => {
            ApplicationError::not_found("MCP connection not found.")
        }
        McpManagementErrorKind::Conflict => {
            ApplicationError::conflict("MCP connection changed; refresh and retry.")
        }
        McpManagementErrorKind::Internal => {
            ApplicationError::internal("Could not access MCP connection settings.")
        }
    };
    result.failure.user =
        agena_failure::UserPresentation::validated("mcp-management", error.message);
    result
}
fn mcp_input_error(message: String) -> ApplicationError {
    let mut result = ApplicationError::bad_request("Invalid MCP connection request.");
    result.failure.user = agena_failure::UserPresentation::validated("mcp-request", message);
    result
}
fn mcp_transport_error(error: agena_mcp_client::McpError) -> ApplicationError {
    mcp_input_error(error.user_message())
}
