use super::*;
use agena_domain::mcp_app_tools::McpAppTaskInput;
use agena_mcp_client::apps::{
    APP_MESSAGE_LIMIT, APP_MIME_TYPE, McpAppBinding, app_resource, app_resource_uri,
    has_app_mime_type, is_app_uri, tool_visible_to,
};
use agena_mcp_client::protocol::{ListResourcesResult, ToolDescriptor};
use agena_mcp_client::manager::{McpTaskObserver, McpTaskViewInputs};
use agena_mcp_client::{McpError, McpHttpTransport, ServerSpec};
use agena_plugin_host::sdk::{
    PluginErrorKind,
    host_api::{McpAppCatalogSnapshot, McpAppCatalogWatch, McpAppServerTaskPort, current_host_callback_context},
};

static APP_CATALOGS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);
const RESOURCE_CATALOG_PAGES: usize = 16;
const RESOURCE_CATALOG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

struct AppTaskInputs<'a>(&'a McpAppServerTaskPort);

#[async_trait::async_trait]
impl McpTaskViewInputs for AppTaskInputs<'_> {
    fn clear_inputs(&self) {
        self.0.clear_view_inputs();
    }

    async fn wait_for_inputs(
        &self,
        requests: BTreeMap<String, McpAppTaskInput>,
        cancellation: &CancellationToken,
    ) -> Option<BTreeMap<String, Value>> {
        self.0.wait_for_view_inputs(requests, cancellation).await
    }
}

pub(super) struct McpAppConnection {
    binding: McpAppBinding,
    manager: Arc<McpConnectionManager>,
    tools: Arc<Vec<ToolDescriptor>>,
    _retained: Option<Arc<McpConnectionManager>>,
    catalog_only: bool,
    resource_catalog: Option<McpAppCatalogWatch>,
    streamable: bool,
    streamable_authority: Option<Arc<ConnectionNetworkAuthority>>,
}

impl McpAppConnection {
    pub(super) async fn list_resources(
        self,
        cursor: Option<String>,
    ) -> SdkResult<ToolInvokeOutput> {
        if let Some(watch) = &self.resource_catalog {
            self.publish_resource_catalog(watch).await?;
            return bounded_app_output(serde_json::json!({}), APP_MESSAGE_LIMIT);
        }
        bounded_app_output(self.visible_resources(cursor).await?, APP_MESSAGE_LIMIT)
    }

    async fn visible_resources(&self, cursor: Option<String>) -> SdkResult<ListResourcesResult> {
        let mut result = self
            .manager
            .list_resources(&self.binding.server, cursor)
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        result.resources.retain(|resource| {
            resource.uri == self.binding.uri
                || !is_app_content(&resource.uri, resource.mime_type.as_deref())
        });
        Ok(result)
    }

    async fn publish_resource_catalog(&self, watch: &McpAppCatalogWatch) -> SdkResult<()> {
        let server = &self.binding.server;
        if !self.streamable {
            if let Some((peer, mut changes, closed)) = self
                .manager
                .observe_resource_list_changes(server)
                .await
                .map_err(|error| PluginError::internal_error(&error))?
            {
                let generation = *changes.borrow_and_update();
                let mut digest = Sha256::new();
                digest.update(peer.as_bytes());
                digest.update(generation.to_le_bytes());
                watch.publish(McpAppCatalogSnapshot {
                    revision: hex::encode(digest.finalize()),
                    changes: Some((changes, closed)),
                    waited: false,
                });
            }
            return Ok(());
        }
        let Some((generation, notify)) = self
            .manager
            .app_resource_catalog_generation(server)
            .await
            .map_err(|error| PluginError::internal_error(&error))?
        else {
            return Ok(());
        };
        let fingerprint =
            tokio::time::timeout(RESOURCE_CATALOG_TIMEOUT, self.resource_fingerprint())
                .await
                .unwrap_or(Ok(None))?;
        if fingerprint.is_none() && (!notify || self.streamable_authority.is_none()) {
            return Ok(());
        }
        let mut digest = Sha256::new();
        digest.update(
            self.streamable_authority
                .as_ref()
                .map_or(0, |_| generation)
                .to_le_bytes(),
        );
        if let Some(fingerprint) = fingerprint {
            digest.update(fingerprint);
        }
        let revision = hex::encode(digest.finalize());
        let waited = if let Some(authority) = &self.streamable_authority
            && let Some((after, timeout)) = watch.requested_wait()
            && revision == after
        {
            authority.finish(
                self.manager
                    .wait_for_app_http_resource_change(
                        server,
                        generation,
                        timeout,
                        authority.as_ref(),
                    )
                    .await,
            )?
        } else {
            false
        };
        watch.publish(McpAppCatalogSnapshot {
            revision,
            changes: None,
            waited,
        });
        Ok(())
    }

