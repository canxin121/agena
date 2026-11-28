use std::{collections::BTreeMap, sync::Arc, time::Duration};

use agena_bundled_plugins::plugins::provided::mcp::{
    ConnectionNetworkAuthority, network_permission_target, prepare_stdio_sandbox,
};
use agena_mcp_client::{
    McpConnectionManager, McpConnectionObservation, McpProbeOptions, McpSandboxContext, ServerSpec,
};
use agena_plugin_host::{
    AgentPluginFormat, LoadedPlugin, PluginError,
    agent_plugin::{DeepSeekManagedMcpServer, agent_plugin_mcp_server_name},
    current_plugin_call_completion,
    sdk::{
        PluginKey, Result,
        host_api::{DeepSeekMcpManagerRequest, DeepSeekMcpProbeOptions},
    },
};
use agena_storage::kernel::{ChangeKind, KernelChange, KernelWatch, ReadView, WatchError};
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use super::{RuntimeHostClient, host_unavailable, plugin_error_from_app};
use crate::tool::ToolExecutor;

mod cleanup;
pub(in crate::runtime) use cleanup::CleanupLedger;
mod observe;
mod reconnect;
mod runtime_calls;

// One metadata update can wake every peer and listener in a project. Leave
// kernel control reads available to the foreground command's projection.
static BACKGROUND_POLICY_CHECKS: Semaphore = Semaphore::const_new(2);

