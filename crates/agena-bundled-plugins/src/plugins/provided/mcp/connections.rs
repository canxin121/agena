//! Persistent connection tools share the UI store and retain callback permissions.
#[cfg(test)]
mod tests;
use super::*;
use agena_domain::{PathKind, mcp_connections::*};
use agena_mcp_client::{FileTokenStore, HttpAuth, McpToolPolicy, ServerSpec, management};
use agena_plugin_host::sdk::{HostClient, host_api::HostMcpServerSpec};
use agena_runtime_config::{
    McpHttpAuthConfig, McpServerConfig,
    mcp_connections::{
        McpConnectionStore, McpManagementError, McpManagementResult, validate_connection,
    },
};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
pub(super) struct ConnectionListInput {}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
pub(super) struct ConnectionIdInput {
    /// Opaque ID returned by connections.list or connections.get.
    pub id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, ToolInput)]
pub(super) struct ConnectionCreateInput {
    #[serde(flatten)]
    pub request: McpConnectionCreateRequest,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, ToolInput)]
pub(super) struct ConnectionUpdateInput {
    pub id: String,
    #[serde(flatten)]
    pub request: McpConnectionUpdateRequest,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
pub(super) struct ConnectionDeleteInput {
    pub id: String,
    pub expected_revision: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, ToolInput)]
pub(super) struct ConnectionEnabledInput {
    pub id: String,
    #[serde(flatten)]
    pub request: McpConnectionEnabledRequest,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, ToolInput)]
pub(super) struct ConnectionImportPreviewInput {
    #[serde(flatten)]
    pub request: McpImportPreviewRequest,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, ToolInput)]
pub(super) struct ConnectionImportInput {
    #[serde(flatten)]
    pub request: McpImportRequest,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
pub(super) struct ConnectionCredentialInput {
    pub id: String,
    /// Bearer credential to store. Never returned by this tool.
    pub token: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
pub(super) struct ConnectionAuthStartInput {
    pub id: String,
    pub redirect_uri: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
pub(super) struct ConnectionAuthCompleteInput {
    pub id: String,
    pub flow_id: String,
    pub callback_url: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
pub(super) struct ConnectionAuthCancelInput {
    pub id: String,
    pub flow_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, ToolInput)]
pub(super) struct ConnectionBridgeInput {
    #[serde(flatten)]
    pub request: McpBridgeEnabledRequest,
}

fn store_error(error: McpManagementError) -> PluginError {
    PluginError::invalid_params(error.message)
}
fn transport_error(error: agena_mcp_client::McpError) -> PluginError {
    PluginError::invalid_params(error.user_message())
}
async fn io<T: Send + 'static>(
    f: impl FnOnce() -> McpManagementResult<T> + Send + 'static,
) -> SdkResult<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| PluginError::internal("MCP configuration operation could not finish."))?
        .map_err(store_error)
}
fn output<T: Serialize>(label: &str, value: T) -> SdkResult<ToolInvokeOutput> {
    let value = serde_json::to_value(value)
        .map_err(|_| PluginError::internal("Could not encode MCP management result."))?;
    Ok(ToolInvokeOutput::from_parts(
        label,
        label,
        serde_json::to_string_pretty(&value).unwrap_or_default(),
        Some(value),
        BTreeMap::new(),
        vec![],
    ))
}

struct ConnectionOAuthAuthority {
    host: Arc<dyn HostClient>,
    denial: std::sync::Mutex<Option<PluginError>>,
}
#[async_trait::async_trait]
impl agena_mcp_client::McpOAuthNetworkAuthorizer for ConnectionOAuthAuthority {
    async fn authorize(&self, target: &str) -> agena_mcp_client::McpResult<()> {
        if let Err(error) = self
            .host
            .require_network_permission(target.to_owned())
            .await
        {
            if let Ok(mut denial) = self.denial.lock() {
                *denial = Some(error);
            }
            return Err(agena_mcp_client::McpError::PermissionDenied);
        }
        Ok(())
    }
}
impl ConnectionOAuthAuthority {
    fn finish<T>(&self, result: agena_mcp_client::McpResult<T>) -> SdkResult<T> {
        if let Some(error) = self.denial.lock().ok().and_then(|mut denial| denial.take()) {
            return Err(error);
        }
        result.map_err(transport_error)
    }
}