    async fn resource_fingerprint(&self) -> SdkResult<Option<[u8; 32]>> {
        let mut cursor = None;
        let mut seen = std::collections::BTreeSet::new();
        let mut entries: Vec<[u8; 32]> = Vec::new();
        let mut remaining = RESOURCE_CATALOG_PAGES * APP_MESSAGE_LIMIT;
        for _ in 0..RESOURCE_CATALOG_PAGES {
            let page = self.visible_resources(cursor).await?;
            for resource in page.resources {
                let encoded = serde_json::to_vec(&resource)
                    .map_err(|error| PluginError::internal_error(&error))?;
                let Some(budget) = remaining.checked_sub(encoded.len()) else {
                    return Ok(None);
                };
                remaining = budget;
                entries.push(Sha256::digest(encoded).into());
            }
            let Some(next) = page.next_cursor else {
                entries.sort_unstable();
                let mut digest = Sha256::new();
                for entry in entries {
                    digest.update(entry);
                }
                return Ok(Some(digest.finalize().into()));
            };
            if !seen.insert(next.clone()) {
                break;
            }
            cursor = Some(next);
        }
        Ok(None)
    }

    pub(super) async fn list_resource_templates(
        self,
        cursor: Option<String>,
    ) -> SdkResult<ToolInvokeOutput> {
        let mut result = self
            .manager
            .list_resource_templates(&self.binding.server, cursor)
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        result.resource_templates.retain(|template| {
            !template.uri_template.starts_with("ui://")
                && !template.mime_type.as_deref().is_some_and(has_app_mime_type)
        });
        bounded_app_output(result, APP_MESSAGE_LIMIT)
    }
}

impl McpPlugin {
    fn app_interactions(&self) -> Option<&dyn McpElicitationHandler> {
        current_host_callback_context()
            .and_then(|context| context.mcp_app)
            .is_some_and(|binding| binding.view.is_some())
            .then_some(self as &dyn McpElicitationHandler)
    }

