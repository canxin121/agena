//! Thin one-shot session commands backed by the long-lived server.
//!
//! This module deliberately depends only on the public API/client contracts.
//! It never opens the session database, creates an Application, starts a
//! scheduler/plugin host, or acquires an execution lease.

use std::{collections::BTreeMap, fs, io::Read as _, path::Path, time::Duration};

use super::{
    ActiveSnapshotOutput, AgenaCli, AppError, ApplyArgs, ApplyOutput, AuthCommand, AuthListArgs,
    AuthListOutput, AuthSubcommand, AuthSummary, CallToolParams, CallToolResult, CommitArgs,
    CommitOutput, ConfigCommand, ConfigResolveArgs, ConfigSubcommand, ContinueArgs, CostArgs,
    CostOutput, DebugCommand, DebugRunOutput, DebugSessionArgs, DebugSessionOutput,
    DebugSubcommand, DiagnosticsArgs, DiagnosticsConfigOutput, DiagnosticsEnvironmentOutput,
    DiagnosticsOutput, ExecArgs, ExecOutput, ForkArgs, GitArgs, GitOutput, LoginArgs, LogoutArgs,
    ManagedSnapshotOutput, McpAddArgs, McpConfigLayerArg, McpHttpAuthArg, McpPluginToggleArgs,
    McpReconnectArgs, McpRemoveArgs, McpServerArgs, McpServerBackend, McpServerError,
    McpStatusArgs, MemoryCommand, MemoryListArgs, MemoryListOutput, MemorySubcommand,
    MemorySummaryOutput, OutputFormat, PermissionModeArg, PermissionReplyKindArg,
    PermissionScopeArg, PermissionsArgs, PermissionsListArgs, PermissionsOutput,
    PermissionsSubcommand, PermissionsWriteArgs, PluginInspectArgs, PluginInspectOutput,
    PluginLogOutputFormat, PluginLogsArgs, PluginLogsOutput, PluginStatusArgs, PluginStatusOutput,
    PrArgs, PrOutput, ProviderCapabilitiesOutput, ProviderCommand, ProviderDefaultsSummary,
    ProviderListArgs, ProviderListOutput, ProviderModelsOutput, ProviderSubcommand,
    ProviderSummary, ResumeArgs, ReviewArgs, SessionDetail, SessionForkOutput, SessionImportOutput,
    SessionListArgs, SessionListOutput, SessionListView, SessionOutput, SessionSummary,
    SessionsCommand, SessionsSubcommand, SnapshotArgs, SnapshotBackendSupportOutput,
    SnapshotCapabilitiesOutput, SnapshotOutput, ToolDescriptor, UsageArgs, WorkflowState,
    async_trait, browser_login_redirect_uri, filter_session_summaries_by_view, format_apply_output,
    format_debug_session_output, format_plugin_logs_output, memory_type_label,
    normalize_login_provider, paginate_session_summaries, permission_rule_output,
    prompt_browser_login, prompt_device_login, render_serialized, review_prompt, title_from_prompt,
    usage_stats_query_from_args,
};
use agena_api::{
    commands::{
        Command, CommandResult, ForkSessionParams, ImportSessionParams, ListSessionTreeParams,
        ReplacePermissionRuleParams, ResolveWorkspaceParams, RevokePermissionRuleParams,
        SubmitRunParams, UpsertPermissionRuleParams,
    },
    queries::{ListPermissionRulesParams, ListSessionsParams, Query, QueryResult},
    resource::{
        ModelRef as WireModelRef, PermissionMode as WirePermissionMode, PermissionReply,
        PermissionReplyKind as WirePermissionReplyKind, PermissionScope as WirePermissionScope,
        ProviderSummaryResource, RunOptions, SessionExecutionResource, SessionResource,
        SessionTranscriptPart,
    },
};
use agena_application::dto::{
    AuthBrowserStartResource, AuthDeviceStartResource, AuthLoginKindResource,
    AuthLoginResultResource, AuthProviderResource, GitCommitResource, GitPullRequestResource,
    GitStatusResource, MemoryResource, OperatorToolResource, SnapshotStatusResource,
};
use agena_client::{AgenaClient, ClientError};

struct ServerSessionClient {
    client: AgenaClient,
    workspace_id: i64,
    workspace_root: std::path::PathBuf,
}

#[derive(serde::Deserialize)]
struct ServerMemoryOverview {
    workspace_root: String,
    directory: String,
    items: Vec<MemoryResource>,
}

#[derive(serde::Deserialize)]
struct ServerPathResource {
    path: String,
}

#[derive(Clone)]
pub(super) struct ServerMcpBackend {
    client: AgenaClient,
    workspace_id: i64,
}

const HIDDEN_MCP_PLUGIN_IDS: &[&str] = &["agena.chatgpt", "agena.gemini", "agena.claude"];

fn name_belongs_to_plugin(name: &str, plugin_id: &str) -> bool {
    name == plugin_id
        || name
            .strip_prefix(plugin_id)
            .is_some_and(|suffix| suffix.starts_with('.'))
}

fn mcp_tool_uses_hidden_provider(tool: &OperatorToolResource) -> bool {
    if let Some(plugin_id) = tool.plugin_id.as_deref() {
        return HIDDEN_MCP_PLUGIN_IDS.contains(&plugin_id);
    }

    // Older Agena servers do not send plugin_id yet. Their compact tool names
    // still carry the provider name, so retain a conservative compatibility
    // fallback until those servers are upgraded.
    HIDDEN_MCP_PLUGIN_IDS.iter().any(|plugin_id| {
        let compact_plugin_id = plugin_id.strip_prefix("agena.").unwrap_or(plugin_id);
        name_belongs_to_plugin(tool.name.as_str(), compact_plugin_id)
            || name_belongs_to_plugin(tool.name.as_str(), plugin_id)
    })
}

fn mcp_tool_is_exposed(tool: &OperatorToolResource) -> bool {
    !tool.interactive && !mcp_tool_uses_hidden_provider(tool)
}

fn mcp_tool_is_callable(tools: &[OperatorToolResource], name: &str) -> bool {
    tools
        .iter()
        .any(|tool| tool.name == name && mcp_tool_is_exposed(tool))
}

#[async_trait]
impl McpServerBackend for ServerMcpBackend {
    async fn list_tools(&self) -> Result<Vec<ToolDescriptor>, McpServerError> {
        let value = self
            .client
            .operator_tools()
            .await
            .map_err(|error| McpServerError::Backend(error.to_string()))?;
        let tools: Vec<OperatorToolResource> = serde_json::from_value(value)?;
        Ok(tools
            .into_iter()
            .filter(mcp_tool_is_exposed)
            .map(|tool| ToolDescriptor {
                name: tool.name,
                title: None,
                aliases: Vec::new(),
                description: tool.summary,
                before_help: tool.before_help,
                after_help: tool.after_help,
                input_schema: Some(tool.input_schema),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: Vec::new(),
                meta: None,
            })
            .collect())
    }

    async fn call_tool(&self, params: CallToolParams) -> Result<CallToolResult, McpServerError> {
        // Re-read the live catalog for every call. Besides handling runtime
        // reloads, this closes the gap where a client skips tools/list and
        // guesses the name of an interactive or provider-owned tool.
        let value = self
            .client
            .operator_tools()
            .await
            .map_err(|error| McpServerError::Backend(error.to_string()))?;
        let tools: Vec<OperatorToolResource> = serde_json::from_value(value)?;
        if !mcp_tool_is_callable(&tools, params.name.as_str()) {
            return Err(McpServerError::NotFound(format!(
                "tool '{}' is not exposed by the Agena MCP server",
                params.name
            )));
        }

        let result = self
            .client
            .invoke_operator_tool(self.workspace_id, params.name.as_str(), params.arguments)
            .await;
        match result {
            Ok(value) => {
                let summary: agena_tool::ToolExecutionSummary = serde_json::from_value(value)?;
                let text = if summary.output_text.is_empty() {
                    serde_json::to_string_pretty(&summary.payload)
                        .unwrap_or_else(|_| "<empty output>".to_owned())
                } else {
                    summary.output_text
                };
                Ok(agena_mcp_server::text_result(text))
            }
            Err(error) => Ok(agena_mcp_server::text_error(error.to_string())),
        }
    }
}

#[derive(serde::Serialize)]
struct McpReconnectOutput {
    title: String,
    output_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<serde_json::Value>,
}

impl ServerSessionClient {
    async fn connect(cli: &AgenaCli, workspace: Option<&Path>) -> Result<Self, AppError> {
        let client = connect_server_client(cli).await?;
        let workspace_root = workspace
            .map(Path::to_path_buf)
            .map(Ok)
            .unwrap_or_else(std::env::current_dir)?;
        let workspace = client
            .command(Command::ResolveWorkspace(ResolveWorkspaceParams {
                path: workspace_root.to_string_lossy().into_owned(),
                create_if_missing: true,
            }))
            .await
            .map_err(|error| {
                client_error(
                    "failed to resolve the CLI workspace through the server",
                    error,
                )
            })?;
        let CommandResult::Workspace(workspace) = workspace else {
            return Err(AppError::Internal(
                "server returned the wrong workspace result".to_owned(),
            ));
        };
        Ok(Self {
            client,
            workspace_id: workspace.id,
            workspace_root,
        })
    }

    async fn run_options(
        &self,
        model: Option<&str>,
        temperature: Option<f32>,
        max_output_tokens: Option<u32>,
    ) -> Result<RunOptions, AppError> {
        let model = match model {
            Some(target) => {
                let providers = self
                    .client
                    .query(Query::ListProviders)
                    .await
                    .map_err(|error| {
                        client_error("failed to read the server's provider catalog", error)
                    })?;
                let QueryResult::Providers(providers) = providers else {
                    return Err(AppError::Internal(
                        "server returned the wrong provider-list result".to_owned(),
                    ));
                };
                Some(resolve_model_target(providers.as_slice(), target)?)
            }
            None => None,
        };
        Ok(RunOptions {
            model,
            temperature,
            max_output_tokens,
            ..RunOptions::default()
        })
    }

