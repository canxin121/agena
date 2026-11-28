//! Adapt the MCP contributions of active agent plugins.

use std::{collections::BTreeMap, path::Path, sync::Arc};

use agena_mcp_client::{
    McpConnectionManager, McpHeaderHelper, McpHttpTransport, McpOnDemandConfig, McpRedactions,
    McpToolPolicy, ScopedPluginServer, ScopedPluginServerResolver, ScopedPluginTimeouts,
    ServerSpec,
};
use agena_plugin_host::agent_plugin::AgentPluginMcpTransport;
use agena_plugin_host::{
    AgentPluginContext, AgentPluginFormat, AgentPluginMcpServer, LoadedPlugin, PluginHost,
    PluginPackage, ResolvedAgentPluginMcp, agent_plugin_data_path,
};
use agena_runtime_config::{
    McpHttpAuthConfig, McpHttpEndpointConfig, McpServerConfig, McpStdioProcessConfig,
};

use super::{MCP_PLUGIN_ID, mcp_server_spec};

/// Store definitions after their source plugin and the MCP bridge activate.
pub(crate) async fn register_agent_plugin_mcp(
    host: &PluginHost,
    manager: Option<&McpConnectionManager>,
    workspace: &Path,
) {
    let Some(manager) = manager else {
        return;
    };
    if !host
        .plugins()
        .iter()
        .any(|plugin| plugin.key().to_string() == MCP_PLUGIN_ID)
    {
        return;
    }
    let logs = host.log_store();
    for plugin in host.plugins() {
        let Some(package) = plugin.agent_plugin() else {
            continue;
        };
        if package.mcp_servers.is_empty() {
            continue;
        }
        let PluginPackage::AgentPlugin {
            environment,
            user_config_env,
            ..
        } = &plugin.configured_plugin.package
        else {
            continue;
        };
        let key = plugin.key();
        let plugin_id = key.to_string();
        let data = agent_plugin_data_path(workspace, &plugin_id);
        let package = package.clone();
        let workspace = workspace.to_path_buf();
        let environment = environment.clone();
        let user_config_env = user_config_env.clone();
        let resolved = crate::blocking::FILE_OPERATIONS
            .run(move || {
                let prepare_data = if package.mcp_servers.iter().any(|server| server.is_stdio()) {
                    agena_storage_layout::secure_directory(&data)
                        .map_err(|error| format!("cannot prepare plugin data directory: {error}"))
                } else {
                    Ok(())
                };
                let needs_workspace = matches!(
                    package.format,
                    AgentPluginFormat::ClaudeCode | AgentPluginFormat::DeepSeekHarness
                );
                let paths_and_options =
                    Arc::new((package.root, data, environment, user_config_env));
                package
                    .mcp_servers
                    .into_iter()
                    .map(|server| {
                        let source_name = server.name.clone();
                        let result = (|| {
                            if server.is_stdio() {
                                prepare_data.as_ref().map_err(Clone::clone)?;
                            }
                            let name =
                                agena_plugin_host::agent_plugin::agent_plugin_mcp_server_name(
                                    &plugin_id,
                                    &source_name,
                                );
                            let resolved_name = name.clone();
                            let options = Arc::clone(&paths_and_options);
                            let timeouts = ScopedPluginTimeouts {
                                startup: server.settings.startup_timeout,
                                tool_call: server.settings.tool_timeout,
                                close: server.settings.close_timeout,
                            };
                            let on_demand = server.settings.on_demand_description_limit.map(
                                |description_limit| McpOnDemandConfig {
                                    source_name: source_name.clone(),
                                    description_limit,
                                },
                            );
                            let resolve: ScopedPluginServerResolver = Arc::new(move |workspace| {
                                let context = AgentPluginContext {
                                    root: &options.0,
                                    data: &options.1,
                                    workspace,
                                    environment: &options.2,
                                    user_config_env: &options.3,
                                };
                                resolve_plugin_server(&resolved_name, &server, &context)
                            });
                            let spec = resolve(&workspace)?;
                            Ok::<_, String>((
                                name,
                                spec,
                                timeouts,
                                needs_workspace.then_some(resolve),
                                on_demand,
                            ))
                        })();
                        (source_name, result)
                    })
                    .collect::<Vec<_>>()
            })
            .await;
        let report = |message| {
            logs.append(
                &key,
                "warn",
                "agent_plugin.mcp",
                message,
                serde_json::Value::Null,
            );
        };
        let resolved = match resolved {
            Ok(resolved) => resolved,
            Err(error) => {
                report(format!("plugin MCP configuration worker failed: {error}"));
                continue;
            }
        };
        for (server, result) in resolved {
            match result {
                Ok((name, spec, timeouts, resolve_workspace, on_demand)) => {
                    if let Err(error) = manager
                        .register_scoped_plugin_server_with_loading(
                            &name,
                            spec,
                            timeouts,
                            resolve_workspace,
                            on_demand,
                        )
                        .await
                    {
                        report(format!(
                            "could not register MCP server {server:?}: {}",
                            error.user_message()
                        ));
                    }
                }
                Err(error) => report(format!(
                    "could not configure MCP server {server:?}: {error}"
                )),
            }
        }
    }
}