    pub(super) async fn app_connection(
        &self,
        context: &ToolInvokeContext<'_>,
        requested_server: Option<&str>,
    ) -> SdkResult<Option<McpAppConnection>> {
        let Some(app) = current_host_callback_context().and_then(|context| context.mcp_app) else {
            return Ok(None);
        };
        let binding: McpAppBinding =
            serde_json::from_str(&app.reference).map_err(|_| unavailable_app())?;
        let server = binding.server.as_str();
        if requested_server.is_some_and(|name| name != server) {
            return Err(unavailable_app());
        }
        let workspace = std::path::Path::new(context.workspace_root);
        let spec = self
            .manager
            .server_spec_in_workspace(server, workspace)
            .await
            .map_err(|_| unavailable_app())?;
        if !binding
            .verifies(context.session_id, app.source_call_id, &spec)
            .await
        {
            return Err(unavailable_app());
        }
        let mut retained = None;
        let mut streamable_authority = None;
        let manager = match &spec {
            ServerSpec::Stdio { .. } => {
                self.borrow_stdio_app(
                    context.session_id,
                    server,
                    workspace,
                    binding.stdio_peer_id.ok_or_else(unavailable_app)?,
                    spec.clone(),
                )
                .await?
            }
            ServerSpec::Http { url, .. } => {
                retained = self
                    .manager
                    .app_http_connection(context.session_id, app.source_call_id, server, workspace)
                    .await;
                if retained.is_none() && !binding.requires_original_peer {
                    retained = self
                        .reopen_stateless_app_http(context, app.source_call_id, server, workspace)
                        .await?;
                }
                if let Some(connection) = &retained {
                    let network = self
                        .host
                        .get()
                        .ok_or_else(unavailable_app)?
                        .require_network_permission(network_permission_target(url.as_str())?)
                        .await;
                    if let Err(error) = network {
                        if error.kind == PluginErrorKind::PolicyDenied {
                            self.manager
                                .revoke_app_http_connection(
                                    context.session_id,
                                    app.source_call_id,
                                    connection,
                                )
                                .await;
                        }
                        return Err(error);
                    }
                    let authority = self.connection_network_authority()?;
                    let manager = authority.finish(
                        connection
                            .borrow_http(server, authority.clone(), false)
                            .await,
                    )?;
                    self.refresh_http_roots(&manager, &authority).await?;
                    if matches!(
                        &spec,
                        ServerSpec::Http {
                            transport: McpHttpTransport::Streamable,
                            ..
                        }
                    ) {
                        streamable_authority = Some(authority);
                    }
                    manager
                } else {
                    if binding.requires_original_peer {
                        return Err(unavailable_app());
                    }
                    let manager = self.authorize_server(server).await?;
                    let connected = manager
                        .server_spec(server)
                        .await
                        .map_err(|_| unavailable_app())?;
                    if !binding
                        .verifies(context.session_id, app.source_call_id, &connected)
                        .await
                    {
                        return Err(unavailable_app());
                    }
                    manager
                }
            }
        };
        let catalogs = app.catalogs;
        let streamable = matches!(
            spec,
            ServerSpec::Http {
                transport: McpHttpTransport::Streamable,
                ..
            },
        );
        let tool_changes = if catalogs.tools.is_some() && !streamable {
            manager
                .observe_tool_list_changes(server)
                .await
                .map_err(|error| PluginError::internal_error(&error))?
        } else {
            None
        };
        let tools =
            if retained.is_some() || binding.stdio_peer_id.is_some() || catalogs.tools.is_some() {
                let policy = match &spec {
                    ServerSpec::Http { tool_policy, .. }
                    | ServerSpec::Stdio { tool_policy, .. } => tool_policy,
                };
                Arc::new(
                    manager
                        .refresh_tools(server)
                        .await
                        .map_err(|error| PluginError::internal_error(&error))?
                        .into_iter()
                        .filter(|tool| policy.permits(&tool.name))
                        .collect::<Vec<_>>(),
                )
            } else {
                manager
                    .connected_tools(server)
                    .await
                    .map_err(|error| PluginError::internal_error(&error))?
            };
        if !tools.iter().any(|tool| {
            tool.name == binding.tool
                && tool_visible_to(tool, "model")
                && app_resource_uri(tool) == Some(binding.uri.as_str())
        }) {
            return Err(unavailable_app());
        }
        let catalog_only = catalogs.tools.is_some() || catalogs.resources.is_some();
        if let Some(watch) = catalogs.tools {
            let snapshot_tools = tools.clone();
            let revision = APP_CATALOGS
                .run(move || app_tool_catalog_revision(&snapshot_tools))
                .await
                .map_err(|error| PluginError::internal_error(&error))?
                .map_err(|error| PluginError::internal_error(&error))?;
            watch.publish(McpAppCatalogSnapshot {
                revision,
                changes: tool_changes,
                waited: false,
            });
        }
        Ok(Some(McpAppConnection {
            binding,
            manager,
            tools,
            _retained: retained,
            catalog_only,
            resource_catalog: catalogs.resources,
            streamable,
            streamable_authority,
        }))
    }

    async fn reopen_stateless_app_http(
        &self,
        context: &ToolInvokeContext<'_>,
        source_call: i64,
        server: &str,
        workspace: &std::path::Path,
    ) -> SdkResult<Option<Arc<McpConnectionManager>>> {
        let setup = self.open_app_http(context, server).await?;
        if setup.manager.app_requires_original_peer(server).await {
            setup.manager.shutdown_all().await;
            return Err(unavailable_app());
        }
        let retained = self
            .manager
            .retain_app_http_connection(
                context.session_id,
                source_call,
                server,
                workspace,
                setup.manager.clone(),
            )
            .await;
        if retained
            .as_ref()
            .is_none_or(|peer| !Arc::ptr_eq(peer, &setup.manager))
        {
            setup.manager.shutdown_all().await;
        }
        Ok(retained)
    }