    async fn list_sessions(&self, limit: Option<u64>) -> Result<Vec<SessionResource>, AppError> {
        let mut cursor = None;
        let mut sessions = Vec::new();
        loop {
            let response = self
                .client
                .query(Query::ListSessions(ListSessionsParams {
                    cursor,
                    limit: Some(
                        limit
                            .unwrap_or(agena_api::pagination::MAX_LIMIT)
                            .clamp(1, agena_api::pagination::MAX_LIMIT),
                    ),
                    workspace_id: Some(self.workspace_id),
                    parent_id: None,
                    roots: false,
                    exclude_subagents: true,
                    search: None,
                }))
                .await
                .map_err(|error| client_error("failed to list sessions from the server", error))?;
            let QueryResult::Sessions(page) = response else {
                return Err(AppError::Internal(
                    "server returned the wrong session-list result".to_owned(),
                ));
            };
            sessions.extend(page.items);
            if limit.is_some_and(|limit| sessions.len() as u64 >= limit) {
                sessions.truncate(limit.unwrap_or_default() as usize);
                return Ok(sessions);
            }
            if !page.page.has_more {
                return Ok(sessions);
            }
            cursor = page.page.next_cursor;
            if cursor.is_none() {
                return Err(AppError::Internal(
                    "server returned a truncated session page without a cursor".to_owned(),
                ));
            }
        }
    }

    async fn list_permission_rules(
        &self,
        search: Option<String>,
    ) -> Result<Vec<agena_api::resource::PermissionRuleResource>, AppError> {
        let mut cursor = None;
        let mut rules = Vec::new();
        loop {
            let response = self
                .client
                .query(Query::ListPermissionRules(ListPermissionRulesParams {
                    cursor,
                    limit: Some(200),
                    search: search.clone(),
                }))
                .await
                .map_err(|error| {
                    client_error("failed to list permission rules from server", error)
                })?;
            let QueryResult::PermissionRules(page) = response else {
                return Err(AppError::Internal(
                    "server returned the wrong permission-rule list result".to_owned(),
                ));
            };
            rules.extend(page.items);
            if !page.page.has_more {
                return Ok(rules);
            }
            cursor = page.page.next_cursor;
            if cursor.is_none() {
                return Err(AppError::Internal(
                    "server returned a truncated permission-rule page without a cursor".to_owned(),
                ));
            }
        }
    }

    async fn selected_session_id(
        &self,
        session_id: Option<i64>,
        last: bool,
    ) -> Result<i64, AppError> {
        if session_id.is_some() && last {
            return Err(AppError::Config(
                "pass either a session id or --last, not both".to_owned(),
            ));
        }
        if let Some(session_id) = session_id {
            return Ok(session_id);
        }
        self.list_sessions(Some(1))
            .await?
            .first()
            .map(|session| session.id)
            .ok_or_else(|| AppError::Config("no sessions found".to_owned()))
    }

    async fn execution(&self, session_id: i64) -> Result<SessionExecutionResource, AppError> {
        self.client
            .get_session_state_with_parts(session_id, true)
            .await
            .map_err(|error| client_error("failed to read session from server", error))
    }
}

async fn connect_server_client(cli: &AgenaCli) -> Result<AgenaClient, AppError> {
    if cli.database_url.is_some() || cli.database_path.is_some() {
        return Err(AppError::Config(
                "--database-url/--database-path belong to the server and cannot be used by thin CLI session commands"
                    .to_owned(),
            ));
    }
    if !cli.overrides.is_empty() {
        return Err(AppError::Config(
            "--set overrides belong to the server and cannot be used by thin CLI session commands"
                .to_owned(),
        ));
    }

    let server_url = cli
        .server
        .as_deref()
        .filter(|url| !url.trim().is_empty())
        .unwrap_or("http://127.0.0.1:3210");
    let client = AgenaClient::connect_server(
        server_url,
        cli.server_token.as_deref(),
        cli.server_password.as_deref(),
    )
    .await
    .map_err(|error| client_error("server readiness handshake failed", error))?;
    Ok(client)
}

fn ensure_server_workspace_matches(
    workspace_root: &str,
    requested_workspace: &Path,
) -> Result<(), AppError> {
    let cli_workspace = fs::canonicalize(requested_workspace).map_err(|error| {
        AppError::Config(format!(
            "failed to canonicalize requested CLI workspace `{}`: {error}",
            requested_workspace.display()
        ))
    })?;
    let server_workspace = fs::canonicalize(workspace_root).map_err(|error| {
        AppError::Config(format!(
            "failed to canonicalize server workspace `{workspace_root}`: {error}"
        ))
    })?;
    if cli_workspace != server_workspace {
        return Err(AppError::Config(format!(
            "server is bound to workspace `{}`, but the CLI current directory is `{}`; refusing to operate on a different repository",
            server_workspace.display(),
            cli_workspace.display()
        )));
    }
    Ok(())
}

fn ensure_server_workspace_matches_cli(workspace_root: &str) -> Result<(), AppError> {
    let current_dir = std::env::current_dir()?;
    ensure_server_workspace_matches(workspace_root, current_dir.as_path())
}

fn decode_server_resource<T>(value: serde_json::Value, resource_name: &str) -> Result<T, AppError>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value(value).map_err(|error| {
        AppError::Internal(format!(
            "server returned an invalid {resource_name} response: {error}"
        ))
    })
}

async fn connect_workspace_bound_client(
    cli: &AgenaCli,
    requested_workspace: &Path,
) -> Result<ServerMcpBackend, AppError> {
    let client = connect_server_client(cli).await?;
    let status: GitStatusResource = decode_server_resource(
        client
            .git_status()
            .await
            .map_err(|error| client_error("failed to verify the server workspace", error))?,
        "git-status",
    )?;
    ensure_server_workspace_matches(status.workspace_root.as_str(), requested_workspace)?;
    let workspace = client
        .command(Command::ResolveWorkspace(ResolveWorkspaceParams {
            path: requested_workspace.to_string_lossy().into_owned(),
            create_if_missing: true,
        }))
        .await
        .map_err(|error| {
            client_error(
                "failed to resolve the operator workspace through the server",
                error,
            )
        })?;
    let CommandResult::Workspace(workspace) = workspace else {
        return Err(AppError::Internal(
            "server returned the wrong operator workspace result".to_owned(),
        ));
    };
    Ok(ServerMcpBackend {
        client,
        workspace_id: workspace.id,
    })
}

impl AgenaCli {
    pub(super) async fn render_server_apply_command(
        &self,
        args: ApplyArgs,
    ) -> Result<String, AppError> {
        let requested_workspace = args
            .workspace
            .as_deref()
            .map(Path::to_path_buf)
            .map(Ok)
            .unwrap_or_else(std::env::current_dir)?;
        let operator = connect_workspace_bound_client(self, requested_workspace.as_path()).await?;
        let patch = fs::read_to_string(&args.patch_file)?;
        let value = operator
            .client
            .invoke_operator_tool(
                operator.workspace_id,
                "fs.apply_patch",
                Some(serde_json::json!({ "patch": patch })),
            )
            .await
            .map_err(|error| client_error("failed to apply patch through server", error))?;
        let summary: agena_tool::ToolExecutionSummary =
            decode_server_resource(value, "operator-tool-result")?;
        let patch_payload = summary.payload.ok_or_else(|| {
            AppError::Internal("apply_patch tool did not return patch metadata".to_owned())
        })?;
        let patch = agena_tool::ApplyPatchExecution::from_tool_payload(&patch_payload).ok_or_else(
            || AppError::Internal("apply_patch tool returned invalid patch metadata".to_owned()),
        )?;
        if args.json {
            render_serialized(
                OutputFormat::Json,
                &ApplyOutput {
                    title: summary.title,
                    output_text: summary.output_text,
                    patch,
                },
            )
        } else {
            Ok(format_apply_output(&patch))
        }
    }

    pub(super) async fn server_mcp_backend(
        &self,
        args: McpServerArgs,
    ) -> Result<ServerMcpBackend, AppError> {
        let requested_workspace = args
            .workspace
            .as_deref()
            .map(Path::to_path_buf)
            .map(Ok)
            .unwrap_or_else(std::env::current_dir)?;
        connect_workspace_bound_client(self, requested_workspace.as_path()).await
    }

    pub(super) async fn render_server_mcp_reconnect(
        &self,
        args: McpReconnectArgs,
    ) -> Result<String, AppError> {
        let server = args.server.trim();
        if server.is_empty() {
            return Err(AppError::Config(
                "MCP server name must not be empty".to_owned(),
            ));
        }
        let client = connect_server_client(self).await?;
        let status = client.runtime_status().await.map_err(|error| {
            client_error("failed to resolve the server operator workspace", error)
        })?;
        let workspace = client
            .command(Command::ResolveWorkspace(ResolveWorkspaceParams {
                path: status.workspace_root,
                create_if_missing: true,
            }))
            .await
            .map_err(|error| {
                client_error("failed to resolve the server operator workspace", error)
            })?;
        let CommandResult::Workspace(workspace) = workspace else {
            return Err(AppError::Internal(
                "server returned the wrong operator workspace result".to_owned(),
            ));
        };
        let value = client
            .invoke_operator_tool(
                workspace.id,
                "agena.mcp.servers.reconnect",
                Some(serde_json::json!({ "server": server })),
            )
            .await
            .map_err(|error| {
                client_error("failed to reconnect MCP server through server", error)
            })?;
        let summary: agena_tool::ToolExecutionSummary =
            decode_server_resource(value, "operator-tool-result")?;
        render_serialized(
            args.format,
            &McpReconnectOutput {
                title: summary.title,
                output_text: summary.output_text,
                payload: summary.payload,
            },
        )
    }

    pub(super) async fn render_server_mcp_add(&self, args: McpAddArgs) -> Result<String, AppError> {
        let server = normalized_mcp_server_name(args.server.as_str())?;
        let config = mcp_server_config_value(&args)?;
        self.mutate_server_mcp_plugin_config(
            args.layer,
            args.dry_run,
            !args.no_reload,
            args.format,
            McpConfigMutation::Add {
                server,
                config,
                force: args.force,
            },
        )
        .await
    }

    pub(super) async fn render_server_mcp_remove(
        &self,
        args: McpRemoveArgs,
    ) -> Result<String, AppError> {
        let server = normalized_mcp_server_name(args.server.as_str())?;
        self.mutate_server_mcp_plugin_config(
            args.layer,
            args.dry_run,
            !args.no_reload,
            args.format,
            McpConfigMutation::Remove { server },
        )
        .await
    }

    pub(super) async fn render_server_mcp_toggle(
        &self,
        args: McpPluginToggleArgs,
        enabled: bool,
    ) -> Result<String, AppError> {
        self.mutate_server_mcp_plugin_config(
            args.layer,
            args.dry_run,
            !args.no_reload,
            args.format,
            McpConfigMutation::SetEnabled(enabled),
        )
        .await
    }