impl McpPlugin {
    fn connection_host(&self) -> SdkResult<&Arc<dyn HostClient>> {
        self.host
            .get()
            .ok_or_else(|| PluginError::internal("MCP plugin invoked before initialization."))
    }

    async fn connection_store(
        &self,
        write: Option<McpConnectionScope>,
    ) -> SdkResult<McpConnectionStore> {
        let host = self.connection_host()?;
        let meta = host.read_config(Some("meta".into())).await?;
        let path = |key: &str| -> SdkResult<PathBuf> {
            meta.get(key)
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .ok_or_else(|| PluginError::internal("MCP configuration paths are unavailable."))
        };
        let global = path("config_path")?;
        let workspace = path("project_config_path")?;
        let root = workspace
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| PluginError::internal("MCP workspace path is unavailable."))?
            .to_path_buf();
        for path in [&global, &workspace] {
            crate::permissions::require_path(
                host.as_ref(),
                PathKind::Read,
                &path.to_string_lossy(),
            )
            .await?;
        }
        if let Some(scope) = write {
            let path = if scope == McpConnectionScope::Global {
                &global
            } else {
                &workspace
            };
            crate::permissions::require_path(
                host.as_ref(),
                PathKind::Write,
                &path.to_string_lossy(),
            )
            .await?;
        }
        let active = host
            .read_config(Some("config.plugins.list.\"agena.mcp\".settings".into()))
            .await?;
        Ok(McpConnectionStore::new(global, workspace, root).with_active_settings(active))
    }

    async fn writable_connection_store(&self, id: &str) -> SdkResult<McpConnectionStore> {
        let store = self.connection_store(None).await?;
        let target = id.to_owned();
        let scope = io(move || store.raw(&target).map(|(scope, _, _)| scope)).await?;
        self.connection_store(Some(scope)).await
    }

    fn connection_oauth_authority(&self) -> SdkResult<Arc<ConnectionOAuthAuthority>> {
        Ok(Arc::new(ConnectionOAuthAuthority {
            host: Arc::clone(self.connection_host()?),
            denial: Default::default(),
        }))
    }

    async fn connection_secret_access(&self) -> SdkResult<()> {
        let path = agena_storage_layout::StorageLayout::current().secrets_dir();
        for kind in [PathKind::Read, PathKind::Write] {
            crate::permissions::require_path(
                self.connection_host()?.as_ref(),
                kind,
                &path.to_string_lossy(),
            )
            .await?;
        }
        Ok(())
    }

    async fn authorize_connection(
        &self,
        store: &McpConnectionStore,
        name: &str,
        config: &McpServerConfig,
    ) -> SdkResult<ServerSpec> {
        let spec = connection_spec(config, &store.workspace_root)?;
        let authority = match &spec {
            ServerSpec::Stdio {
                command,
                args,
                env,
                cwd,
                ..
            } => HostMcpServerSpec::Stdio {
                command: command.clone(),
                args: args.clone(),
                env: env
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
                cwd: cwd.as_ref().map(|p| p.to_string_lossy().into_owned()),
            },
            ServerSpec::Http { url, .. } => HostMcpServerSpec::Http {
                url: url.to_string(),
                bearer: None,
                headers: BTreeMap::new(),
            },
        };
        self.connection_host()?
            .require_mcp_server_permission(authority)
            .await?;
        if matches!(
            config,
            McpServerConfig::Http {
                auth: Some(McpHttpAuthConfig::BearerFromStore | McpHttpAuthConfig::OAuth { .. }),
                ..
            }
        ) {
            self.connection_secret_access().await?;
        }
        config.validate(name).map_err(PluginError::invalid_params)?;
        Ok(spec)
    }

    async fn authorize_saved_connection(
        &self,
        store: &McpConnectionStore,
        scope: McpConnectionScope,
        name: &str,
        config: &McpServerConfig,
    ) -> SdkResult<()> {
        if !config.enabled() {
            return Ok(());
        }
        let cloned = store.clone();
        let catalog = io(move || cloned.list()).await?;
        if scope == McpConnectionScope::Global
            && catalog
                .connections
                .iter()
                .any(|c| c.scope == McpConnectionScope::Workspace && c.name == name)
        {
            return Ok(());
        }
        // Saving a configuration grants no process/network permission implicitly.
        self.authorize_connection(store, name, config).await?;
        Ok(())
    }

    async fn connection_saved(
        &self,
        store: McpConnectionStore,
        id: Option<String>,
    ) -> SdkResult<ToolInvokeOutput> {
        let (reload_task_id, warning) = match self.connection_host()?.request_config_reload().await {
            Ok(task) => (Some(task.task_id), Some("Configuration saved. Runtime reload is queued after this tool call; activation is not yet confirmed.".into())),
            Err(_) => (None, Some("Configuration saved, but reload could not be queued. Reload from Diagnostics before using the new settings.".into())),
        };
        let connection = if let Some(id) = id {
            Some(io(move || store.get(&id)).await?)
        } else {
            None
        };
        output(
            "MCP connection saved",
            McpMutationResult {
                saved: true,
                connection,
                warning,
                reload_task_id,
            },
        )
    }

    pub(super) async fn connection_list(&self) -> SdkResult<ToolInvokeOutput> {
        let store = self.connection_store(None).await?;
        let mut catalog = io(move || store.list()).await?;
        management::project_catalog(&mut catalog, Some(&self.manager)).await;
        output("MCP connections", catalog)
    }
    pub(super) async fn connection_get(&self, id: &str) -> SdkResult<ToolInvokeOutput> {
        output("MCP connection", self.connection_detail(id).await?)
    }
    async fn connection_detail(&self, id: &str) -> SdkResult<McpConnectionDetail> {
        let store = self.connection_store(None).await?;
        let id = id.to_owned();
        let mut detail = io(move || store.get(&id)).await?;
        management::project_detail(&mut detail, Some(&self.manager)).await;
        Ok(detail)
    }
    pub(super) async fn connection_create(
        &self,
        input: McpConnectionCreateRequest,
    ) -> SdkResult<ToolInvokeOutput> {
        let store = self.connection_store(Some(input.scope)).await?;
        let config =
            validate_connection(input.name.trim(), input.config.clone()).map_err(store_error)?;
        self.authorize_saved_connection(&store, input.scope, input.name.trim(), &config)
            .await?;
        let cloned = store.clone();
        let id = io(move || cloned.create(input)).await?;
        self.connection_saved(store, Some(id)).await
    }
    pub(super) async fn connection_update(
        &self,
        id: String,
        input: McpConnectionUpdateRequest,
    ) -> SdkResult<ToolInvokeOutput> {
        let store = self.writable_connection_store(&id).await?;
        let cloned = store.clone();
        let target = id.clone();
        let prepared = input.clone();
        let (scope, name, config) = io(move || cloned.prepare_update(&target, &prepared)).await?;
        self.authorize_saved_connection(&store, scope, &name, &config)
            .await?;
        let cloned = store.clone();
        let target = id.clone();
        io(move || cloned.update(&target, input)).await?;
        self.connection_saved(store, Some(id)).await
    }
    pub(super) async fn connection_delete(
        &self,
        id: String,
        revision: String,
    ) -> SdkResult<ToolInvokeOutput> {
        let store = self.writable_connection_store(&id).await?;
        let cloned = store.clone();
        let target = id.clone();
        let (scope, name, _) = io(move || cloned.raw(&target)).await?;
        if scope == McpConnectionScope::Workspace {
            let cloned = store.clone();
            let fallback_id = agena_runtime_config::mcp_connections::connection_id(
                McpConnectionScope::Global,
                &name,
            );
            let fallback = io(move || Ok(cloned.raw(&fallback_id).ok())).await?;
            if let Some((_, name, raw)) = fallback
                && let Ok(config) = validate_connection(&name, raw)
                && config.enabled()
            {
                self.authorize_connection(&store, &name, &config).await?;
            }
        }
        let cloned = store.clone();
        io(move || cloned.remove(&id, &revision)).await?;
        self.connection_saved(store, None).await
    }
    pub(super) async fn connection_set_enabled(
        &self,
        id: String,
        input: McpConnectionEnabledRequest,
    ) -> SdkResult<ToolInvokeOutput> {
        let store = self.writable_connection_store(&id).await?;
        let cloned = store.clone();
        let target = id.clone();
        let (scope, name, raw) = io(move || cloned.raw(&target)).await?;
        let mut config = validate_connection(&name, raw).map_err(store_error)?;
        config.set_enabled(input.enabled);
        self.authorize_saved_connection(&store, scope, &name, &config)
            .await?;
        let cloned = store.clone();
        let target = id.clone();
        io(move || cloned.set_enabled(&target, input)).await?;
        self.connection_saved(store, Some(id)).await
    }
    pub(super) async fn connection_import_preview(
        &self,
        input: McpImportPreviewRequest,
    ) -> SdkResult<ToolInvokeOutput> {
        let store = self.connection_store(None).await?;
        output(
            "MCP import preview",
            io(move || store.preview_import(&input)).await?,
        )
    }
    pub(super) async fn connection_import(
        &self,
        input: McpImportRequest,
    ) -> SdkResult<ToolInvokeOutput> {
        let store = self.connection_store(Some(input.scope)).await?;
        let cloned = store.clone();
        let prepared = input.clone();
        let connections = io(move || cloned.prepare_import(&prepared)).await?;
        for (name, config) in connections {
            self.authorize_saved_connection(&store, input.scope, &name, &config)
                .await?;
        }
        let cloned = store.clone();
        let ids = io(move || cloned.import(input)).await?;
        let (reload_task_id, warning) = match self.connection_host()?.request_config_reload().await
        {
            Ok(task) => (
                Some(task.task_id),
                Some("Import saved. Runtime reload is queued after this tool call.".into()),
            ),
            Err(_) => (
                None,
                Some("Import saved, but runtime reload could not be queued.".into()),
            ),
        };
        let catalog = io(move || store.list()).await?;
        output(
            "MCP connections imported",
            McpImportResult {
                installed: catalog
                    .connections
                    .into_iter()
                    .filter(|c| ids.contains(&c.id))
                    .collect(),
                warning,
                reload_task_id,
            },
        )
    }
    async fn configured_connection(
        &self,
        id: &str,
        active: bool,
    ) -> SdkResult<(McpConnectionStore, String, McpServerConfig)> {
        let store = self.connection_store(None).await?;
        let cloned = store.clone();
        let target = id.to_owned();
        let (name, config) = io(move || {
            if active {
                let detail = cloned.get(&target)?;
                if !detail.summary.enabled
                    || !detail.summary.effective
                    || matches!(
                        detail.summary.state.as_str(),
                        "pending_reload" | "bridge_disabled"
                    )
                {
                    return Err(McpManagementError::invalid(
                        "Enable and reload the effective connection before using it.",
                    ));
                }
            }
            let (_, name, raw) = cloned.raw(&target)?;
            Ok((name.clone(), validate_connection(&name, raw)?))
        })
        .await?;
        Ok((store, name, config))
    }
    pub(super) async fn connection_test(&self, id: &str) -> SdkResult<ToolInvokeOutput> {
        let (store, name, config) = self.configured_connection(id, false).await?;
        let spec = self.authorize_connection(&store, &name, &config).await?;
        let cloned = store.clone();
        let token_store_enabled = io(move || cloned.token_store_enabled()).await?;
        let timeout = config.timeouts();
        output(
            "MCP connection test",
            management::test_connection(
                &name,
                spec,
                store.workspace_root,
                timeout.connect_ms,
                timeout.request_ms,
                token_store_enabled,
            )
            .await,
        )
    }
    pub(super) async fn connection_reconnect(&self, id: &str) -> SdkResult<ToolInvokeOutput> {
        let (store, name, config) = self.configured_connection(id, true).await?;
        self.authorize_connection(&store, &name, &config).await?;
        self.manager
            .reconnect(&name)
            .await
            .map_err(transport_error)?;
        self.connection_get(id).await
    }
    pub(super) async fn connection_credential(
        &self,
        id: &str,
        token: Option<String>,
    ) -> SdkResult<ToolInvokeOutput> {
        let (store, name, config) = self.configured_connection(id, false).await?;
        let McpServerConfig::Http {
            endpoint,
            auth: Some(McpHttpAuthConfig::BearerFromStore),
            ..
        } = &config
        else {
            return Err(PluginError::invalid_params(
                "Select stored bearer authentication before editing the credential.",
            ));
        };
        if token.as_ref().is_some_and(|t| t.trim().is_empty()) {
            return Err(PluginError::invalid_params(
                "Bearer token must not be empty.",
            ));
        }
        self.authorize_connection(&store, &name, &config).await?;
        let endpoint = url::Url::parse(&endpoint.url)
            .map_err(|_| PluginError::invalid_params("Invalid HTTP endpoint."))?;
        let key = agena_mcp_client::credential_store_name(&name, &endpoint);
        io(move || {
            let store = FileTokenStore::new();
            match token {
                Some(token) => store.put_bearer(&key, &token),
                None => store.delete(&key),
            }
            .map_err(|_| McpManagementError::internal("Could not update the stored credential."))
        })
        .await?;
        let detail = self.connection_detail(id).await?;
        if detail.summary.effective
            && detail.summary.enabled
            && !matches!(
                detail.summary.state.as_str(),
                "pending_reload" | "bridge_disabled" | "invalid"
            )
        {
            let _ = self.manager.reconnect(&name).await;
        }
        self.connection_get(id).await
    }
    pub(super) async fn connection_auth_start(
        &self,
        input: &ConnectionAuthStartInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let store = self.connection_store(None).await?;
        let cloned = store.clone();
        let id = input.id.clone();
        let (detail, raw) = io(move || cloned.snapshot(&id)).await?;
        let name = detail.summary.name.clone();
        let config = validate_connection(&name, raw).map_err(store_error)?;
        self.authorize_connection(&store, &name, &config).await?;
        let McpServerConfig::Http {
            endpoint,
            auth:
                Some(McpHttpAuthConfig::OAuth {
                    scopes,
                    client_id,
                    client_secret,
                    client_metadata_url,
                }),
            ..
        } = config
        else {
            return Err(PluginError::invalid_params(
                "Select OAuth authentication before signing in.",
            ));
        };
        let endpoint = url::Url::parse(&endpoint.url)
            .map_err(|_| PluginError::invalid_params("Invalid HTTP endpoint."))?;
        let binding = format!("{}:{}", detail.summary.id, detail.summary.revision);
        let authority = self.connection_oauth_authority()?;
        let result = self
            .manager
            .oauth_sessions()
            .begin_authorized(
                &name,
                endpoint,
                scopes,
                &input.redirect_uri,
                binding,
                agena_mcp_client::McpOAuthRegistration {
                    client_id,
                    client_secret,
                    client_metadata_url,
                },
                authority.clone(),
            )
            .await;
        output("MCP sign-in URL", authority.finish(result)?)
    }
    pub(super) async fn connection_auth_complete(
        &self,
        input: &ConnectionAuthCompleteInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let (store, name, config) = self.configured_connection(&input.id, false).await?;
        self.authorize_connection(&store, &name, &config).await?;
        let detail = self.connection_detail(&input.id).await?;
        let authority = self.connection_oauth_authority()?;
        let result = self
            .manager
            .oauth_sessions()
            .complete_authorized(
                &input.flow_id,
                &input.callback_url,
                &format!("{}:{}", detail.summary.id, detail.summary.revision),
                authority.clone(),
            )
            .await;
        authority.finish(result)?;
        if detail.summary.effective
            && detail.summary.enabled
            && !matches!(
                detail.summary.state.as_str(),
                "pending_reload" | "bridge_disabled" | "invalid"
            )
        {
            let _ = self.manager.reconnect(&name).await;
        }
        self.connection_get(&input.id).await
    }
    pub(super) async fn connection_auth_clear(
        &self,
        id: &str,
        flow: Option<&str>,
    ) -> SdkResult<ToolInvokeOutput> {
        let (store, name, config) = self.configured_connection(id, false).await?;
        let detail = self.connection_detail(id).await?;
        if let Some(flow) = flow {
            self.manager
                .oauth_sessions()
                .cancel_checked(
                    flow,
                    &format!("{}:{}", detail.summary.id, detail.summary.revision),
                )
                .await
                .map_err(transport_error)?;
        } else {
            self.authorize_connection(&store, &name, &config).await?;
            self.connection_secret_access().await?;
            let McpServerConfig::Http { endpoint, .. } = config else {
                return Err(PluginError::invalid_params(
                    "OAuth is available for HTTP connections.",
                ));
            };
            let endpoint = url::Url::parse(&endpoint.url)
                .map_err(|_| PluginError::invalid_params("Invalid HTTP endpoint."))?;
            self.manager
                .oauth_sessions()
                .logout(&name, &endpoint)
                .await
                .map_err(transport_error)?;
            if detail.summary.effective
                && detail.summary.enabled
                && !matches!(
                    detail.summary.state.as_str(),
                    "pending_reload" | "bridge_disabled" | "invalid"
                )
            {
                let _ = self.manager.reconnect(&name).await;
            }
        }
        self.connection_get(id).await
    }
    pub(super) async fn connection_bridge(
        &self,
        input: McpBridgeEnabledRequest,
    ) -> SdkResult<ToolInvokeOutput> {
        let store = self.connection_store(Some(input.scope)).await?;
        if input.enabled {
            let cloned = store.clone();
            let catalog = io(move || cloned.list()).await?;
            for entry in catalog
                .connections
                .into_iter()
                .filter(|c| c.enabled && c.effective)
            {
                let (store, name, config) = self.configured_connection(&entry.id, false).await?;
                self.authorize_connection(&store, &name, &config).await?;
            }
        }
        let cloned = store.clone();
        io(move || cloned.set_bridge_enabled(input)).await?;
        self.connection_saved(store, None).await
    }
}