    async fn borrow_stdio_app(
        &self,
        session: i64,
        server: &str,
        workspace: &std::path::Path,
        peer_id: uuid::Uuid,
        spec: ServerSpec,
    ) -> SdkResult<Arc<McpConnectionManager>> {
        let live = self
            .manager
            .app_stdio_spec(session, server, workspace, peer_id)
            .await
            .map_err(|_| unavailable_app())?;
        if live != spec {
            return Err(unavailable_app());
        }
        let policy_changed = || {
            PluginError::from_kind(
                PluginErrorKind::PolicyDenied,
                "The MCP App's original process policy has changed.",
            )
        };
        let result = async {
            let prepared = self
                .prepare_stdio_sandbox(&spec)
                .await?
                .ok_or_else(unavailable_app)?;
            if prepared.call_bound {
                return Err(policy_changed());
            }
            let context = super::scoped::stdio_sandbox_context(prepared)?;
            self.manager
                .borrow_app_stdio(session, server, workspace, peer_id, spec, &context)
                .await
                .map_err(|error| {
                    if matches!(error, McpError::PermissionDenied) {
                        policy_changed()
                    } else {
                        unavailable_app()
                    }
                })
        }
        .await;
        if result.as_ref().is_err_and(|error| {
            matches!(
                error.kind,
                PluginErrorKind::PolicyDenied | PluginErrorKind::UserDeclined
            )
        }) {
            self.manager
                .revoke_app_stdio(session, server, workspace, peer_id)
                .await;
        }
        result
    }

    pub(super) async fn model_tool_descriptor(
        &self,
        manager: &McpConnectionManager,
        server: &str,
        name: &str,
    ) -> SdkResult<ToolDescriptor> {
        let tools = manager
            .connected_tools(server)
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        if let Some(tool) = tools.iter().find(|tool| tool.name == name) {
            return Ok(tool.clone());
        }
        manager
            .refresh_tools(server)
            .await
            .map_err(|error| PluginError::internal_error(&error))?
            .into_iter()
            .find(|tool| tool.name == name)
            .ok_or_else(|| PluginError::invalid_params("The MCP tool is unavailable."))
    }