    async fn mutate_server_mcp_plugin_config(
        &self,
        layer: McpConfigLayerArg,
        dry_run: bool,
        reload: bool,
        format: OutputFormat,
        mutation: McpConfigMutation,
    ) -> Result<String, AppError> {
        let client = match layer {
            McpConfigLayerArg::Global => connect_server_client(self).await?,
            McpConfigLayerArg::Workspace => {
                let current = std::env::current_dir()?;
                connect_workspace_bound_client(self, current.as_path())
                    .await?
                    .client
            }
        };
        let layer = mcp_config_layer_name(layer);
        let current = client
            .settings_layer_value(layer, MCP_PLUGIN_SETTINGS_PATH)
            .await
            .map_err(|error| client_error("failed to read server MCP settings", error))?;
        let current = current.get("value").cloned().ok_or_else(|| {
            AppError::Internal("server returned an invalid settings-layer response".to_owned())
        })?;
        let mut record = mcp_plugin_record(current)?;
        apply_mcp_config_mutation(&mut record, mutation)?;
        let response = client
            .set_settings_layer_value(
                layer,
                MCP_PLUGIN_SETTINGS_PATH,
                serde_json::Value::Object(record),
                dry_run,
                reload,
            )
            .await
            .map_err(|error| client_error("failed to update server MCP settings", error))?;
        render_serialized(format, &response)
    }

    pub(super) async fn render_server_memory_command(
        &self,
        command: MemoryCommand,
    ) -> Result<String, AppError> {
        let command = command
            .command
            .unwrap_or(MemorySubcommand::List(MemoryListArgs {
                workspace: None,
                format: OutputFormat::Json,
            }));
        let requested_workspace = match &command {
            MemorySubcommand::List(args) => args.workspace.as_deref(),
            MemorySubcommand::Forget(args) => args.workspace.as_deref(),
            MemorySubcommand::Edit(args) => args.workspace.as_deref(),
        }
        .map(Path::to_path_buf)
        .map(Ok)
        .unwrap_or_else(std::env::current_dir)?;
        let client = connect_server_client(self).await?;
        let overview: ServerMemoryOverview = decode_server_resource(
            client
                .memory_overview()
                .await
                .map_err(|error| client_error("failed to read memories from server", error))?,
            "memory-overview",
        )?;
        ensure_server_workspace_matches(
            overview.workspace_root.as_str(),
            requested_workspace.as_path(),
        )?;

        match command {
            MemorySubcommand::List(args) => {
                let memories = overview
                    .items
                    .into_iter()
                    .map(|memory| MemorySummaryOutput {
                        file_name: memory.file_name,
                        name: memory.name,
                        description: memory.description,
                        memory_type: memory_type_label(memory.memory_type),
                        path: memory.path,
                    })
                    .collect::<Vec<_>>();
                render_serialized(
                    args.format,
                    &MemoryListOutput {
                        dir: overview.directory,
                        count: memories.len(),
                        memories,
                    },
                )
            }
            MemorySubcommand::Forget(args) => {
                client
                    .delete_memory(args.name.as_str())
                    .await
                    .map_err(|error| {
                        client_error("failed to forget memory through server", error)
                    })?;
                Ok(format!("forgot memory: {}", args.name))
            }
            MemorySubcommand::Edit(args) => match args.name {
                Some(name) => {
                    let memory: MemoryResource = decode_server_resource(
                        client.get_memory(name.as_str()).await.map_err(|error| {
                            client_error("failed to read memory from server", error)
                        })?,
                        "memory",
                    )?;
                    Ok(memory.path)
                }
                None => {
                    let index: ServerPathResource = decode_server_resource(
                        client.ensure_memory_index().await.map_err(|error| {
                            client_error("failed to ensure memory index through server", error)
                        })?,
                        "memory-index",
                    )?;
                    Ok(index.path)
                }
            },
        }
    }

    pub(super) async fn render_server_mcp_status(
        &self,
        args: McpStatusArgs,
        server: Option<String>,
    ) -> Result<String, AppError> {
        let client = connect_server_client(self).await?;
        let runtime = client
            .runtime_status()
            .await
            .map_err(|error| client_error("failed to read MCP status from server", error))?;
        let mcp = runtime.operator.mcp;
        match server {
            Some(server) => {
                let server = mcp
                    .servers
                    .into_iter()
                    .find(|entry| entry.name == server)
                    .ok_or_else(|| {
                        AppError::Config(format!("MCP server not configured: {server}"))
                    })?;
                render_serialized(args.format, &server)
            }
            None => render_serialized(args.format, &mcp),
        }
    }

    pub(super) async fn render_server_config_command(
        &self,
        command: ConfigCommand,
    ) -> Result<String, AppError> {
        let client = connect_server_client(self).await?;
        match command
            .command
            .unwrap_or(ConfigSubcommand::Resolve(ConfigResolveArgs {
                format: OutputFormat::Json,
            })) {
            ConfigSubcommand::Resolve(args) => {
                let document = client.resolved_config().await.map_err(|error| {
                    client_error("failed to read resolved configuration from server", error)
                })?;
                render_serialized(args.format, &document)
            }
            ConfigSubcommand::Validate => {
                let response = client.validate_config().await.map_err(|error| {
                    client_error("failed to validate server configuration", error)
                })?;
                let valid = response
                    .get("valid")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                let path = response
                    .get("config_path")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("<unknown>");
                if !valid {
                    return Err(AppError::Config(format!(
                        "server configuration is invalid: path={path}"
                    )));
                }
                Ok(format!("config valid: path={path}"))
            }
        }
    }