fn connection_spec(config: &McpServerConfig, root: &Path) -> SdkResult<ServerSpec> {
    Ok(match config {
        McpServerConfig::Stdio { process, tools, .. } => ServerSpec::Stdio {
            command: process.command.clone(),
            args: process.args.clone(),
            env: process
                .env
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            cwd: Some(match &process.cwd {
                Some(cwd) if cwd.is_absolute() => cwd.clone(),
                Some(cwd) => root.join(cwd),
                None => root.into(),
            }),
            tool_policy: McpToolPolicy {
                include: tools.include.clone(),
                exclude: tools.exclude.clone(),
            },
        },
        McpServerConfig::Http {
            endpoint,
            auth,
            tools,
            ..
        } => ServerSpec::Http {
            url: url::Url::parse(&endpoint.url)
                .map_err(|_| PluginError::invalid_params("Invalid HTTP endpoint."))?,
            headers: endpoint
                .headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            auth: auth.as_ref().map(|auth| match auth {
                McpHttpAuthConfig::Bearer { token } => HttpAuth::Bearer(token.clone()),
                McpHttpAuthConfig::BearerFromEnv { env } => HttpAuth::BearerFromEnv(env.clone()),
                McpHttpAuthConfig::BearerFromStore => HttpAuth::BearerFromStore,
                McpHttpAuthConfig::OAuth {
                    scopes,
                    client_id,
                    client_secret,
                    client_metadata_url,
                } => HttpAuth::OAuth {
                    scopes: scopes.clone(),
                    registration: agena_mcp_client::McpOAuthRegistration {
                        client_id: client_id.clone(),
                        client_secret: client_secret.clone(),
                        client_metadata_url: client_metadata_url.clone(),
                    },
                },
                McpHttpAuthConfig::Custom { headers } => HttpAuth::Custom(
                    headers
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                ),
            }),
            tool_policy: McpToolPolicy {
                include: tools.include.clone(),
                exclude: tools.exclude.clone(),
            },
        },
    })
}