impl RuntimeHostClient {
    pub(super) async fn manage_deepseek_mcp(
        &self,
        request: DeepSeekMcpManagerRequest,
        cancellation: CancellationToken,
    ) -> Result<Value> {
        let context = self.callback_context()?;
        let session = context.session_id.filter(|id| *id > 0).ok_or_else(|| {
            host_unavailable("the DeepSeek MCP manager requires a project session")
        })?;
        let completion = current_plugin_call_completion()
            .ok_or_else(|| host_unavailable("the DeepSeek MCP manager requires a live call"))?;
        let key = self.callback_plugin_key()?;
        let runtime = self.runtime()?;
        let cleanup_ledger = &runtime.deepseek_mcp_cleanup;
        let tool = match &request {
            DeepSeekMcpManagerRequest::List | DeepSeekMcpManagerRequest::Watch { .. } => {
                "agena.mcp.servers.status"
            }
            DeepSeekMcpManagerRequest::Restart { .. } => "agena.mcp.servers.reconnect",
            DeepSeekMcpManagerRequest::Probe { .. } => "agena.mcp.connections.test",
        };
        let (snapshot, sessions, executor) = loop {
            let snapshot = runtime.current_snapshot();
            let sessions = snapshot
                .session_manager()
                .ok_or_else(|| host_unavailable("the DeepSeek MCP manager requires sessions"))?;
            let authorization = sessions
                .authorize_deepseek_mcp_manager(session, &key, tool)
                .await;
            let published = runtime.current_snapshot();
            let executor_changed = authorization.as_ref().is_ok_and(|executor| {
                !Arc::ptr_eq(executor.plugin_manager(), &snapshot.plugin_manager())
            });
            if Arc::ptr_eq(&snapshot, &published) && !executor_changed {
                break (
                    snapshot,
                    sessions,
                    authorization.map_err(plugin_error_from_app)?,
                );
            }
            match &request {
                DeepSeekMcpManagerRequest::List => {
                    // A read-only list can retry across concurrent publication.
                    // The session manager is updated just before the snapshot
                    // swap, so an executor mismatch can precede replacement.
                    tokio::select! {
                        _ = snapshot.replaced.cancelled() => {}
                        _ = cancellation.cancelled() => {
                            return Err(PluginError::from_kind(
                                agena_plugin_host::sdk::PluginErrorKind::Cancelled,
                                "the DeepSeek MCP catalog request was cancelled",
                            ));
                        }
                    }
                }
                DeepSeekMcpManagerRequest::Watch { .. } => return Ok(Value::Null),
                _ => {
                    return Err(host_unavailable(
                        "the DeepSeek MCP manager configuration changed",
                    ));
                }
            }
        };
        // An unchanged plugin can keep its initialized host client across a
        // runtime reload. Pin this operation to the published snapshot while
        // the plugin host independently validates the callback's effect scope.
        let generation = snapshot.generation();
        let plugins = snapshot.plugin_manager();
        let caller = plugins
            .plugins()
            .iter()
            .find(|plugin| plugin.key() == key)
            .ok_or_else(|| host_unavailable("the DeepSeek package is unavailable"))?;
        caller
            .agent_plugin()
            .filter(|package| package.format == AgentPluginFormat::DeepSeekHarness)
            .ok_or_else(|| host_unavailable("this callback belongs to a DeepSeek bundle"))?;
        let mcp = snapshot
            .mcp_manager()
            .ok_or_else(|| host_unavailable("the Agena MCP bridge is disabled"))?;
        let declarations = managed_servers(plugins.plugins());
        let name = match &request {
            DeepSeekMcpManagerRequest::List => {
                let revision = mcp
                    .connection_revision()
                    .saturating_add(cleanup_ledger.revision());
                let views = manager_views(
                    &mcp,
                    &declarations,
                    &executor,
                    session,
                    completion.id(),
                    cleanup_ledger,
                    generation,
                )
                .await;
                return Ok(json!({ "views": views, "revision": revision }));
            }
            DeepSeekMcpManagerRequest::Watch { previous } => {
                let reader = &sessions.kernel().reader;
                let session_id = session.try_into().map_err(PluginError::invalid_params)?;
                let subscribe = || {
                    reader
                        .watch(session_id, None)
                        .map_err(|error| PluginError::internal_error(&error))
                };
                let mut session_changes = subscribe()?;
                let mut changes = mcp.observe_plugin_connections();
                let mut retirements = cleanup_ledger.watch();
                loop {
                    if cancellation.is_cancelled()
                        || runtime.current_snapshot().generation() != generation
                    {
                        return Ok(Value::Null);
                    }
                    let revision = changes
                        .borrow_and_update()
                        .saturating_add(*retirements.borrow_and_update());
                    // Subscribe first so policy updates that race with this
                    // read are observed on the next iteration.
                    let executor = tokio::select! {
                        biased;
                        _ = cancellation.cancelled() => return Ok(Value::Null),
                        _ = snapshot.replaced.cancelled() => return Ok(Value::Null),
                        result = sessions.authorize_deepseek_mcp_manager(session, &key, tool) => {
                            let executor = result.map_err(plugin_error_from_app)?;
                            if !Arc::ptr_eq(executor.plugin_manager(), &plugins) {
                                return Ok(Value::Null);
                            }
                            executor
                        }
                    };
                    let views = manager_views(
                        &mcp,
                        &declarations,
                        &executor,
                        session,
                        completion.id(),
                        cleanup_ledger,
                        generation,
                    )
                    .await;
                    if &views != previous {
                        return Ok(json!({ "views": views, "revision": revision }));
                    }
                    tokio::select! {
                        biased;
                        _ = cancellation.cancelled() => return Ok(Value::Null),
                        _ = snapshot.replaced.cancelled() => return Ok(Value::Null),
                        change = changes.changed() => {
                            if change.is_err() {
                                return Ok(Value::Null);
                            }
                        }
                        change = retirements.changed() => {
                            if change.is_err() {
                                return Ok(Value::Null);
                            }
                        }
                        change = next_catalog_change(&mut session_changes) => match change {
                            Ok(change) if session_was_deleted(&change) => return Ok(Value::Null),
                            Ok(_) => {}
                            Err(WatchError::Reset { .. }) => session_changes = subscribe()?,
                            Err(WatchError::Closed) => return Ok(Value::Null),
                        },
                    }
                }
            }
            DeepSeekMcpManagerRequest::Restart { name }
            | DeepSeekMcpManagerRequest::Probe { name, .. } => name,
        };
        let target = resolve_server(&declarations, name, &key, &executor)?;
        let entry = target.definition;
        let workspace = executor.workspace_root();
        let definition = entry.server.as_ref().ok_or_else(|| {
            PluginError::invalid_params_with_public_detail(
                "invalid DeepSeek managed MCP declaration",
                "The MCP server declaration is invalid.",
            )
        })?;
        let restart = matches!(&request, DeepSeekMcpManagerRequest::Restart { .. });
        if restart && !entry.enabled {
            return Ok(json!({
                "view": view(target.name(), json!({ "state": "disabled" })),
                "revision": mcp.connection_revision().saturating_add(cleanup_ledger.revision()),
            }));
        }
        let id = &target.id;
        if let Some(status) = mcp.managed_connection_status(session, id, workspace).await
            && status["reason"] == "cleanup"
        {
            return Err(host_unavailable(
                "the previous MCP connection did not confirm closure",
            ));
        }
        let mut server = if entry.enabled {
            mcp.plugin_server_in_workspace(id, workspace)
                .await
                .map_err(transport_error)?
        } else {
            crate::mcp_runtime::resolve_inactive_agent_plugin_mcp(
                target.owner,
                definition.clone(),
                runtime.workspace_root(),
                workspace,
                id.clone(),
            )
            .await
            .map_err(|_| {
                PluginError::invalid_params_with_public_detail(
                    "failed to resolve disabled DeepSeek MCP declaration",
                    "Check the server settings and required credentials.",
                )
            })?
        };
        let host = plugins
            .host_handle()
            .scoped_host_client_for_scope(key.to_string(), caller.effect_scope());
        let authority = ConnectionNetworkAuthority::current(Arc::clone(&host))?;
        let probe_options = match &request {
            DeepSeekMcpManagerRequest::Probe { options, .. } => Some(probe_options(
                *options,
                definition
                    .settings
                    .close_timeout
                    .unwrap_or(Duration::from_secs(5)),
                cancellation,
            )?),
            _ => None,
        };
        let sandbox = if probe_options
            .as_ref()
            .is_some_and(|options| options.cancellation.is_cancelled())
        {
            None
        } else {
            if let ServerSpec::Http { url, .. } = &server.spec {
                host.require_network_permission(network_permission_target(url.as_str())?)
                    .await?;
            }
            prepare_stdio_sandbox(host.as_ref(), &server.spec, true).await?
        };
        // Hold the same admission lock across reloads. A probe must record
        // its cleanup result before another operation opens a replacement.
        let operation = cleanup_ledger.lock_server(session, id).await;
        if snapshot.replaced.is_cancelled() {
            return Err(host_unavailable(
                "the DeepSeek MCP manager configuration changed",
            ));
        }
        cleanup_ledger.check_replacement(session, id, generation)?;
        if let Some(options) = probe_options {
            if let Some(prepared) = sandbox
                && let ServerSpec::Stdio { cwd, sandbox, .. } = &mut server.spec
            {
                *cwd = Some(prepared.cwd.into());
                *sandbox = Some(
                    serde_json::from_value(prepared.policy)
                        .map_err(|error| PluginError::internal_error(&error))?,
                );
            }
            let result = mcp
                .probe_plugin_server(id, server, authority, options)
                .await
                .map_err(transport_error)?;
            let result = serde_json::to_value(result)
                .map_err(|error| PluginError::internal_error(&error))?;
            if result["closeConfirmed"] == false || result.get("cleanupError").is_some() {
                cleanup_ledger.probe_failure(
                    target.identity(session, workspace),
                    generation,
                    &result,
                );
            }
            return Ok(result);
        }

        if context.call_id.is_none()
            && mcp.reconnect_supervisor_running()
            && sandbox.as_ref().is_none_or(|prepared| !prepared.call_bound)
            && let Some(policy) = definition.settings.deepseek_reconnect
        {
            let (scope, previous) = mcp
                .replace_managed_connection(session, id, workspace, snapshot.replaced.child_token())
                .await
                .map_err(transport_error)?;
            let authority = Arc::new(reconnect::ManagedAuthority {
                sessions,
                plugins: plugins.clone(),
                caller: key,
                owner: target.owner_key.clone(),
                tool: reconnect::RECONNECT_TOOL.to_owned(),
                workspace: workspace.to_owned(),
                session,
                cancellation: scope.cancellation(),
                cleanup: false,
            });
            cleanup_ledger.track(
                target.identity(session, workspace),
                generation,
                scope.clone(),
            );
            let started = reconnect::ManagedRestart {
                authority,
                mcp: mcp.clone(),
                name: id.clone(),
                scope: scope.clone(),
                previous,
                policy,
            }
            .start();
            drop(operation);
            started
                .await
                .map_err(|_| host_unavailable("the MCP restart was cancelled"))?
                .map_err(transport_error)?;
            return Ok(json!({
                "view": view(target.name(), scope.status()),
                "revision": mcp.connection_revision().saturating_add(cleanup_ledger.revision()),
            }));
        }

        let connection = match sandbox {
            Some(prepared) => {
                let scope = prepared.scope;
                let connection = mcp
                    .restart_sandbox_scope(
                        id,
                        McpSandboxContext {
                            scope: scope.clone(),
                            workspace_root: prepared.workspace_root.into(),
                            cwd: prepared.cwd.into(),
                            additional_working_directories: prepared.additional_working_directories,
                            policy: serde_json::from_value(prepared.policy)
                                .map_err(|error| PluginError::internal_error(&error))?,
                        },
                    )
                    .await
                    .map_err(transport_error)?;
                if prepared.call_bound {
                    let owner = Arc::downgrade(&mcp);
                    tokio::spawn(async move {
                        completion.wait().await;
                        if let Some(owner) = owner.upgrade() {
                            owner.release_sandbox_scope(&scope).await;
                        }
                    });
                }
                connection
            }
            None => {
                let connection = mcp
                    .for_plugin_http_call(
                        id,
                        authority.clone(),
                        workspace,
                        session,
                        completion.id(),
                        completion.cancellation_token(),
                    )
                    .await
                    .map_err(transport_error)?;
                if let Some(roots) = agena_plugin_sdk::HostClient::mcp_session_roots(self).await? {
                    authority.finish(connection.replace_roots(roots).await)?;
                }
                authority.finish(connection.reconnect(id).await)?;
                connection
            }
        };
        let revision = mcp
            .connection_revision()
            .saturating_add(cleanup_ledger.revision());
        Ok(json!({
            "view": view(target.name(), connection_status(connection.connection_observation(id).await)),
            "revision": revision,
        }))
    }
}