    pub(super) async fn render_server_diagnostics_command(
        &self,
        args: DiagnosticsArgs,
    ) -> Result<String, AppError> {
        let client = connect_server_client(self).await?;
        let runtime = client
            .runtime_status()
            .await
            .map_err(|error| client_error("failed to read server runtime status", error))?;
        ensure_server_workspace_matches_cli(runtime.workspace_root.as_str())?;
        let document = client
            .resolved_config()
            .await
            .map_err(|error| client_error("failed to read server configuration metadata", error))?;
        let metadata = document.get("meta").cloned().unwrap_or_default();
        let project_path = metadata
            .get("project_config_path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let project_found = metadata
            .get("project_config_found")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let applied_layers = metadata
            .get("applied_layers")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|layer| layer.get("description").and_then(serde_json::Value::as_str))
            .map(str::to_owned)
            .collect();
        render_serialized(
            args.format,
            &DiagnosticsOutput {
                version: env!("CARGO_PKG_VERSION"),
                os: std::env::consts::OS.to_owned(),
                arch: std::env::consts::ARCH.to_owned(),
                current_dir: std::env::current_dir()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|_| "<unavailable>".to_owned()),
                config: DiagnosticsConfigOutput {
                    path: runtime.config_path,
                    found: runtime.config_found,
                    project_path,
                    project_found,
                    applied_layers,
                    provider_count: runtime.provider_ids.len(),
                    plugin_count: runtime.plugin_count,
                },
                environment: DiagnosticsEnvironmentOutput {
                    agena_database_url_set: std::env::var_os("AGENA_DATABASE_URL").is_some(),
                    agena_database_path_set: std::env::var_os("AGENA_DATABASE_PATH").is_some(),
                    agena_adapter_log_set: std::env::var_os("AGENA_ADAPTER_LOG").is_some(),
                },
            },
        )
    }

    pub(super) async fn render_server_snapshot_command(
        &self,
        args: SnapshotArgs,
    ) -> Result<String, AppError> {
        let client = connect_server_client(self).await?;
        let snapshot: SnapshotStatusResource = decode_server_resource(
            client.snapshot_status().await.map_err(|error| {
                client_error("failed to read snapshot status from server", error)
            })?,
            "snapshot-status",
        )?;
        ensure_server_workspace_matches_cli(snapshot.workspace_root.as_str())?;
        if !snapshot.registry_available {
            return Err(AppError::Config(
                "snapshot registry is not enabled in the server".to_owned(),
            ));
        }
        let active = snapshot
            .active
            .into_iter()
            .map(|entry| ActiveSnapshotOutput {
                session_id: entry.session_id,
                path: entry.path,
                branch: entry.branch,
                backend: entry.backend,
                created_here: entry.created_here,
            })
            .collect::<Vec<_>>();
        let managed = snapshot
            .managed
            .into_iter()
            .map(|entry| ManagedSnapshotOutput {
                path: entry.path,
                session_id: entry.session_id,
                branch: entry.branch,
                backend: entry.backend,
                registered_with_git: entry.registered_with_git,
                registered_with_rift: entry.registered_with_rift,
                stale: entry.stale,
            })
            .collect::<Vec<_>>();
        render_serialized(
            args.format,
            &SnapshotOutput {
                workspace_root: snapshot.workspace_root,
                capabilities: SnapshotCapabilitiesOutput {
                    preferred_backend: snapshot.preferred_backend,
                    git: SnapshotBackendSupportOutput {
                        available: snapshot.git.available,
                        detail: snapshot.git.detail,
                    },
                    rift: SnapshotBackendSupportOutput {
                        available: snapshot.rift.available,
                        detail: snapshot.rift.detail,
                    },
                },
                active,
                managed,
            },
        )
    }

    pub(super) async fn render_server_git_command(
        &self,
        args: GitArgs,
    ) -> Result<String, AppError> {
        let client = connect_server_client(self).await?;
        let status: GitStatusResource = decode_server_resource(
            client
                .git_status()
                .await
                .map_err(|error| client_error("failed to read git status from server", error))?,
            "git-status",
        )?;
        ensure_server_workspace_matches_cli(status.workspace_root.as_str())?;
        render_serialized(
            args.format,
            &GitOutput {
                workspace_root: status.workspace_root,
                git_available: status.git_available,
                repo: status.repo,
                gh_available: status.gh_available,
                branch: status.branch,
                upstream: status.upstream,
                ahead: status.ahead,
                behind: status.behind,
                staged_files: status.staged_files,
                unstaged_files: status.unstaged_files,
                untracked_files: status.untracked_files,
                changed_files: status.changed_files,
                clean: status.clean,
                snapshot_active_sessions: status.snapshot_active_sessions,
                snapshot_managed_dirs: status.snapshot_managed_dirs,
            },
        )
    }

    pub(super) async fn render_server_commit_command(
        &self,
        args: CommitArgs,
    ) -> Result<String, AppError> {
        let client = connect_server_client(self).await?;
        let status: GitStatusResource = decode_server_resource(
            client.git_status().await.map_err(|error| {
                client_error(
                    "failed to verify the server workspace before committing",
                    error,
                )
            })?,
            "git-status",
        )?;
        ensure_server_workspace_matches_cli(status.workspace_root.as_str())?;

        let result: GitCommitResource = decode_server_resource(
            client
                .create_git_commit(args.message)
                .await
                .map_err(|error| {
                    client_error("failed to create git commit through server", error)
                })?,
            "git-commit",
        )?;
        ensure_server_workspace_matches_cli(result.status.workspace_root.as_str())?;
        render_serialized(
            args.format,
            &CommitOutput {
                workspace_root: result.status.workspace_root,
                commit: result.commit,
                summary: result.summary,
            },
        )
    }

    pub(super) async fn render_server_pr_command(&self, args: PrArgs) -> Result<String, AppError> {
        let client = connect_server_client(self).await?;
        let status: GitStatusResource = decode_server_resource(
            client.git_status().await.map_err(|error| {
                client_error(
                    "failed to verify the server workspace before creating a pull request",
                    error,
                )
            })?,
            "git-status",
        )?;
        ensure_server_workspace_matches_cli(status.workspace_root.as_str())?;
        let branch = args
            .head
            .clone()
            .or(status.branch)
            .ok_or_else(|| AppError::Config("could not determine current branch".to_owned()))?;
        let created: GitPullRequestResource = decode_server_resource(
            client
                .create_git_pull_request(args.title, args.body, args.base, args.head)
                .await
                .map_err(|error| {
                    client_error("failed to create pull request through server", error)
                })?,
            "git-pull-request",
        )?;
        render_serialized(
            args.format,
            &PrOutput {
                workspace_root: status.workspace_root,
                branch,
                url: created.url,
            },
        )
    }

    pub(super) async fn render_server_auth_command(
        &self,
        command: AuthCommand,
    ) -> Result<String, AppError> {
        let client = connect_server_client(self).await?;
        match command
            .command
            .unwrap_or(AuthSubcommand::List(AuthListArgs {
                format: OutputFormat::Json,
            })) {
            AuthSubcommand::List(args) => {
                let value = client.auth_providers().await.map_err(|error| {
                    client_error("failed to read provider authentication from server", error)
                })?;
                let items = value.as_array().ok_or_else(|| {
                    AppError::Internal("server returned an invalid auth-provider list".to_owned())
                })?;
                let mut credentials = items
                    .iter()
                    .map(auth_summary_from_value)
                    .collect::<Result<Vec<_>, _>>()?;
                credentials.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
                render_serialized(args.format, &AuthListOutput { credentials })
            }
        }
    }

    pub(super) async fn run_server_login(&self, args: LoginArgs) -> Result<(), AppError> {
        let provider_id = normalize_login_provider(args.provider_id.as_str());
        let method_count = usize::from(args.api_key.is_some())
            + usize::from(args.browser)
            + usize::from(args.device);
        if method_count != 1 {
            return Err(AppError::Config(
                "login requires exactly one of --api-key, --browser, or --device".to_owned(),
            ));
        }
        let client = connect_server_client(self).await?;
        let provider: AuthProviderResource = decode_server_resource(
            client
                .auth_provider(provider_id.as_str())
                .await
                .map_err(|error| {
                    client_error("failed to read provider authentication from server", error)
                })?,
            "auth-provider",
        )?;

        if let Some(api_key) = args.api_key {
            if !provider.api_key_write_supported {
                return Err(AppError::Config(format!(
                    "{provider_id} does not support api key login"
                )));
            }
            client
                .set_auth_api_key(provider_id.as_str(), api_key)
                .await
                .map_err(|error| {
                    client_error("failed to store provider credential in server", error)
                })?;
            println!("logged in: {provider_id}");
            return Ok(());
        }

        if args.browser {
            let kind = provider.browser_login_kind.ok_or_else(|| {
                AppError::Config(format!("{provider_id} does not support browser login"))
            })?;
            let redirect_uri = browser_login_redirect_uri(args.port);
            let start_value = match kind {
                AuthLoginKindResource::OpenaiChatgpt => {
                    client
                        .start_openai_browser_auth(provider_id.as_str(), redirect_uri.as_str())
                        .await
                }
                AuthLoginKindResource::Gitlab => {
                    client
                        .start_gitlab_browser_auth(provider_id.as_str(), redirect_uri.as_str())
                        .await
                }
                AuthLoginKindResource::GithubCopilot => {
                    return Err(AppError::Config(format!(
                        "{provider_id} does not support browser login"
                    )));
                }
            }
            .map_err(|error| client_error("failed to start browser login through server", error))?;
            let start: AuthBrowserStartResource =
                decode_server_resource(start_value, "auth-browser-start")?;
            prompt_browser_login(start.authorize_url.as_str())?;
            let callback = agena_runtime::wait_for_oauth_callback_async(
                args.port,
                start.state.as_str(),
                Duration::from_secs(args.timeout_secs),
            )
            .await
            .map_err(|error| AppError::Config(format!("browser login failed: {error}")))?;
            let result_value = match kind {
                AuthLoginKindResource::OpenaiChatgpt => {
                    client
                        .finish_openai_browser_auth(
                            provider_id.as_str(),
                            callback.code,
                            start.pkce_verifier,
                            redirect_uri,
                        )
                        .await
                }
                AuthLoginKindResource::Gitlab => {
                    client
                        .finish_gitlab_browser_auth(
                            provider_id.as_str(),
                            callback.code,
                            start.pkce_verifier,
                            redirect_uri,
                        )
                        .await
                }
                AuthLoginKindResource::GithubCopilot => unreachable!("rejected above"),
            }
            .map_err(|error| {
                client_error("failed to finish browser login through server", error)
            })?;
            let result: AuthLoginResultResource =
                decode_server_resource(result_value, "auth-login-result")?;
            if !result.completed {
                return Err(AppError::Internal(
                    "server did not complete browser login".to_owned(),
                ));
            }
            println!("logged in: {provider_id}");
            return Ok(());
        }

        let kind = provider.device_login_kind.ok_or_else(|| {
            AppError::Config(format!("{provider_id} does not support device login"))
        })?;
        let enterprise_domain = args
            .enterprise_domain
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let start_value = match kind {
            AuthLoginKindResource::OpenaiChatgpt => {
                client.start_openai_device_auth(provider_id.as_str()).await
            }
            AuthLoginKindResource::GithubCopilot => {
                client
                    .start_copilot_device_auth(provider_id.as_str(), enterprise_domain.as_deref())
                    .await
            }
            AuthLoginKindResource::Gitlab => {
                return Err(AppError::Config(format!(
                    "{provider_id} does not support device login"
                )));
            }
        }
        .map_err(|error| client_error("failed to start device login through server", error))?;
        let start: AuthDeviceStartResource =
            decode_server_resource(start_value, "auth-device-start")?;
        prompt_device_login(&start)?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(args.timeout_secs);
        let interval = Duration::from_secs(start.interval_seconds.max(1));
        loop {
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Err(AppError::Config("device login timed out".to_owned()));
            }
            tokio::time::sleep(interval.min(deadline - now)).await;
            let result_value = match kind {
                AuthLoginKindResource::OpenaiChatgpt => {
                    client
                        .poll_openai_device_auth(
                            provider_id.as_str(),
                            start.device_code.clone(),
                            start.user_code.clone(),
                        )
                        .await
                }
                AuthLoginKindResource::GithubCopilot => {
                    client
                        .poll_copilot_device_auth(
                            provider_id.as_str(),
                            start.device_code.clone(),
                            enterprise_domain.as_deref(),
                        )
                        .await
                }
                AuthLoginKindResource::Gitlab => unreachable!("rejected above"),
            }
            .map_err(|error| client_error("failed to poll device login through server", error))?;
            let result: AuthLoginResultResource =
                decode_server_resource(result_value, "auth-login-result")?;
            if result.completed {
                println!("logged in: {provider_id}");
                return Ok(());
            }
        }
    }

    pub(super) async fn run_server_logout(&self, args: LogoutArgs) -> Result<(), AppError> {
        let provider_id = normalize_login_provider(args.provider_id.as_str());
        let client = connect_server_client(self).await?;
        client
            .delete_auth_provider(provider_id.as_str())
            .await
            .map_err(|error| {
                client_error("failed to remove provider credential from server", error)
            })?;
        println!("logged out: {provider_id}");
        Ok(())
    }

    pub(super) async fn render_server_plugin_status(
        &self,
        args: PluginStatusArgs,
    ) -> Result<String, AppError> {
        let client = connect_server_client(self).await?;
        let value = client
            .plugin_statuses()
            .await
            .map_err(|error| client_error("failed to read plugin status from server", error))?;
        let statuses = value
            .get("items")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .ok_or_else(|| {
                AppError::Internal("server returned an invalid plugin-status response".to_owned())
            })?;
        render_serialized(args.format, &PluginStatusOutput { statuses })
    }

    pub(super) async fn render_server_plugin_inspect(
        &self,
        args: PluginInspectArgs,
    ) -> Result<String, AppError> {
        let client = connect_server_client(self).await?;
        let value = client
            .plugin_inspect(args.plugin_id.as_str())
            .await
            .map_err(|error| client_error("failed to inspect plugin through server", error))?;
        let plugin = value.get("plugin").cloned().ok_or_else(|| {
            AppError::Internal("server returned an invalid plugin-inspect response".to_owned())
        })?;
        render_serialized(args.format, &PluginInspectOutput { plugin })
    }

    pub(super) async fn render_server_plugin_logs(
        &self,
        args: PluginLogsArgs,
    ) -> Result<String, AppError> {
        let client = connect_server_client(self).await?;
        let value = client
            .plugin_logs(args.plugin_id.as_str(), args.after_seq, args.limit)
            .await
            .map_err(|error| client_error("failed to read plugin logs from server", error))?;
        let output: PluginLogsOutput = serde_json::from_value(value).map_err(|error| {
            AppError::Internal(format!(
                "server returned an invalid plugin-log response: {error}"
            ))
        })?;
        match args.format {
            PluginLogOutputFormat::Text => Ok(format_plugin_logs_output(&output)),
            PluginLogOutputFormat::Json => serde_json::to_string_pretty(&output).map_err(|error| {
                AppError::Config(format!("failed to render json output: {error}"))
            }),
        }
    }

    pub(super) async fn render_server_debug_command(
        &self,
        command: DebugCommand,
    ) -> Result<String, AppError> {
        match command.command {
            DebugSubcommand::Session(args) => self.render_server_debug_session(args).await,
        }
    }

    async fn render_server_debug_session(
        &self,
        args: DebugSessionArgs,
    ) -> Result<String, AppError> {
        let server = ServerSessionClient::connect(self, None).await?;
        let execution = server.execution(args.session_id).await?;
        let runs = execution
            .parts
            .iter()
            .filter(|part| part.kind == "run")
            .map(|run| {
                Ok(DebugRunOutput {
                    id: run.part_id,
                    role: run.role.parse().map_err(|error| {
                        AppError::Internal(format!(
                            "server returned invalid run role `{}`: {error}",
                            run.role
                        ))
                    })?,
                    state: run.state.parse().map_err(|error| {
                        AppError::Internal(format!(
                            "server returned invalid run state `{}`: {error}",
                            run.state
                        ))
                    })?,
                    text: run_visible_text(execution.parts.as_slice(), run.part_id),
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        let output = DebugSessionOutput {
            session: session_detail(&execution),
            runs,
        };
        if args.json {
            render_serialized(OutputFormat::Json, &output)
        } else {
            Ok(format_debug_session_output(&output))
        }
    }

    pub(super) async fn render_server_provider_command(
        &self,
        command: ProviderCommand,
    ) -> Result<String, AppError> {
        let client = connect_server_client(self).await?;
        match command
            .command
            .unwrap_or(ProviderSubcommand::List(ProviderListArgs {
                format: OutputFormat::Json,
            })) {
            ProviderSubcommand::List(args) => {
                let providers = server_providers(&client).await?;
                let mut providers = providers
                    .into_iter()
                    .map(|provider| ProviderSummary {
                        defaults: ProviderDefaultsSummary {
                            adapter: provider.defaults.adapter,
                            model: provider.defaults.model,
                        },
                        provider_id: provider.provider_id,
                    })
                    .collect::<Vec<_>>();
                providers.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
                render_serialized(args.format, &ProviderListOutput { providers })
            }
            ProviderSubcommand::Models(args) => {
                let response = server_provider_models(&client, args.provider_id.as_str()).await?;
                render_serialized(
                    args.format,
                    &ProviderModelsOutput {
                        provider_id: response.provider_id,
                        models: response.models,
                    },
                )
            }
            ProviderSubcommand::Capabilities(args) => {
                let providers = server_providers(&client).await?;
                let model_ref = resolve_provider_model_target(
                    providers.as_slice(),
                    args.target.as_str(),
                    args.model.as_deref(),
                )?;
                let response =
                    server_provider_models(&client, model_ref.provider_id.as_str()).await?;
                let model = response
                    .models
                    .into_iter()
                    .find(|candidate| {
                        candidate.id == model_ref.model_id
                            && model_ref.adapter_id.as_ref().is_none_or(|adapter| {
                                candidate.adapter_id.as_ref() == Some(adapter)
                            })
                    })
                    .ok_or_else(|| {
                        AppError::Config(format!(
                            "model not found: {}/{}",
                            model_ref.provider_id, model_ref.model_id
                        ))
                    })?;
                let model_ref_text = match model_ref.adapter_id.as_deref() {
                    Some(adapter) => format!(
                        "provider={} adapter={} model={}",
                        model_ref.provider_id, adapter, model_ref.model_id
                    ),
                    None => format!("{}/{}", model_ref.provider_id, model_ref.model_id),
                };
                render_serialized(
                    args.format,
                    &ProviderCapabilitiesOutput {
                        provider_id: model_ref.provider_id,
                        model: model_ref.model_id,
                        model_ref: model_ref_text,
                        capabilities: model.capabilities,
                        metadata: model.metadata,
                    },
                )
            }
        }
    }

    pub(super) async fn render_server_permissions_command(
        &self,
        args: PermissionsArgs,
    ) -> Result<String, AppError> {
        let server = ServerSessionClient::connect(self, None).await?;
        match args
            .command
            .unwrap_or(PermissionsSubcommand::List(PermissionsListArgs {
                search: None,
                format: OutputFormat::Json,
            })) {
            PermissionsSubcommand::List(args) => {
                let rules = server
                    .list_permission_rules(
                        args.search
                            .map(|search| search.trim().to_owned())
                            .filter(|search| !search.is_empty()),
                    )
                    .await?
                    .into_iter()
                    .map(permission_rule_output)
                    .collect::<Result<Vec<_>, _>>()?;
                render_serialized(
                    args.format,
                    &PermissionsOutput {
                        count: rules.len(),
                        rules,
                    },
                )
            }
            PermissionsSubcommand::Create(args) => {
                let format = args.format;
                let params = permission_rule_params(&server, args)?;
                let result = server
                    .client
                    .command(Command::UpsertPermissionRule(params))
                    .await
                    .map_err(|error| {
                        client_error("failed to create permission rule through server", error)
                    })?;
                let CommandResult::PermissionRule(rule) = result else {
                    return Err(AppError::Internal(
                        "server returned the wrong permission-rule create result".to_owned(),
                    ));
                };
                render_serialized(format, &permission_rule_output(rule)?)
            }
            PermissionsSubcommand::Replace(args) => {
                let format = args.rule.format;
                let rule_id = args.rule_id;
                let rule = permission_rule_params(&server, args.rule)?;
                let result = server
                    .client
                    .command(Command::ReplacePermissionRule(
                        ReplacePermissionRuleParams { rule_id, rule },
                    ))
                    .await
                    .map_err(|error| {
                        client_error("failed to replace permission rule through server", error)
                    })?;
                let CommandResult::PermissionRule(rule) = result else {
                    return Err(AppError::Internal(
                        "server returned the wrong permission-rule replace result".to_owned(),
                    ));
                };
                render_serialized(format, &permission_rule_output(rule)?)
            }
            PermissionsSubcommand::Revoke(args) => {
                let result = server
                    .client
                    .command(Command::RevokePermissionRule(RevokePermissionRuleParams {
                        rule_id: args.rule_id,
                        reason: args.reason,
                    }))
                    .await
                    .map_err(|error| {
                        client_error("failed to revoke permission rule through server", error)
                    })?;
                let CommandResult::PermissionRule(rule) = result else {
                    return Err(AppError::Internal(
                        "server returned the wrong permission-rule revoke result".to_owned(),
                    ));
                };
                render_serialized(args.format, &permission_rule_output(rule)?)
            }
            PermissionsSubcommand::Reply(args) => {
                let session_id = server
                    .selected_session_id(args.session_id, args.last)
                    .await?;
                let execution = server
                    .client
                    .reply_permission(agena_api::commands::ReplyPermissionParams {
                        session_id,
                        options: RunOptions::default(),
                        reply: PermissionReply {
                            request_id: args.request_id,
                            kind: permission_reply_kind(args.kind),
                            reason: args.reason,
                            scope: args.scope.map(permission_scope),
                        },
                    })
                    .await
                    .map_err(|error| {
                        client_error(
                            "failed to reply to permission request through server",
                            error,
                        )
                    })?;
                render_serialized(
                    args.format,
                    &SessionOutput {
                        session: session_detail(&execution),
                    },
                )
            }
        }
    }

    pub(super) async fn render_server_sessions_command(
        &self,
        command: SessionsCommand,
    ) -> Result<String, AppError> {
        let server = ServerSessionClient::connect(self, None).await?;
        match command
            .command
            .unwrap_or(SessionsSubcommand::List(SessionListArgs {
                limit: 20,
                offset: 0,
                view: SessionListView::All,
                anchor_session_id: None,
                format: OutputFormat::Json,
            })) {
            SessionsSubcommand::List(args) => {
                let resources = server.list_sessions(None).await?;
                let summaries = resources
                    .into_iter()
                    .map(session_summary_from_resource)
                    .collect::<Result<Vec<_>, _>>()?;
                let summaries =
                    filter_session_summaries_by_view(summaries, args.view, args.anchor_session_id)?;
                let sessions = paginate_session_summaries(summaries, args.offset, args.limit);
                render_serialized(args.format, &SessionListOutput { sessions })
            }
            SessionsSubcommand::Export(args) => {
                let result = server
                    .client
                    .command(Command::ExportSession(
                        agena_api::commands::ExportSessionParams {
                            session_id: args.session_id,
                        },
                    ))
                    .await
                    .map_err(|error| client_error("failed to export session from server", error))?;
                let CommandResult::SessionExport { jsonl } = result else {
                    return Err(AppError::Internal(
                        "server returned the wrong session-export result".to_owned(),
                    ));
                };
                Ok(jsonl)
            }
            SessionsSubcommand::Import(args) => {
                let bundle = match args.path {
                    Some(path) => std::fs::read_to_string(&path).map_err(|error| {
                        AppError::Internal(format!(
                            "read import bundle {}: {error}",
                            path.display()
                        ))
                    })?,
                    None => {
                        let mut buffer = String::new();
                        std::io::stdin()
                            .read_to_string(&mut buffer)
                            .map_err(|error| AppError::Internal(format!("read stdin: {error}")))?;
                        buffer
                    }
                };
                let result = server
                    .client
                    .command(Command::ImportSession(ImportSessionParams {
                        jsonl: bundle,
                    }))
                    .await
                    .map_err(|error| {
                        client_error("failed to import session through server", error)
                    })?;
                let execution = expect_execution(result, "session-import")?;
                render_serialized(
                    args.format,
                    &SessionImportOutput {
                        session: session_detail(&execution),
                    },
                )
            }
            SessionsSubcommand::Tree(args) => {
                let result = server
                    .client
                    .command(Command::ListSessionTree(ListSessionTreeParams {
                        root_id: args.root_id,
                    }))
                    .await
                    .map_err(|error| {
                        client_error("failed to read session tree from server", error)
                    })?;
                let CommandResult::SessionTree(resources) = result else {
                    return Err(AppError::Internal(
                        "server returned the wrong session-tree result".to_owned(),
                    ));
                };
                let mut sessions = resources
                    .into_iter()
                    .map(session_summary_from_resource)
                    .collect::<Result<Vec<_>, _>>()?;
                if let Some(max_depth) = args.max_depth {
                    let root_depth = sessions.first().map(|first| first.depth).unwrap_or(0);
                    sessions.retain(|session| session.depth - root_depth <= max_depth);
                }
                if let Some(limit) = args.limit {
                    sessions.truncate(limit);
                }
                render_serialized(args.format, &SessionListOutput { sessions })
            }
        }
    }

    pub(super) async fn render_server_resume_command(
        &self,
        args: ResumeArgs,
    ) -> Result<String, AppError> {
        let server = ServerSessionClient::connect(self, None).await?;
        let session_id = server
            .selected_session_id(args.session_id, args.last)
            .await?;
        let execution = server.execution(session_id).await?;
        render_serialized(
            args.format,
            &SessionOutput {
                session: session_detail(&execution),
            },
        )
    }

    pub(super) async fn render_server_continue_command(
        &self,
        args: ContinueArgs,
    ) -> Result<String, AppError> {
        let server = ServerSessionClient::connect(self, None).await?;
        let session_id = server
            .selected_session_id(args.session_id, args.last)
            .await?;
        let options = server
            .run_options(
                args.model.as_deref(),
                args.temperature,
                args.max_output_tokens,
            )
            .await?;
        let execution = server
            .client
            .continue_run(session_id, options)
            .await
            .map_err(|error| client_error("failed to continue session through server", error))?;
        render_serialized(
            args.format,
            &SessionOutput {
                session: session_detail(&execution),
            },
        )
    }

    pub(super) async fn render_server_cost_command(
        &self,
        args: CostArgs,
    ) -> Result<String, AppError> {
        let server = ServerSessionClient::connect(self, None).await?;
        let session_id = server
            .selected_session_id(args.session_id, args.last)
            .await?;
        let execution = server.execution(session_id).await?;
        let summary = server
            .client
            .session_cost_summary(session_id)
            .await
            .map_err(|error| client_error("failed to read session cost from server", error))?;
        render_serialized(
            args.format,
            &CostOutput {
                session: session_detail(&execution),
                summary,
            },
        )
    }

    pub(super) async fn render_server_usage_command(
        &self,
        args: UsageArgs,
    ) -> Result<String, AppError> {
        let server = ServerSessionClient::connect(self, None).await?;
        let custom_range = args.from.is_some() || args.to.is_some();
        let query = usage_stats_query_from_args(&args)?;
        let output = server
            .client
            .usage_stats(
                query.period,
                custom_range.then_some(query.from).flatten(),
                custom_range.then_some(query.to).flatten(),
                query.provider_ids.as_slice(),
                query.model_ids.as_slice(),
                query.session_ids.as_slice(),
                query.include_subagents,
                query.timezone_offset_minutes,
            )
            .await
            .map_err(|error| client_error("failed to read usage from server", error))?;
        render_serialized(args.format, &output)
    }

    pub(super) async fn render_server_exec_command(
        &self,
        args: ExecArgs,
    ) -> Result<String, AppError> {
        let title = title_from_prompt(args.prompt.as_str());
        self.render_server_prompt_command(
            args.workspace.as_deref(),
            args.prompt.as_str(),
            title,
            args.model.as_deref(),
            args.temperature,
            args.max_output_tokens,
            args.json,
        )
        .await
    }

    pub(super) async fn render_server_review_command(
        &self,
        args: ReviewArgs,
    ) -> Result<String, AppError> {
        let prompt = review_prompt(args.base.as_str());
        self.render_server_prompt_command(
            args.workspace.as_deref(),
            prompt.as_str(),
            format!("Review changes against {}", args.base),
            args.model.as_deref(),
            args.temperature,
            args.max_output_tokens,
            args.json,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn render_server_prompt_command(
        &self,
        workspace: Option<&Path>,
        prompt: &str,
        title: String,
        model: Option<&str>,
        temperature: Option<f32>,
        max_output_tokens: Option<u32>,
        json: bool,
    ) -> Result<String, AppError> {
        let server = ServerSessionClient::connect(self, workspace).await?;
        let options = server
            .run_options(model, temperature, max_output_tokens)
            .await?;
        let session = server
            .client
            .create_session(server.workspace_id, title, None)
            .await
            .map_err(|error| client_error("failed to create session through server", error))?;
        let execution = server
            .client
            .submit_message(SubmitRunParams {
                session_id: session.id,
                options,
                document: agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
                    text: prompt.to_owned(),
                }]),
            })
            .await
            .map_err(|error| client_error("failed to submit run through server", error))?;
        if execution.session.state.is_attention() {
            return Err(AppError::Config(
                "command requires session interaction or recovery".to_owned(),
            ));
        }
        let text = last_assistant_text(execution.parts.as_slice()).unwrap_or_default();
        if json {
            render_serialized(
                OutputFormat::Json,
                &ExecOutput {
                    session: session_detail(&execution),
                    text,
                },
            )
        } else {
            Ok(text)
        }
    }

    pub(super) async fn render_server_fork_command(
        &self,
        args: ForkArgs,
    ) -> Result<String, AppError> {
        let server = ServerSessionClient::connect(self, None).await?;
        let result = server
            .client
            .command(Command::ForkSession(ForkSessionParams {
                session_id: args.session_id,
                at_message_id: args.at_message,
                title: args.title,
            }))
            .await
            .map_err(|error| client_error("failed to fork session through server", error))?;
        let execution = expect_execution(result, "session-fork")?;
        render_serialized(
            args.format,
            &SessionForkOutput {
                source_session_id: args.session_id,
                forked: session_detail(&execution),
            },
        )
    }
}

fn auth_summary_from_value(value: &serde_json::Value) -> Result<AuthSummary, AppError> {
    let provider_id = value
        .get("provider_id")
        .and_then(serde_json::Value::as_str)
        .filter(|provider_id| !provider_id.is_empty())
        .ok_or_else(|| {
            AppError::Internal("server returned an auth provider without an id".to_owned())
        })?
        .to_owned();
    let string_field = |name: &str| {
        value
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    let expires_at_ms = value
        .get("expires_at")
        .and_then(serde_json::Value::as_str)
        .and_then(|timestamp| chrono::DateTime::parse_from_rfc3339(timestamp).ok())
        .map(|timestamp| timestamp.timestamp_millis());
    Ok(AuthSummary {
        provider_id,
        kind: string_field("credential_type").unwrap_or_else(|| "none".to_owned()),
        account_id: string_field("account_id"),
        enterprise_url: string_field("enterprise_url"),
        username: string_field("username"),
        display_name: string_field("display_name"),
        email: string_field("email"),
        issuer: string_field("credential_issuer"),
        expires_at_ms,
    })
}

async fn server_providers(client: &AgenaClient) -> Result<Vec<ProviderSummaryResource>, AppError> {
    let response = client
        .query(Query::ListProviders)
        .await
        .map_err(|error| client_error("failed to read the server's provider catalog", error))?;
    let QueryResult::Providers(providers) = response else {
        return Err(AppError::Internal(
            "server returned the wrong provider-list result".to_owned(),
        ));
    };
    Ok(providers)
}

async fn server_provider_models(
    client: &AgenaClient,
    provider_id: &str,
) -> Result<agena_api::resource::ProviderModelsResponse, AppError> {
    let response = client
        .query(Query::ListProviderModels(
            agena_api::queries::ListProviderModelsParams {
                provider_id: provider_id.to_owned(),
            },
        ))
        .await
        .map_err(|error| client_error("failed to read provider models from server", error))?;
    let QueryResult::ProviderModels(models) = response else {
        return Err(AppError::Internal(
            "server returned the wrong provider-model result".to_owned(),
        ));
    };
    Ok(models)
}

fn resolve_provider_model_target(
    providers: &[ProviderSummaryResource],
    target: &str,
    model: Option<&str>,
) -> Result<WireModelRef, AppError> {
    let target = target.trim();
    if target.is_empty() {
        return Err(AppError::Config(
            "provider or model reference cannot be empty".to_owned(),
        ));
    }
    if target.contains('/') {
        if model.is_some_and(|model| !model.trim().is_empty()) {
            return Err(AppError::Config(format!(
                "model reference `{target}` already includes a model; omit --model"
            )));
        }
        return resolve_model_target(providers, target);
    }
    let provider = providers
        .iter()
        .find(|provider| provider.provider_id == target)
        .ok_or_else(|| AppError::Config(format!("provider not found: {target}")))?;
    let model_id = model
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .unwrap_or(provider.defaults.model.as_str())
        .to_owned();
    Ok(WireModelRef {
        provider_id: provider.provider_id.clone(),
        adapter_id: provider.defaults.adapter.clone(),
        model_id,
    })
}

fn permission_rule_params(
    server: &ServerSessionClient,
    args: PermissionsWriteArgs,
) -> Result<UpsertPermissionRuleParams, AppError> {
    if matches!(args.scope, PermissionScopeArg::Session) && args.session_id.is_none() {
        return Err(AppError::Config(
            "session scope requires --session-id".to_owned(),
        ));
    }
    let action_key = args
        .action_key
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let tool_name = args
        .tool_name
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let network_target = args
        .network_target
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let network_host = args
        .network_host
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let path_access_kind = args
        .path_access_kind
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let target_path = args
        .target_path
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let subject_kind = if action_key.is_some() {
        None
    } else if tool_name.is_some() {
        Some("tool".to_owned())
    } else if network_target.is_some() || network_host.is_some() {
        Some("network_access".to_owned())
    } else if path_access_kind.is_some() && target_path.is_some() {
        Some("path_access".to_owned())
    } else {
        return Err(AppError::Config(
            "permission rule requires either --action-key, --tool-name, network fields, or both --path-access-kind and --target-path"
                .to_owned(),
        ));
    };
    let workspace_root = args
        .workspace_root
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            (subject_kind.as_deref() == Some("path_access"))
                .then(|| server.workspace_root.to_string_lossy().into_owned())
        });
    Ok(UpsertPermissionRuleParams {
        action_key,
        subject_kind,
        tool_name,
        qualifier: args
            .qualifier
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty()),
        path_access_kind,
        workspace_root,
        target_path,
        network_target,
        network_host,
        network_port: args.network_port,
        scope: Some(permission_scope_name(args.scope).to_owned()),
        session_id: args.session_id,
        mode: permission_mode(args.rule_mode),
    })
}