    pub(super) async fn read_app_resource(
        &self,
        input: &ReadResourceInput,
        connection: McpAppConnection,
    ) -> SdkResult<ToolInvokeOutput> {
        let McpAppConnection {
            manager, binding, ..
        } = connection;
        if is_app_uri(&input.uri) && input.uri != binding.uri {
            return Err(unavailable_app());
        }
        let result = manager
            .read_resource_with_elicitation(&input.server, &input.uri, self.app_interactions())
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        if input.uri == binding.uri {
            let resource = app_resource(&binding.uri, result).ok_or_else(unavailable_app)?;
            let ui = match resource.ui {
                Some(ui) => ui,
                None => tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    listed_app_metadata(&manager, &input.server, &binding.uri),
                )
                .await
                .ok()
                .flatten()
                .unwrap_or_default(),
            };
            let ui = self.app_browser_policy(ui).await;
            return bounded_app_output(
                serde_json::json!({"contents":[{
                    "uri": binding.uri, "mimeType": APP_MIME_TYPE, "text": resource.html, "_meta": { "ui": ui },
                }]}),
                8 * APP_MESSAGE_LIMIT,
            );
        }
        let mut result = result;
        result
            .contents
            .retain(|item| !is_app_content(&item.uri, item.mime_type.as_deref()));
        bounded_app_output(result, APP_MESSAGE_LIMIT)
    }

    pub(super) async fn call_app_tool(
        &self,
        input: &CallToolInput,
        connection: McpAppConnection,
        task: Option<&McpAppServerTaskPort>,
    ) -> SdkResult<ToolInvokeOutput> {
        let tool = connection
            .tools
            .iter()
            .find(|tool| tool.name == input.name && tool_visible_to(tool, "app"))
            .ok_or_else(|| {
                PluginError::invalid_params("The MCP tool is unavailable to this app.")
            })?;
        super::tool_loading::validate_tool_arguments(tool, input.arguments.as_ref()).await?;
        let arguments = empty_object_to_none(input.arguments.clone());
        let result = if let Some(task) = task {
            let view = AppTaskInputs(task);
            let progress = |info| {
                if let Ok(info) = serde_json::to_value(info) {
                    task.observe(info);
                }
            };
            let observer = McpTaskObserver {
                legacy_ttl_ms: task.legacy_ttl_ms(),
                cancellation: task.cancellation(),
                progress: &progress,
                view: Some(&view),
            };
            connection.manager.call_tool_observed(&input.server, &input.name, arguments, self, &observer).await
        } else {
            connection.manager.call_tool_with_elicitation(
                &input.server, &input.name, arguments, self.app_interactions(),
            ).await
        }.map_err(|error| PluginError::internal_error(&error))?;
        bounded_app_output(result, APP_MESSAGE_LIMIT)
    }

    pub(super) fn list_app_tools(
        offset: u32,
        connection: McpAppConnection,
    ) -> SdkResult<ToolInvokeOutput> {
        if connection.catalog_only {
            return bounded_app_output(serde_json::json!({}), APP_MESSAGE_LIMIT);
        }
        let visible = app_visible_tools(&connection.tools);
        let offset = offset as usize;
        let remaining = visible
            .get(offset..)
            .ok_or_else(|| PluginError::invalid_params("The MCP App cursor is out of range."))?;
        let mut page = Vec::new();
        // Leave room for the result envelope and cursor.
        let mut budget = APP_MESSAGE_LIMIT - 128;
        for tool in remaining.iter().take(50) {
            let descriptor =
                serde_json::to_value(tool).map_err(|error| PluginError::internal_error(&error))?;
            let size = descriptor.to_string().len() + 1;
            if size > budget {
                if page.is_empty() {
                    return Err(PluginError::invalid_params(
                        "The MCP App tool descriptor exceeds its size limit.",
                    ));
                }
                break;
            }
            budget -= size;
            page.push(descriptor);
        }
        let next = offset + page.len();
        let mut result = serde_json::json!({"tools": page});
        if next < visible.len() {
            result["nextCursor"] = Value::String(next.to_string());
        }
        bounded_app_output(result, APP_MESSAGE_LIMIT)
    }

    async fn app_browser_policy(&self, ui: Value) -> Value {
        // Browser origins are explicit exact origins or strict TLS subdomain
        // wildcards (e.g. `https://*.example.com`). The Web client asks before
        // applying origins or device permissions already permitted by the session.
        let Some(host) = self.host.get() else {
            return Value::Null;
        };
        let mut csp = serde_json::Map::new();
        let mut decisions = BTreeMap::new();
        for field in [
            "connectDomains",
            "resourceDomains",
            "baseUriDomains",
            "frameDomains",
        ] {
            let declared = ui
                .get("csp")
                .and_then(|csp| csp.get(field))
                .and_then(Value::as_array);
            let mut origins = Vec::new();
            for origin in declared
                .into_iter()
                .flatten()
                .take(12)
                .filter_map(Value::as_str)
            {
                let Some((origin, targets)) =
                    browser_csp_origin(origin, field == "connectDomains")
                else {
                    continue;
                };
                let mut allowed = true;
                for target in targets {
                    let permitted = if let Some(permitted) = decisions.get(&target) {
                        *permitted
                    } else {
                        let permitted = host
                            .require_network_permission(target.clone())
                            .await
                            .is_ok();
                        decisions.insert(target, permitted);
                        permitted
                    };
                    if !permitted {
                        allowed = false;
                        break;
                    }
                }
                if allowed && !origins.contains(&origin) {
                    origins.push(origin);
                }
            }
            if !origins.is_empty() {
                csp.insert(field.to_owned(), serde_json::json!(origins));
            }
        }
        let mut policy = serde_json::json!({"csp": csp});
        if let Some(border) = ui.get("prefersBorder").and_then(Value::as_bool) {
            policy["prefersBorder"] = Value::Bool(border);
        }
        if let Some(permissions) = browser_device_permissions(ui.get("permissions")) {
            policy["permissions"] = Value::Object(permissions);
        }
        policy
    }
}