struct ManagedServer<'a> {
    owner: &'a LoadedPlugin,
    owner_key: PluginKey,
    definition: &'a DeepSeekManagedMcpServer,
    id: String,
    qualified_name: String,
    shared_name: bool,
}

impl ManagedServer<'_> {
    fn identity(&self, session: i64, workspace: &std::path::Path) -> cleanup::ManagedIdentity {
        cleanup::ManagedIdentity {
            id: self.id.clone(),
            owner: self.owner_key.clone(),
            display_name: self.name().to_owned(),
            qualified_name: self.qualified_name.clone(),
            workspace: workspace.to_owned(),
            session,
        }
    }

    fn name(&self) -> &str {
        if self.shared_name {
            &self.qualified_name
        } else {
            &self.definition.name
        }
    }
}

fn managed_servers(plugins: &[Arc<LoadedPlugin>]) -> Vec<ManagedServer<'_>> {
    let mut servers = Vec::new();
    let mut counts = BTreeMap::new();
    for plugin in plugins {
        let Some(package) = plugin
            .agent_plugin()
            .filter(|package| package.format == AgentPluginFormat::DeepSeekHarness)
        else {
            continue;
        };
        let key = plugin.key();
        let plugin_id = key.to_string();
        for definition in &package.deepseek_managed_mcp_servers {
            let name = definition.name.as_str();
            *counts.entry(name).or_insert(0usize) += 1;
            servers.push(ManagedServer {
                owner: plugin,
                owner_key: key.clone(),
                definition,
                id: agent_plugin_mcp_server_name(&plugin_id, name),
                qualified_name: format!("plugin:{plugin_id}:{name}"),
                shared_name: false,
            });
        }
    }
    for server in &mut servers {
        // Keep names stable when a session's policy hides one of the owners.
        server.shared_name = counts[server.definition.name.as_str()] > 1;
    }
    servers.sort_by_key(|server| server.definition.registered);
    servers
}