const fn permission_mode(mode: PermissionModeArg) -> WirePermissionMode {
    match mode {
        PermissionModeArg::Allow => WirePermissionMode::Allow,
        PermissionModeArg::Auto => WirePermissionMode::Auto,
        PermissionModeArg::Ask => WirePermissionMode::Ask,
        PermissionModeArg::Deny => WirePermissionMode::Deny,
    }
}

const fn permission_scope_name(scope: PermissionScopeArg) -> &'static str {
    match scope {
        PermissionScopeArg::Session => "session",
        PermissionScopeArg::Workspace => "workspace",
        PermissionScopeArg::Global => "global",
    }
}

const fn permission_scope(scope: PermissionScopeArg) -> WirePermissionScope {
    match scope {
        PermissionScopeArg::Session => WirePermissionScope::Session,
        PermissionScopeArg::Workspace => WirePermissionScope::Workspace,
        PermissionScopeArg::Global => WirePermissionScope::Global,
    }
}

const fn permission_reply_kind(kind: PermissionReplyKindArg) -> WirePermissionReplyKind {
    match kind {
        PermissionReplyKindArg::AllowOnce => WirePermissionReplyKind::AllowOnce,
        PermissionReplyKindArg::AllowAlways => WirePermissionReplyKind::AllowAlways,
        PermissionReplyKindArg::DenyOnce => WirePermissionReplyKind::DenyOnce,
        PermissionReplyKindArg::DenyAlways => WirePermissionReplyKind::DenyAlways,
    }
}