fn app_visible_tools(tools: &[ToolDescriptor]) -> Vec<&ToolDescriptor> {
    let mut visible = tools
        .iter()
        .filter(|tool| tool_visible_to(tool, "app"))
        .collect::<Vec<_>>();
    visible.sort_unstable_by(|left, right| left.name.cmp(&right.name));
    visible
}

fn app_tool_catalog_revision(tools: &[ToolDescriptor]) -> serde_json::Result<String> {
    let mut digest = Sha256::new();
    for tool in app_visible_tools(tools) {
        let encoded = serde_json::to_vec(tool)?;
        digest.update((encoded.len() as u64).to_le_bytes());
        digest.update(encoded);
    }
    Ok(hex::encode(digest.finalize()))
}

async fn listed_app_metadata(
    manager: &McpConnectionManager,
    server: &str,
    uri: &str,
) -> Option<Value> {
    let mut cursor = None;
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..RESOURCE_CATALOG_PAGES {
        let page = manager.list_resources(server, cursor).await.ok()?;
        if let Some(entry) = page.resources.into_iter().find(|entry| entry.uri == uri) {
            return entry.meta?.get("ui").cloned();
        }
        let next = page.next_cursor?;
        if !seen.insert(next.clone()) {
            return None;
        }
        cursor = Some(next);
    }
    None
}

fn browser_device_permissions(value: Option<&Value>) -> Option<serde_json::Map<String, Value>> {
    let declared = value?.as_object()?;
    let mut accepted = serde_json::Map::new();
    for key in ["camera", "microphone", "geolocation", "clipboardWrite"] {
        let requested = match declared.get(key) {
            Some(Value::Object(_)) | Some(Value::Bool(true)) => true,
            _ => false,
        };
        if requested {
            accepted.insert(key.to_owned(), serde_json::json!({}));
        }
    }
    (!accepted.is_empty()).then_some(accepted)
}

fn valid_wildcard_base_domain(domain: &str) -> bool {
    if domain.is_empty()
        || domain.len() > 253
        || domain.parse::<std::net::IpAddr>().is_ok()
        || domain.eq_ignore_ascii_case("localhost")
        || domain.ends_with(".localhost")
        || domain.ends_with(".local")
        || domain.ends_with(".internal")
    {
        return false;
    }
    let labels: Vec<&str> = domain.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    labels.iter().all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })
}

fn browser_csp_origin(value: &str, websocket: bool) -> Option<(String, Vec<String>)> {
    if value.len() > 256
        || !value
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || b":/.[-]_*" .contains(&ch))
    {
        return None;
    }
    if let Some((scheme, rest)) = value.split_once("://*.") {
        let scheme = scheme.to_ascii_lowercase();
        let allowed_scheme = scheme == "https" || (websocket && scheme == "wss");
        if !allowed_scheme {
            return None;
        }
        let authority = rest.strip_suffix('/').unwrap_or(rest);
        if authority.contains('/')
            || authority.contains('?')
            || authority.contains('#')
            || authority.contains('@')
            || authority.contains('*')
        {
            return None;
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => {
                let port: u16 = p.parse().ok()?;
                if port == 0 {
                    return None;
                }
                (h, Some(port))
            }
            None => (authority, None),
        };
        let base = host.to_ascii_lowercase();
        if !valid_wildcard_base_domain(&base) {
            return None;
        }
        let effective_port = port.unwrap_or(443);
        let serialized = match port {
            Some(p) if p != 443 => format!("{scheme}://*.{base}:{p}"),
            _ => format!("{scheme}://*.{base}"),
        };
        // Wildcard subdomain CSP allows connecting to arbitrary public subdomains;
        // require the live session policy to permit unrestricted outbound HTTPS on
        // the target port without issuing untrusted DNS queries.
        let targets = vec![
            format!("https://198.51.100.1:{effective_port}"),
            format!("https://203.0.113.1:{effective_port}"),
        ];
        return Some((serialized, targets));
    }
    if value.contains('*') {
        return None;
    }
    let mut url = url::Url::parse(value).ok()?;
    if url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    let origin = url.origin().ascii_serialization();
    match url.scheme() {
        "http" | "https" => {}
        "ws" if websocket => {
            url.set_scheme("http").ok()?;
        }
        "wss" if websocket => {
            url.set_scheme("https").ok()?;
        }
        _ => return None,
    }
    Some((origin, vec![network_permission_target(url.as_str()).ok()?]))
}