fn resolve_server<'entries, 'plugins>(
    servers: &'entries [ManagedServer<'plugins>],
    name: &str,
    caller: &PluginKey,
    executor: &ToolExecutor,
) -> Result<&'entries ManagedServer<'plugins>> {
    let mut shared = false;
    for server in servers
        .iter()
        .filter(|server| executor.plugin_is_enabled(&server.owner_key))
    {
        if name == server.name()
            || name == server.qualified_name
            || (name == server.definition.name && &server.owner_key == caller)
        {
            return Ok(server);
        }
        shared |= name == server.definition.name;
    }
    let message = if shared {
        "The MCP server name is shared by multiple packages. Use a qualified name from list()."
    } else {
        "The managed MCP server is unavailable in this session."
    };
    Err(PluginError::invalid_params_with_public_detail(
        message, message,
    ))
}

fn session_was_deleted(change: &KernelChange) -> bool {
    change
        .list_change
        .as_ref()
        .is_some_and(|list| list.before.is_some() && list.after.is_none())
}

async fn next_catalog_change(
    watch: &mut KernelWatch,
) -> std::result::Result<ReadView<KernelChange>, WatchError> {
    loop {
        let change = watch.recv().await?;
        if change.kind == ChangeKind::Metadata || session_was_deleted(&change) {
            return Ok(change);
        }
    }
}