const MCP_PLUGIN_SETTINGS_PATH: &str = r#"plugins.list."agena.mcp""#;

enum McpConfigMutation {
    Add {
        server: String,
        config: serde_json::Value,
        force: bool,
    },
    Remove {
        server: String,
    },
    SetEnabled(bool),
}

const fn mcp_config_layer_name(layer: McpConfigLayerArg) -> &'static str {
    match layer {
        McpConfigLayerArg::Global => "global",
        McpConfigLayerArg::Workspace => "workspace",
    }
}

fn normalized_mcp_server_name(server: &str) -> Result<String, AppError> {
    let server = server.trim();
    if server.is_empty() {
        return Err(AppError::Config(
            "MCP server name must not be empty".to_owned(),
        ));
    }
    Ok(server.to_owned())
}

fn mcp_server_config_value(args: &McpAddArgs) -> Result<serde_json::Value, AppError> {
    match (args.url.as_deref(), args.command.as_deref()) {
        (Some(url), None) => {
            if !args.args.is_empty() || !args.env.is_empty() || args.cwd.is_some() {
                return Err(AppError::Config(
                    "--arg, --env, and --cwd are valid only with --command".to_owned(),
                ));
            }
            let url = url::Url::parse(url.trim())
                .map_err(|error| AppError::Config(format!("invalid MCP HTTP URL: {error}")))?;
            if !matches!(url.scheme(), "http" | "https") {
                return Err(AppError::Config(
                    "MCP HTTP URL must use http or https".to_owned(),
                ));
            }
            if !url.username().is_empty() || url.password().is_some() {
                return Err(AppError::Config(
                    "MCP HTTP URL must not embed credentials; use --auth bearer-from-env or a configured MCP client credential store"
                        .to_owned(),
                ));
            }
            let headers = parse_mcp_key_value_pairs(&args.headers, "--header")?;
            if headers
                .keys()
                .any(|name| name.eq_ignore_ascii_case("authorization"))
            {
                return Err(AppError::Config(
                    "Authorization headers are not accepted by mcp add; use --auth bearer-from-store or --auth bearer-from-env"
                        .to_owned(),
                ));
            }
            let auth = match args.auth {
                McpHttpAuthArg::None => {
                    if args.auth_env.is_some() {
                        return Err(AppError::Config(
                            "--auth-env requires --auth bearer-from-env".to_owned(),
                        ));
                    }
                    None
                }
                McpHttpAuthArg::BearerFromStore => {
                    if args.auth_env.is_some() {
                        return Err(AppError::Config(
                            "--auth-env is only valid with --auth bearer-from-env".to_owned(),
                        ));
                    }
                    Some(serde_json::json!({ "kind": "bearer_from_store" }))
                }
                McpHttpAuthArg::BearerFromEnv => {
                    let env = args
                        .auth_env
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| {
                            AppError::Config(
                                "--auth bearer-from-env requires --auth-env NAME".to_owned(),
                            )
                        })?;
                    Some(serde_json::json!({
                        "kind": "bearer_from_env",
                        "env": env,
                    }))
                }
                McpHttpAuthArg::OAuth => {
                    if args.auth_env.is_some() {
                        return Err(AppError::Config(
                            "--auth-env is not valid with --auth oauth".to_owned(),
                        ));
                    }
                    let scopes = args
                        .scopes
                        .iter()
                        .map(|scope| scope.trim())
                        .filter(|scope| !scope.is_empty())
                        .collect::<Vec<_>>();
                    Some(serde_json::json!({ "kind": "oauth", "scopes": scopes }))
                }
            };
            if !matches!(args.auth, McpHttpAuthArg::OAuth) && !args.scopes.is_empty() {
                return Err(AppError::Config("--scope requires --auth oauth".to_owned()));
            }
            let mut value = serde_json::json!({
                "transport": "http",
                "endpoint": {
                    "url": url.to_string(),
                    "headers": headers,
                },
            });
            if !args.include_tools.is_empty() || !args.exclude_tools.is_empty() {
                value
                    .as_object_mut()
                    .expect("MCP HTTP config is an object")
                    .insert(
                        "tools".to_owned(),
                        serde_json::json!({
                            "include": args.include_tools,
                            "exclude": args.exclude_tools,
                        }),
                    );
            }
            if let Some(auth) = auth {
                value
                    .as_object_mut()
                    .expect("MCP HTTP config is an object")
                    .insert("auth".to_owned(), auth);
            }
            Ok(value)
        }
        (None, Some(command)) => {
            if !args.headers.is_empty()
                || !matches!(args.auth, McpHttpAuthArg::None)
                || !args.scopes.is_empty()
                || !args.include_tools.is_empty()
                || !args.exclude_tools.is_empty()
                || args.auth_env.is_some()
            {
                return Err(AppError::Config(
                    "--header, --auth, --scope, --include-tool, --exclude-tool, and --auth-env are valid only with --url"
                        .to_owned(),
                ));
            }
            let command = command.trim();
            if command.is_empty() {
                return Err(AppError::Config(
                    "MCP stdio command must not be empty".to_owned(),
                ));
            }
            let env = parse_mcp_key_value_pairs(&args.env, "--env")?;
            Ok(serde_json::json!({
                "transport": "stdio",
                "process": {
                    "command": command,
                    "args": args.args,
                    "env": env,
                    "cwd": args.cwd,
                },
            }))
        }
        (Some(_), Some(_)) | (None, None) => Err(AppError::Config(
            "mcp add requires exactly one of --url or --command".to_owned(),
        )),
    }
}