pub(crate) async fn resolve_inactive_agent_plugin_mcp(
    plugin: &LoadedPlugin,
    server: AgentPluginMcpServer,
    runtime_workspace: &Path,
    workspace: &Path,
    name: String,
) -> Result<ScopedPluginServer, String> {
    let package = plugin.agent_plugin().ok_or("invalid MCP plugin package")?;
    let PluginPackage::AgentPlugin {
        environment,
        user_config_env,
        ..
    } = &plugin.configured_plugin().package
    else {
        return Err("invalid MCP plugin package".into());
    };
    let data = agent_plugin_data_path(runtime_workspace, &plugin.key().to_string());
    let root = package.root.clone();
    let workspace = workspace.to_path_buf();
    let environment = environment.clone();
    let user_config_env = user_config_env.clone();
    crate::blocking::FILE_OPERATIONS
        .run(move || {
            if server.is_stdio() {
                agena_storage_layout::secure_directory(&data)
                    .map_err(|error| format!("cannot prepare plugin data directory: {error}"))?;
            }
            let context = AgentPluginContext {
                root: &root,
                data: &data,
                workspace: &workspace,
                environment: &environment,
                user_config_env: &user_config_env,
            };
            resolve_plugin_server(&name, &server, &context)
        })
        .await
        .map_err(|error| error.to_string())?
}

fn resolve_plugin_server(
    name: &str,
    server: &AgentPluginMcpServer,
    context: &AgentPluginContext<'_>,
) -> Result<ScopedPluginServer, String> {
    let tool_policy = McpToolPolicy::ExactNames {
        enabled: server.settings.enabled_tools.clone(),
        disabled: server.settings.disabled_tools.clone(),
    };
    plugin_server_spec(name, server.resolve(context)?, context, tool_policy)
}

fn plugin_server_spec(
    name: &str,
    server: ResolvedAgentPluginMcp,
    context: &AgentPluginContext<'_>,
    tool_policy: McpToolPolicy,
) -> Result<ScopedPluginServer, String> {
    let (config, transport, header_helper, credentials) = match server {
        ResolvedAgentPluginMcp::Stdio {
            command,
            args,
            env,
            cwd,
            bundle_root,
            writable_directories,
            credential_values,
        } => (
            McpServerConfig::Stdio {
                enabled: true,
                timeouts: Default::default(),
                process: McpStdioProcessConfig {
                    command,
                    args,
                    env,
                    cwd: Some(cwd),
                    reads: std::iter::once(context.root)
                        .chain(bundle_root.as_deref())
                        .map(|path| path.to_string_lossy().into_owned())
                        .collect(),
                    writes: std::iter::once(context.data)
                        .chain(writable_directories.iter().map(|path| path.as_path()))
                        .map(|path| path.to_string_lossy().into_owned())
                        .collect(),
                    network: Vec::new(),
                },
                tools: Default::default(),
            },
            McpHttpTransport::Streamable,
            None,
            credential_values,
        ),
        ResolvedAgentPluginMcp::Http {
            url,
            transport,
            mut headers,
            bearer_env,
            headers_helper,
            credential_values,
        } => {
            let auth = if let Some(env) = bearer_env {
                Some(McpHttpAuthConfig::BearerFromEnv { env })
            } else {
                let credentials: BTreeMap<_, _> = headers
                    .extract_if(.., |name, _| {
                        ["authorization", "proxy-authorization", "cookie"]
                            .iter()
                            .any(|header| name.eq_ignore_ascii_case(header))
                    })
                    .collect();
                (!credentials.is_empty()).then_some(McpHttpAuthConfig::Custom {
                    headers: credentials,
                })
            };
            let transport = match transport {
                AgentPluginMcpTransport::Streamable => McpHttpTransport::Streamable,
                AgentPluginMcpTransport::Sse => McpHttpTransport::Sse,
                AgentPluginMcpTransport::WebSocket => McpHttpTransport::WebSocket,
            };
            (
                McpServerConfig::Http {
                    enabled: true,
                    timeouts: Default::default(),
                    endpoint: McpHttpEndpointConfig { url, headers },
                    auth,
                    tools: Default::default(),
                },
                transport,
                headers_helper.map(|helper| McpHeaderHelper {
                    command: helper.command,
                    cwd: helper.cwd,
                    server_name: helper.server_name,
                    display_url: helper.display_url,
                }),
                credential_values,
            )
        }
    };
    let mut spec = mcp_server_spec(name, &config, context.workspace)?;
    match &mut spec {
        ServerSpec::Stdio {
            tool_policy: policy,
            ..
        } => *policy = tool_policy,
        ServerSpec::Http {
            transport: selected,
            header_helper: selected_helper,
            tool_policy: policy,
            ..
        } => {
            *selected = transport;
            *selected_helper = header_helper;
            *policy = tool_policy;
        }
    }
    let redactions =
        McpRedactions::for_credentials(credentials).map_err(|error| error.to_string())?;
    Ok(ScopedPluginServer { spec, redactions })
}