async fn manager_views(
    mcp: &McpConnectionManager,
    declarations: &[ManagedServer<'_>],
    executor: &ToolExecutor,
    session: i64,
    call: &str,
    cleanup: &CleanupLedger,
    generation: u64,
) -> Vec<Value> {
    let mut views = Vec::with_capacity(declarations.len());
    let mut cleanup_rows: BTreeMap<_, _> = cleanup
        .rows(executor, session, generation)
        .into_iter()
        .map(|(identity, status)| (identity.id.clone(), (identity, status)))
        .collect();
    for server in declarations
        .iter()
        .filter(|server| executor.plugin_is_enabled(&server.owner_key))
    {
        let entry = server.definition;
        let id = &server.id;
        let status = if let Some((_, status)) = cleanup_rows.remove(id) {
            status
        } else if !entry.enabled {
            json!({ "state": "disabled" })
        } else if entry.server.is_none() || !mcp.is_plugin_server(id).await {
            json!({ "state": "failed", "reason": "load", "lastError": "The MCP declaration failed to load." })
        } else if let Some(status) = mcp
            .managed_connection_status(session, id, executor.workspace_root())
            .await
        {
            status
        } else {
            connection_status(
                mcp.plugin_connection_observation(id, executor.workspace_root(), session, call)
                    .await,
            )
        };
        views.push(view(server.name(), status));
    }
    for (_, (identity, status)) in cleanup_rows {
        let name = if views
            .iter()
            .any(|view| view["name"] == identity.display_name)
        {
            identity.qualified_name
        } else {
            identity.display_name
        };
        views.push(json!({ "name": name, "configured": false, "status": status }));
    }
    views
}

fn probe_options(
    options: DeepSeekMcpProbeOptions,
    close_timeout: Duration,
    cancellation: CancellationToken,
) -> Result<McpProbeOptions> {
    let timeout_ms = options.timeout_ms.unwrap_or(10_000.0);
    let max_pages = options.max_pages.unwrap_or(16);
    let max_tool_names = options.max_tool_names.unwrap_or(512);
    if !timeout_ms.is_finite() || timeout_ms <= 0.0 || max_pages == 0 || max_tool_names == 0 {
        return Err(PluginError::invalid_params(
            "invalid DeepSeek MCP probe budgets",
        ));
    }
    if options.aborted {
        cancellation.cancel();
    }
    Ok(McpProbeOptions {
        timeout: Duration::from_secs_f64(timeout_ms.clamp(1.0, 60_000.0) / 1_000.0),
        close_timeout: close_timeout.min(Duration::from_secs(20)),
        max_pages: max_pages.min(128) as usize,
        max_tool_names: max_tool_names.min(4096) as usize,
        cancellation,
    })
}

fn transport_error(error: agena_mcp_client::McpError) -> PluginError {
    let message = error.user_message();
    PluginError::from_kind_with_public_detail(
        agena_plugin_host::sdk::PluginErrorKind::HostUnavailable,
        &message,
        &message,
    )
}

fn view(name: &str, status: Value) -> Value {
    json!({ "name": name, "configured": true, "status": status })
}

fn connection_status(status: McpConnectionObservation) -> Value {
    match status {
        McpConnectionObservation::Closed => json!({
            "state": "closed", "result": { "closeConfirmed": true },
        }),
        McpConnectionObservation::Connecting => json!({ "state": "connecting", "attempt": 0 }),
        McpConnectionObservation::Connected {
            tool_count,
            connected_at_ms,
        } => json!({
            "state": "connected", "toolCount": tool_count, "connectedAt": connected_at_ms,
        }),
        McpConnectionObservation::Failed { message } => json!({
            "state": "failed", "reason": "reconnect-disabled", "lastError": message,
        }),
    }
}