fn unavailable_app() -> PluginError {
    PluginError::invalid_params("The original MCP App connection or resource is unavailable.")
}

fn bounded_app_output(value: impl Serialize, limit: usize) -> SdkResult<ToolInvokeOutput> {
    let payload =
        serde_json::to_value(value).map_err(|error| PluginError::internal_error(&error))?;
    if payload.to_string().len() > limit {
        return Err(PluginError::invalid_params(
            "The MCP App response exceeds its size limit.",
        ));
    }
    Ok(ToolInvokeOutput::from_parts(
        "MCP App",
        "MCP App result",
        "",
        Some(payload),
        Default::default(),
        Vec::new(),
    ))
}

pub(super) fn is_app_content(uri: &str, mime: Option<&str>) -> bool {
    is_app_uri(uri) || mime.is_some_and(has_app_mime_type)
}

pub(super) fn is_app_block(block: &ContentBlock) -> bool {
    match block {
        ContentBlock::Resource { resource, .. } => {
            is_app_content(&resource.uri, resource.mime_type.as_deref())
        }
        ContentBlock::ResourceLink { resource } => {
            is_app_content(&resource.uri, resource.mime_type.as_deref())
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_csp_origin_accepts_exact_and_strict_tls_subdomain_wildcards() {
        let (origin, targets) = browser_csp_origin("https://static.example.com/", false).unwrap();
        assert_eq!(origin, "https://static.example.com");
        assert_eq!(targets, vec!["https://static.example.com"]);

        let (origin, targets) = browser_csp_origin("https://*.cdn.example.com", false).unwrap();
        assert_eq!(origin, "https://*.cdn.example.com");
        assert_eq!(
            targets,
            vec!["https://198.51.100.1:443", "https://203.0.113.1:443"]
        );

        let (origin, targets) =
            browser_csp_origin("https://*.cdn.example.com:8443/", false).unwrap();
        assert_eq!(origin, "https://*.cdn.example.com:8443");
        assert_eq!(
            targets,
            vec!["https://198.51.100.1:8443", "https://203.0.113.1:8443"]
        );

        assert!(browser_csp_origin("wss://*.events.example.com", false).is_none());
        let (origin, _) = browser_csp_origin("wss://*.events.example.com:443", true).unwrap();
        assert_eq!(origin, "wss://*.events.example.com");

        for rejected in [
            "http://*.unsafe.example.com",
            "ws://*.unsafe.example.com",
            "https://*",
            "https://*.com",
            "https://*.localhost",
            "https://*.corp.local",
            "https://*.svc.internal",
            "https://*.192.0.2.1",
            "https://*.*.example.com",
            "https://*.example.com/path",
            "https://user@*.example.com",
            "*; script-src *",
        ] {
            assert!(
                browser_csp_origin(rejected, true).is_none(),
                "expected {rejected:?} to be rejected"
            );
        }
    }

    #[test]
    fn browser_device_permissions_filters_to_supported_capabilities() {
        let declared = serde_json::json!({
            "camera": {},
            "microphone": true,
            "geolocation": false,
            "clipboardWrite": {},
            "usb": {},
        });
        let accepted = browser_device_permissions(Some(&declared)).unwrap();
        assert_eq!(
            Value::Object(accepted),
            serde_json::json!({
                "camera": {},
                "microphone": {},
                "clipboardWrite": {},
            })
        );
        assert!(browser_device_permissions(Some(&serde_json::json!({"usb": {}}))).is_none());
    }
}