fn parse_mcp_key_value_pairs(
    pairs: &[String],
    option_name: &str,
) -> Result<BTreeMap<String, String>, AppError> {
    let mut values = BTreeMap::new();
    for pair in pairs {
        let (name, value) = pair.split_once('=').ok_or_else(|| {
            AppError::Config(format!("{option_name} requires KEY=VALUE, got `{pair}`"))
        })?;
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::Config(format!(
                "{option_name} key must not be empty"
            )));
        }
        if values.insert(name.to_owned(), value.to_owned()).is_some() {
            return Err(AppError::Config(format!(
                "{option_name} contains duplicate key `{name}`"
            )));
        }
    }
    Ok(values)
}

fn mcp_plugin_record(
    current: serde_json::Value,
) -> Result<serde_json::Map<String, serde_json::Value>, AppError> {
    let mut record = match current {
        serde_json::Value::Null => serde_json::json!({
            "enabled": true,
            "package": { "kind": "static" },
            "config": {},
        })
        .as_object()
        .expect("MCP default plugin record is an object")
        .clone(),
        serde_json::Value::Object(record) => record,
        _ => {
            return Err(AppError::Config(
                "plugins.list.\"agena.mcp\" must be an object".to_owned(),
            ));
        }
    };

    match record.get("package") {
        Some(serde_json::Value::Object(package))
            if package.get("kind").and_then(serde_json::Value::as_str) == Some("static") => {}
        Some(_) => {
            return Err(AppError::Config(
                "plugins.list.\"agena.mcp\" must retain package.kind=static".to_owned(),
            ));
        }
        None => {
            record.insert(
                "package".to_owned(),
                serde_json::json!({ "kind": "static" }),
            );
        }
    }
    match record.get_mut("config") {
        Some(value) if value.is_null() => *value = serde_json::Value::Object(Default::default()),
        Some(serde_json::Value::Object(_)) => {}
        Some(_) => {
            return Err(AppError::Config(
                "plugins.list.\"agena.mcp\".config must be an object".to_owned(),
            ));
        }
        None => {
            record.insert(
                "config".to_owned(),
                serde_json::Value::Object(Default::default()),
            );
        }
    }
    Ok(record)
}

fn apply_mcp_config_mutation(
    record: &mut serde_json::Map<String, serde_json::Value>,
    mutation: McpConfigMutation,
) -> Result<(), AppError> {
    match mutation {
        McpConfigMutation::SetEnabled(enabled) => {
            record.insert("enabled".to_owned(), serde_json::Value::Bool(enabled));
        }
        McpConfigMutation::Add {
            server,
            config,
            force,
        } => {
            let servers = mcp_servers_mut(record)?;
            if servers.contains_key(server.as_str()) && !force {
                return Err(AppError::Config(format!(
                    "MCP server `{server}` already exists; pass --force to replace it"
                )));
            }
            servers.insert(server, config);
        }
        McpConfigMutation::Remove { server } => {
            let servers = mcp_servers_mut(record)?;
            if servers.remove(server.as_str()).is_none() {
                return Err(AppError::Config(format!(
                    "MCP server not configured in the selected layer: {server}"
                )));
            }
        }
    }
    Ok(())
}

fn mcp_servers_mut(
    record: &mut serde_json::Map<String, serde_json::Value>,
) -> Result<&mut serde_json::Map<String, serde_json::Value>, AppError> {
    let config = record
        .get_mut("config")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| AppError::Config("MCP plugin config must be an object".to_owned()))?;
    let servers = config
        .entry("servers".to_owned())
        .or_insert_with(|| serde_json::Value::Object(Default::default()));
    if servers.is_null() {
        *servers = serde_json::Value::Object(Default::default());
    }
    servers
        .as_object_mut()
        .ok_or_else(|| AppError::Config("MCP servers must be an object".to_owned()))
}

fn client_error(context: &str, error: ClientError) -> AppError {
    AppError::Config(format!("{context}: {error}"))
}

fn resolve_model_target(
    providers: &[ProviderSummaryResource],
    target: &str,
) -> Result<WireModelRef, AppError> {
    let target = target.trim();
    if target.is_empty() {
        return Err(AppError::Config(
            "provider or model reference cannot be empty".to_owned(),
        ));
    }
    if let Some((provider_id, model_id)) = target.split_once('/') {
        if provider_id.trim().is_empty() || model_id.trim().is_empty() {
            return Err(AppError::Config(format!(
                "invalid model reference `{target}`; expected provider/model"
            )));
        }
        let adapter_id = providers
            .iter()
            .find(|provider| provider.provider_id == provider_id.trim())
            .and_then(|provider| provider.defaults.adapter.clone());
        return Ok(WireModelRef {
            provider_id: provider_id.trim().to_owned(),
            adapter_id,
            model_id: model_id.trim().to_owned(),
        });
    }
    let provider = providers
        .iter()
        .find(|provider| provider.provider_id == target)
        .ok_or_else(|| AppError::Config(format!("provider not found: {target}")))?;
    Ok(WireModelRef {
        provider_id: provider.provider_id.clone(),
        adapter_id: provider.defaults.adapter.clone(),
        model_id: provider.defaults.model.clone(),
    })
}

fn session_summary_from_resource(resource: SessionResource) -> Result<SessionSummary, AppError> {
    serde_json::from_value(serde_json::to_value(resource)?).map_err(AppError::from)
}

fn session_detail(execution: &SessionExecutionResource) -> SessionDetail {
    SessionDetail {
        id: execution.session.id,
        parent_id: execution.session.parent_id,
        workspace_id: execution.session.workspace_id,
        title: execution.session.title.clone(),
        version: execution.session.version,
        created_at: execution.session.created_at,
        updated_at: execution.session.updated_at,
        message_count: usize::try_from(execution.session.message_count).unwrap_or(usize::MAX),
        status: workflow_state_from_wire(execution.session.state.workflow_state()),
        latest_event_seq: execution.latest_event_seq,
    }
}

fn expect_execution(
    result: CommandResult,
    operation: &str,
) -> Result<SessionExecutionResource, AppError> {
    let CommandResult::Execution(execution) = result else {
        return Err(AppError::Internal(format!(
            "server returned the wrong {operation} result"
        )));
    };
    Ok(execution)
}

const fn workflow_state_from_wire(state: agena_api::resource::WorkflowState) -> WorkflowState {
    match state {
        agena_api::resource::WorkflowState::Quiescent => WorkflowState::Quiescent,
        agena_api::resource::WorkflowState::ToolPending => WorkflowState::ToolPending,
        agena_api::resource::WorkflowState::AwaitingInteraction => {
            WorkflowState::AwaitingInteraction
        }
    }
}

fn last_assistant_text(parts: &[SessionTranscriptPart]) -> Option<String> {
    let run_id = parts
        .iter()
        .rev()
        .find(|part| part.kind == "run" && part.role == "assistant")
        .map(|part| part.part_id)?;
    let text = run_visible_text(parts, run_id);
    (!text.trim().is_empty()).then_some(text)
}

fn run_visible_text(parts: &[SessionTranscriptPart], run_id: i64) -> String {
    parts
        .iter()
        .filter(|part| part.part_id != run_id && part.run_id == Some(run_id))
        .filter_map(|part| {
            part.content
                .get("text")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .or_else(|| part.content.as_str().map(str::to_owned))
                .or_else(|| part.summary.clone())
        })
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_api::resource::ProviderDefaultsResource;

    #[test]
    fn workspace_guard_accepts_the_same_canonical_path_and_rejects_its_parent() {
        let current = std::env::current_dir().expect("current test directory");
        let canonical = std::fs::canonicalize(&current).expect("canonical test directory");
        ensure_server_workspace_matches(canonical.to_string_lossy().as_ref(), current.as_path())
            .expect("same workspace must pass");

        let parent = canonical
            .parent()
            .expect("test directory has a parent directory");
        let error = ensure_server_workspace_matches(canonical.to_string_lossy().as_ref(), parent)
            .expect_err("different workspace must fail");
        assert!(error.to_string().contains("refusing to operate"));
    }

    fn operator_tool(
        name: &str,
        interactive: bool,
        plugin_id: Option<&str>,
    ) -> OperatorToolResource {
        OperatorToolResource {
            name: name.to_owned(),
            summary: None,
            before_help: None,
            after_help: None,
            input_schema: serde_json::json!({"type": "object"}),
            interactive,
            plugin_id: plugin_id.map(str::to_owned),
        }
    }

    #[test]
    fn mcp_catalog_hides_interactive_tools_and_provider_plugins() {
        assert!(mcp_tool_is_exposed(&operator_tool(
            "fs.stat",
            false,
            Some("agena.fs")
        )));
        assert!(!mcp_tool_is_exposed(&operator_tool(
            "interaction.ask",
            true,
            Some("agena.interaction")
        )));

        for plugin_id in HIDDEN_MCP_PLUGIN_IDS {
            assert!(!mcp_tool_is_exposed(&operator_tool(
                &format!("{plugin_id}.tool"),
                false,
                Some(plugin_id),
            )));
        }
    }

    #[test]
    fn mcp_call_gate_rejects_hidden_tools_even_when_the_name_is_guessed() {
        let tools = vec![
            operator_tool("fs.stat", false, Some("agena.fs")),
            operator_tool("interaction.ask", true, Some("agena.interaction")),
            operator_tool("chatgpt.web_search", false, Some("agena.chatgpt")),
        ];

        assert!(mcp_tool_is_callable(&tools, "fs.stat"));
        assert!(!mcp_tool_is_callable(&tools, "interaction.ask"));
        assert!(!mcp_tool_is_callable(&tools, "chatgpt.web_search"));
        assert!(!mcp_tool_is_callable(&tools, "agena.chatgpt.web_search"));
        assert!(!mcp_tool_is_callable(&tools, "unknown.tool"));
    }

    #[test]
    fn mcp_catalog_uses_compact_name_fallback_for_older_servers() {
        assert!(!mcp_tool_is_exposed(&operator_tool(
            "chatgpt.web_search",
            false,
            None,
        )));
        assert!(!mcp_tool_is_exposed(&operator_tool(
            "agena.gemini.google_search",
            false,
            None,
        )));
        assert!(mcp_tool_is_exposed(&operator_tool(
            "custom.chatgpt_helper",
            false,
            None,
        )));
    }

    #[test]
    fn cli_model_targets_are_resolved_from_public_provider_summaries() {
        let providers = vec![ProviderSummaryResource {
            provider_id: "example".to_owned(),
            defaults: ProviderDefaultsResource {
                adapter: Some("openai".to_owned()),
                model: "default-model".to_owned(),
            },
            adapters: Vec::new(),
        }];
        assert_eq!(
            resolve_model_target(&providers, "example").expect("provider default"),
            WireModelRef {
                provider_id: "example".to_owned(),
                adapter_id: Some("openai".to_owned()),
                model_id: "default-model".to_owned(),
            }
        );
        assert_eq!(
            resolve_model_target(&providers, "example/other").expect("qualified model"),
            WireModelRef {
                provider_id: "example".to_owned(),
                adapter_id: Some("openai".to_owned()),
                model_id: "other".to_owned(),
            }
        );
    }

    #[test]
    fn mcp_http_add_never_serializes_a_bearer_credential() {
        let args = McpAddArgs {
            server: "example".to_owned(),
            url: Some("https://mcp.example.test/api".to_owned()),
            command: None,
            args: Vec::new(),
            env: Vec::new(),
            cwd: None,
            headers: vec!["X-Client=Agena".to_owned()],
            auth: McpHttpAuthArg::BearerFromStore,
            scopes: Vec::new(),
            include_tools: Vec::new(),
            exclude_tools: Vec::new(),
            auth_env: None,
            layer: McpConfigLayerArg::Global,
            force: false,
            dry_run: false,
            no_reload: false,
            format: OutputFormat::Json,
        };

        let value = mcp_server_config_value(&args).expect("HTTP MCP configuration");
        assert_eq!(
            value["auth"],
            serde_json::json!({"kind": "bearer_from_store"})
        );
        assert!(!value.to_string().contains("Bearer "));
    }

    #[test]
    fn mcp_http_add_rejects_embedded_and_header_credentials() {
        let args = McpAddArgs {
            server: "example".to_owned(),
            url: Some("https://token@example.test/mcp".to_owned()),
            command: None,
            args: Vec::new(),
            env: Vec::new(),
            cwd: None,
            headers: Vec::new(),
            auth: McpHttpAuthArg::None,
            scopes: Vec::new(),
            include_tools: Vec::new(),
            exclude_tools: Vec::new(),
            auth_env: None,
            layer: McpConfigLayerArg::Global,
            force: false,
            dry_run: false,
            no_reload: false,
            format: OutputFormat::Json,
        };
        assert!(
            mcp_server_config_value(&args)
                .expect_err("embedded credential must fail")
                .to_string()
                .contains("must not embed credentials")
        );

        let args = McpAddArgs {
            url: Some("https://mcp.example.test".to_owned()),
            headers: vec!["Authorization=Bearer secret".to_owned()],
            ..args
        };
        assert!(
            mcp_server_config_value(&args)
                .expect_err("Authorization header must fail")
                .to_string()
                .contains("Authorization headers")
        );
    }

    #[test]
    fn mcp_config_mutation_retains_the_static_plugin_contract() {
        let mut record = mcp_plugin_record(serde_json::Value::Null).expect("new MCP record");
        apply_mcp_config_mutation(
            &mut record,
            McpConfigMutation::Add {
                server: "local".to_owned(),
                config: serde_json::json!({
                    "transport": "stdio",
                    "process": {
                        "command": "node",
                        "args": ["server.js"],
                        "env": {"MODE": "test"},
                        "cwd": "/tmp/workspace"
                    }
                }),
                force: false,
            },
        )
        .expect("add MCP server");
        assert_eq!(record["package"]["kind"], "static");
        assert_eq!(record["config"]["servers"]["local"]["transport"], "stdio");

        apply_mcp_config_mutation(
            &mut record,
            McpConfigMutation::Remove {
                server: "local".to_owned(),
            },
        )
        .expect("remove MCP server");
        assert!(
            record["config"]["servers"]
                .as_object()
                .expect("servers object")
                .is_empty()
        );
    }

    #[test]
    fn cli_text_projection_uses_the_latest_assistant_run() {
        let part =
            |part_id, kind: &str, role: &str, run_id, text: Option<&str>| SessionTranscriptPart {
                part_id,
                kind: kind.to_owned(),
                role: role.to_owned(),
                state: "completed".to_owned(),
                content: text
                    .map(|text| serde_json::json!({ "text": text }))
                    .unwrap_or_else(|| serde_json::json!({})),
                summary: None,
                created_at_ms: part_id,
                parent_part_id: None,
                run_id,
            };
        let parts = vec![
            part(1, "run", "assistant", Some(1), None),
            part(2, "text", "assistant", Some(1), Some("old")),
            part(3, "run", "user", Some(3), None),
            part(4, "text", "user", Some(3), Some("question")),
            part(5, "run", "assistant", Some(5), None),
            part(6, "text", "assistant", Some(5), Some("new")),
        ];
        assert_eq!(last_assistant_text(&parts).as_deref(), Some("new"));
    }
}
