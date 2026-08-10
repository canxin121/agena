use super::{
    AgenaCli, AgenaCommand, AppError, ApplyArgs, AuthCommand, AuthLoginKind, CommitArgs,
    CompletionArgs, ConfigCommand, ConfigResolveArgs, ConfigSubcommand, ContinueArgs, CostArgs,
    DebugCommand, DiagnosticsArgs, Duration, ExecArgs, ForkArgs, GitArgs, InspectArgs, LoginArgs,
    LogoutArgs, McpAddArgs, McpCommand, McpConfigLayerArg, McpCredentialStoreArg, McpGetArgs,
    McpHttpAuthArg, McpLoginArgs, McpLogoutArgs, McpPluginToggleArgs, McpReconnectArgs,
    McpRemoveArgs, McpServerArgs, McpStatusArgs, McpSubcommand, MemoryCommand, OutputFormat,
    PermissionsArgs, PluginCommand, PluginInspectOutput, PluginLogOutputFormat, PluginLogsOutput,
    PluginStatusOutput, PluginSubcommand, PrArgs, ProviderCommand, ResumeArgs, ReviewArgs,
    SessionsCommand, SnapshotArgs, UsageArgs, browser_login_redirect_uri, complete_polled_login,
    format_plugin_logs_output, normalize_login_provider, prompt_browser_login, prompt_device_login,
    render_completion_command, render_plugin_validate_output, render_serialized,
    validate_plugin_target,
};
use std::{collections::BTreeMap, io::Read as _, time::Duration as StdDuration};
impl AgenaCli {
    pub async fn run_command(self) -> Result<(), AppError> {
        match self.command.clone() {
            Some(AgenaCommand::RpcServer(_))
            | Some(AgenaCommand::Server(_))
            | Some(AgenaCommand::Tui(_))
            | None => {
                unreachable!("top-level launch mode must be selected before command dispatch")
            }
            Some(AgenaCommand::Apply(args)) => self.run_apply(args).await,
            Some(AgenaCommand::Auth(command)) => self.run_auth(command).await,
            Some(AgenaCommand::Completion(args)) => self.run_completion(args),
            Some(AgenaCommand::Config(command)) => self.run_config(command).await,
            Some(AgenaCommand::Continue(args)) => self.run_continue(args).await,
            Some(AgenaCommand::Commit(args)) => self.run_commit(args).await,
            Some(AgenaCommand::Pr(args)) => self.run_pr(args).await,
            Some(AgenaCommand::Debug(command)) => self.run_debug(command).await,
            Some(AgenaCommand::Diagnostics(args)) => self.run_diagnostics(args).await,
            Some(AgenaCommand::Exec(args)) => self.run_exec(args).await,
            Some(AgenaCommand::Fork(args)) => self.run_fork(args).await,
            Some(AgenaCommand::Cost(args)) => self.run_cost(args).await,
            Some(AgenaCommand::Usage(args)) => self.run_usage(args).await,
            Some(AgenaCommand::Git(args)) => self.run_git(args).await,
            Some(AgenaCommand::Inspect(args)) => self.run_inspect(args),
            Some(AgenaCommand::Login(args)) => self.run_login(args).await,
            Some(AgenaCommand::Logout(args)) => self.run_logout(args).await,
            Some(AgenaCommand::Memory(command)) => self.run_memory(command).await,
            Some(AgenaCommand::Mcp(command)) => self.run_mcp(command).await,
            Some(AgenaCommand::McpServer(args)) => self.run_mcp_server(args).await,
            Some(AgenaCommand::Permissions(args)) => self.run_permissions(args).await,
            Some(AgenaCommand::Provider(command)) => self.run_provider(command).await,
            Some(AgenaCommand::Plugin(command)) => self.run_plugin(command).await,
            Some(AgenaCommand::Resume(args)) => self.run_resume(args).await,
            Some(AgenaCommand::Review(args)) => self.run_review(args).await,
            Some(AgenaCommand::Sessions(command)) => self.run_sessions(command).await,
            Some(AgenaCommand::Snapshot(args)) => self.run_snapshot(args).await,
        }
    }

    pub(super) async fn run_apply(self, args: ApplyArgs) -> Result<(), AppError> {
        let output = self.render_apply_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) fn run_inspect(self, args: InspectArgs) -> Result<(), AppError> {
        if args.identity_snapshot {
            print!(
                "{}",
                agena_bundled_plugins::bundled_capability_identity_snapshot_json()
            );
            return Ok(());
        }
        if args.tools_reference {
            print!(
                "{}",
                agena_bundled_plugins::bundled_tools_markdown_reference()
            );
            return Ok(());
        }
        let manifest = agena_bundled_plugins::bundled_capability_manifest();
        println!(
            "{}",
            serde_json::to_string_pretty(&manifest).map_err(|error| AppError::Config(format!(
                "serialize capability manifest: {error}"
            )))?
        );
        Ok(())
    }

    pub(super) async fn run_plugin(self, command: PluginCommand) -> Result<(), AppError> {
        use agena_plugin_marketplace::{
            InstallRequest, MarketplaceCache, MarketplaceClient, RegistrySpec, default_cache_root,
        };

        let cache = MarketplaceCache::new(default_cache_root());
        let client = MarketplaceClient::new(cache, std::collections::BTreeMap::new());

        match command.command {
            PluginSubcommand::Status(args) => {
                let runtime = self.session_runtime().await?;
                let plugins = runtime.application_services().plugins;
                let output = PluginStatusOutput {
                    statuses: plugins.plugin_statuses(),
                };
                println!("{}", render_serialized(args.format, &output)?);
                Ok(())
            }
            PluginSubcommand::Inspect(args) => {
                let runtime = self.session_runtime().await?;
                let plugins = runtime.application_services().plugins;
                let plugin = plugins
                    .plugin_inspect(args.plugin_id.as_str())
                    .ok_or_else(|| {
                        AppError::Config(format!("plugin not found: {}", args.plugin_id))
                    })?;
                println!(
                    "{}",
                    render_serialized(args.format, &PluginInspectOutput { plugin })?
                );
                Ok(())
            }
            PluginSubcommand::Logs(args) => {
                let runtime = self.session_runtime().await?;
                let plugins = runtime.application_services().plugins;
                if plugins.plugin_status(args.plugin_id.as_str()).is_none() {
                    return Err(AppError::Config(format!(
                        "plugin not found: {}",
                        args.plugin_id
                    )));
                }
                let output = PluginLogsOutput {
                    plugin_id: args.plugin_id.clone(),
                    logs: plugins.plugin_logs(args.plugin_id.as_str(), args.after_seq, args.limit),
                };
                match args.format {
                    PluginLogOutputFormat::Text => {
                        println!("{}", format_plugin_logs_output(&output))
                    }
                    PluginLogOutputFormat::Json => println!(
                        "{}",
                        serde_json::to_string_pretty(&output).map_err(|err| AppError::Config(
                            format!("failed to render json output: {err}")
                        ))?
                    ),
                }
                Ok(())
            }
            PluginSubcommand::Validate(args) => {
                let output = validate_plugin_target(args.path.as_path(), args.strict)?;
                let has_errors = !output.errors.is_empty();
                println!("{}", render_plugin_validate_output(args.format, &output)?);
                if has_errors {
                    return Err(AppError::Config(format!(
                        "plugin validation failed with {} error(s)",
                        output.errors.len()
                    )));
                }
                Ok(())
            }
            PluginSubcommand::Install(args) => {
                let registry_url = args.registry.ok_or_else(|| {
                    AppError::Config("agena plugin install requires --registry <url>".to_string())
                })?;
                let (plugin_id, version) = match args.spec.split_once('@') {
                    Some((id, ver)) => (id.to_string(), Some(ver.to_string())),
                    None => (args.spec.clone(), None),
                };
                let config_path =
                    agena_runtime::default_config_path(&agena_runtime::ProcessEnvironment);
                let outcome = client
                    .install(InstallRequest {
                        registry: RegistrySpec {
                            id: args.registry_id.clone(),
                            url: registry_url,
                            require_signature: args.require_signature,
                        },
                        plugin_id,
                        version,
                        config_path,
                        force: args.force,
                        dry_run: args.dry_run,
                        allow_unverified: args.allow_unverified,
                        refresh_index: args.refresh,
                    })
                    .map_err(|err| AppError::Config(err.to_string()))?;
                if outcome.dry_run {
                    println!(
                        "DRY-RUN: would install {} v{} ({}) into {}",
                        outcome.plugin_id,
                        outcome.version,
                        outcome.kind,
                        outcome.config_path.display()
                    );
                } else {
                    println!(
                        "Installed {} v{} ({}); restart agena to load.",
                        outcome.plugin_id, outcome.version, outcome.kind
                    );
                }
                Ok(())
            }
            PluginSubcommand::Uninstall(args) => {
                let outcomes = client
                    .uninstall_with(&args.plugin_id, args.cascade)
                    .map_err(|err| AppError::Config(err.to_string()))?;
                for outcome in outcomes {
                    println!(
                        "Uninstalled {} v{} from {}",
                        outcome.plugin_id,
                        outcome.version,
                        outcome.config_path.display()
                    );
                }
                Ok(())
            }
            PluginSubcommand::ListInstalled => {
                let records = client
                    .list_installed()
                    .map_err(|err| AppError::Config(err.to_string()))?;
                if records.is_empty() {
                    println!("(no plugins installed via agena marketplace)");
                } else {
                    for record in records {
                        println!(
                            "{} v{} ({}) -> {}",
                            record.plugin_id,
                            record.version,
                            record.kind,
                            record.binary_path.display()
                        );
                    }
                }
                Ok(())
            }
            PluginSubcommand::Sync(args) => {
                let registry = client.registry(RegistrySpec {
                    id: args.registry_id,
                    url: args.registry,
                    require_signature: false,
                });
                let index = registry
                    .fetch_index(true)
                    .map_err(|err| AppError::Config(err.to_string()))?;
                println!(
                    "registry index refreshed: {} plugin(s)",
                    index.plugins.len()
                );
                Ok(())
            }
            PluginSubcommand::Search(args) => {
                let registry = client.registry(RegistrySpec {
                    id: args.registry_id,
                    url: args.registry,
                    require_signature: false,
                });
                let index = registry
                    .fetch_index(false)
                    .map_err(|err| AppError::Config(err.to_string()))?;
                let needle = args.query.to_ascii_lowercase();
                let mut hits = 0usize;
                for plugin in index.plugins {
                    let blob = format!("{} {} {}", plugin.id, plugin.name, plugin.description)
                        .to_ascii_lowercase();
                    if blob.contains(&needle) {
                        hits += 1;
                        println!(
                            "{} — {} ({} version{})",
                            plugin.id,
                            if plugin.description.is_empty() {
                                plugin.name.as_str()
                            } else {
                                plugin.description.as_str()
                            },
                            plugin.versions.len(),
                            if plugin.versions.len() == 1 { "" } else { "s" }
                        );
                    }
                }
                if hits == 0 {
                    println!("(no matches)");
                }
                Ok(())
            }
            PluginSubcommand::Upgrade(args) => {
                let override_spec = args.registry.as_ref().map(|url| RegistrySpec {
                    id: args.registry_id.clone(),
                    url: url.clone(),
                    require_signature: false,
                });
                let targets: Vec<String> = if args.all {
                    client
                        .list_installed()
                        .map_err(|err| AppError::Config(err.to_string()))?
                        .into_iter()
                        .map(|r| r.plugin_id)
                        .collect()
                } else {
                    let id = args.plugin_id.clone().ok_or_else(|| {
                        AppError::Config(
                            "agena plugin upgrade requires <plugin_id> or --all".to_string(),
                        )
                    })?;
                    vec![id]
                };
                let mut errors = Vec::new();
                for id in targets {
                    match client.upgrade(&id, override_spec.clone()) {
                        Ok(out) if out.upgraded => println!(
                            "Upgraded {} {} -> {}",
                            out.plugin_id, out.previous_version, out.installed_version
                        ),
                        Ok(out) => {
                            println!(
                                "{} is up to date (v{})",
                                out.plugin_id, out.previous_version
                            )
                        }
                        Err(err) => errors.push(format!("{id}: {err}")),
                    }
                }
                if !errors.is_empty() {
                    return Err(AppError::Config(errors.join("; ")));
                }
                Ok(())
            }
            PluginSubcommand::Outdated => {
                let outdated = client
                    .list_outdated()
                    .map_err(|err| AppError::Config(err.to_string()))?;
                if outdated.is_empty() {
                    println!("(all installed plugins are up to date)");
                } else {
                    println!("{:<32} {:<14} LATEST", "PLUGIN", "INSTALLED");
                    for record in outdated {
                        println!(
                            "{:<32} {:<14} {}",
                            record.plugin_id, record.installed_version, record.latest_version
                        );
                    }
                }
                Ok(())
            }
        }
    }

    pub(super) async fn run_auth(self, command: AuthCommand) -> Result<(), AppError> {
        let output = self.render_auth_command(command).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_login(self, args: LoginArgs) -> Result<(), AppError> {
        self.with_application(|application| async move {
            let provider_id = normalize_login_provider(args.provider_id.as_str());
            let provider = application
                .auth_provider(provider_id.as_str())
                .map_err(|error| AppError::Config(error.to_string()))?;
            let method_count = usize::from(args.api_key.is_some())
                + usize::from(args.browser)
                + usize::from(args.device);
            if method_count != 1 {
                return Err(AppError::Config(
                    "login requires exactly one of --api-key, --browser, or --device".to_owned(),
                ));
            }

            if let Some(api_key) = args.api_key {
                if !provider.api_key_write_supported {
                    return Err(AppError::Config(format!(
                        "{provider_id} does not support api key login"
                    )));
                }
                application
                    .set_auth_api_key(provider_id.as_str(), api_key)
                    .await
                    .map_err(|error| AppError::Config(error.to_string()))?;
                println!("logged in: {provider_id}");
                return Ok(());
            }

            if args.browser {
                let timeout = Duration::from_secs(args.timeout_secs);
                let kind = AuthLoginKind::from(provider.browser_login_kind.ok_or_else(|| {
                    AppError::Config(format!("{provider_id} does not support browser login"))
                })?);
                let redirect_uri = browser_login_redirect_uri(args.port);
                let start = application
                    .start_auth_browser(provider_id.clone(), kind, redirect_uri.clone())
                    .await
                    .map_err(|error| AppError::Config(error.to_string()))?;
                prompt_browser_login(start.authorize_url.as_str())?;
                let pkce_verifier = start.pkce_verifier.clone();
                let callback_provider_id = provider_id.clone();
                application
                    .complete_auth_browser_callback(
                        callback_provider_id.as_str(),
                        kind,
                        args.port,
                        start.state.as_str(),
                        timeout,
                        pkce_verifier,
                        redirect_uri,
                    )
                    .await
                    .map_err(|error| AppError::Config(error.to_string()))?;
                println!("logged in: {provider_id}");
                return Ok(());
            }

            if args.device {
                let timeout = Duration::from_secs(args.timeout_secs);
                let kind = AuthLoginKind::from(provider.device_login_kind.ok_or_else(|| {
                    AppError::Config(format!("{provider_id} does not support device login"))
                })?);
                let enterprise_domain = args
                    .enterprise_domain
                    .filter(|value| !value.trim().is_empty());
                let start = application
                    .start_auth_device(provider_id.clone(), kind, enterprise_domain.clone())
                    .await
                    .map_err(|error| AppError::Config(error.to_string()))?;
                let device_code = start.device_code.clone();
                let user_code = Some(start.user_code.clone());
                let poll_provider_id = provider_id.clone();
                complete_polled_login(
                    timeout,
                    Duration::from_secs(start.interval_seconds.max(1)),
                    "device login timed out",
                    || prompt_device_login(&start),
                    || {
                        let application = application.clone();
                        let provider_id = poll_provider_id.clone();
                        let device_code = device_code.clone();
                        let user_code = user_code.clone();
                        let enterprise_domain = enterprise_domain.clone();
                        async move {
                            application
                                .poll_auth_device(
                                    provider_id.as_str(),
                                    kind,
                                    device_code,
                                    user_code,
                                    enterprise_domain,
                                )
                                .await
                                .map(|result| result.completed.then_some(()))
                                .map_err(|error| AppError::Config(error.to_string()))
                        }
                    },
                )
                .await?;
                println!("logged in: {provider_id}");
            }

            Ok(())
        })
        .await
    }

    pub(super) async fn run_logout(self, args: LogoutArgs) -> Result<(), AppError> {
        self.with_application(|application| async move {
            let provider_id = normalize_login_provider(args.provider_id.as_str());
            application
                .remove_auth_provider(provider_id.as_str())
                .await
                .map_err(|error| AppError::Config(error.to_string()))?;
            println!("logged out: {provider_id}");
            Ok(())
        })
        .await
    }

    pub(super) async fn run_provider(self, command: ProviderCommand) -> Result<(), AppError> {
        let output = self.render_provider_command(command).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_memory(self, command: MemoryCommand) -> Result<(), AppError> {
        let output = self.render_memory_command(command).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_sessions(self, command: SessionsCommand) -> Result<(), AppError> {
        let output = self.render_sessions_command(command).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_resume(self, args: ResumeArgs) -> Result<(), AppError> {
        let output = self.render_resume_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_continue(self, args: ContinueArgs) -> Result<(), AppError> {
        let output = self.render_continue_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) fn run_completion(self, args: CompletionArgs) -> Result<(), AppError> {
        print!("{}", render_completion_command(args)?);
        Ok(())
    }

    pub(super) async fn run_cost(self, args: CostArgs) -> Result<(), AppError> {
        let output = self.render_cost_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_usage(self, args: UsageArgs) -> Result<(), AppError> {
        let output = self.render_usage_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_permissions(self, args: PermissionsArgs) -> Result<(), AppError> {
        let output = self.render_permissions_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_snapshot(self, args: SnapshotArgs) -> Result<(), AppError> {
        let output = self.render_snapshot_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_git(self, args: GitArgs) -> Result<(), AppError> {
        let output = self.render_git_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_commit(self, args: CommitArgs) -> Result<(), AppError> {
        let output = self.render_commit_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_pr(self, args: PrArgs) -> Result<(), AppError> {
        let output = self.render_pr_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_debug(self, command: DebugCommand) -> Result<(), AppError> {
        let output = self.render_debug_command(command).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_diagnostics(self, args: DiagnosticsArgs) -> Result<(), AppError> {
        let output = self.render_diagnostics_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_exec(self, args: ExecArgs) -> Result<(), AppError> {
        let output = self.render_exec_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_fork(self, args: ForkArgs) -> Result<(), AppError> {
        let output = self.render_fork_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_mcp_server(self, args: McpServerArgs) -> Result<(), AppError> {
        let backend = self.mcp_server_backend(args).await?;
        let lifecycle = backend.clone();
        let result = agena_mcp_server::serve_tools_stdio(backend)
            .await
            .map_err(|err| AppError::Config(err.to_string()));
        lifecycle.shutdown();
        result
    }

    pub(super) async fn run_mcp(self, command: McpCommand) -> Result<(), AppError> {
        match command
            .command
            .unwrap_or(McpSubcommand::Status(McpStatusArgs {
                format: OutputFormat::Json,
            })) {
            McpSubcommand::Status(args) | McpSubcommand::List(args) => {
                self.run_mcp_status(args, None).await
            }
            McpSubcommand::Get(args) => self.run_mcp_get(args).await,
            McpSubcommand::Add(args) => self.run_mcp_add(args).await,
            McpSubcommand::Remove(args) => self.run_mcp_remove(args).await,
            McpSubcommand::Enable(args) => self.run_mcp_toggle(args, true).await,
            McpSubcommand::Disable(args) => self.run_mcp_toggle(args, false).await,
            McpSubcommand::Reconnect(args) => self.run_mcp_reconnect(args).await,
            McpSubcommand::Login(args) => self.run_mcp_login(args).await,
            McpSubcommand::Logout(args) => self.run_mcp_logout(args).await,
        }
    }

    async fn run_mcp_status(
        self,
        args: McpStatusArgs,
        server: Option<String>,
    ) -> Result<(), AppError> {
        let output = self
            .with_session_runtime_services(|services| async move {
                let status = services.status.runtime_status().await;
                match server {
                    Some(server) => {
                        let server = status
                            .mcp
                            .servers
                            .into_iter()
                            .find(|entry| entry.name == server)
                            .ok_or_else(|| {
                                AppError::Config(format!("MCP server not configured: {server}"))
                            })?;
                        render_serialized(args.format, &server)
                    }
                    None => render_serialized(args.format, &status.mcp),
                }
            })
            .await?;
        println!("{output}");
        Ok(())
    }

    async fn run_mcp_get(self, args: McpGetArgs) -> Result<(), AppError> {
        let server = normalized_mcp_server_name(args.server.as_str())?;
        self.run_mcp_status(
            McpStatusArgs {
                format: args.format,
            },
            Some(server),
        )
        .await
    }

    async fn run_mcp_add(self, args: McpAddArgs) -> Result<(), AppError> {
        let server = normalized_mcp_server_name(args.server.as_str())?;
        let server_config = mcp_server_config_value(&args)?;
        let output = self
            .mutate_mcp_plugin_config(
                args.layer,
                args.dry_run,
                !args.no_reload,
                args.format,
                McpConfigMutation::Add {
                    server,
                    config: server_config,
                    force: args.force,
                },
            )
            .await?;
        println!("{output}");
        Ok(())
    }

    async fn run_mcp_remove(self, args: McpRemoveArgs) -> Result<(), AppError> {
        let server = normalized_mcp_server_name(args.server.as_str())?;
        let output = self
            .mutate_mcp_plugin_config(
                args.layer,
                args.dry_run,
                !args.no_reload,
                args.format,
                McpConfigMutation::Remove { server },
            )
            .await?;
        println!("{output}");
        Ok(())
    }

    async fn run_mcp_toggle(
        self,
        args: McpPluginToggleArgs,
        enabled: bool,
    ) -> Result<(), AppError> {
        let output = self
            .mutate_mcp_plugin_config(
                args.layer,
                args.dry_run,
                !args.no_reload,
                args.format,
                McpConfigMutation::SetEnabled(enabled),
            )
            .await?;
        println!("{output}");
        Ok(())
    }

    async fn mutate_mcp_plugin_config(
        &self,
        layer: McpConfigLayerArg,
        dry_run: bool,
        reload: bool,
        format: OutputFormat,
        mutation: McpConfigMutation,
    ) -> Result<String, AppError> {
        self.with_session_runtime_services(|services| async move {
            let read = agena_runtime::ConfigSettingsGetInput {
                target: agena_runtime::ConfigSettingsPathInput {
                    path: Some(MCP_PLUGIN_SETTINGS_PATH.to_owned()),
                },
                source: agena_runtime::ConfigSettingsSource::File,
            };
            let existing = match layer {
                McpConfigLayerArg::Global => services.config_settings.read_file_settings(read),
                McpConfigLayerArg::Workspace => {
                    services.config_settings.read_project_file_settings(read)
                }
            }
            .map_err(|error| AppError::Config(error.to_string()))?;
            let mut record = mcp_plugin_record(existing.value)?;
            apply_mcp_config_mutation(&mut record, mutation)?;
            let input = agena_runtime::ConfigSettingsSetInput {
                path: MCP_PLUGIN_SETTINGS_PATH.to_owned(),
                value: serde_json::Value::Object(record),
                options: agena_runtime::ConfigSettingsEditOptions {
                    dry_run,
                    validate: true,
                    reload,
                },
            };
            let response = match layer {
                McpConfigLayerArg::Global => services.config_settings.set_file_setting(input),
                McpConfigLayerArg::Workspace => {
                    services.config_settings.set_project_file_setting(input)
                }
            }
            .map_err(|error| AppError::Config(error.to_string()))?;
            render_serialized(format, &response)
        })
        .await
    }

    async fn run_mcp_reconnect(self, args: McpReconnectArgs) -> Result<(), AppError> {
        let server = args.server.trim().to_owned();
        if server.is_empty() {
            return Err(AppError::Config(
                "MCP server name must not be empty".to_owned(),
            ));
        }
        let output = self
            .with_session_runtime_services(|services| async move {
                let input = agena_domain::StructuredObject::try_from(serde_json::json!({
                    "server": server,
                }))
                .map_err(|error| AppError::Config(error.to_string()))?;
                let invocation =
                    agena_domain::ToolInvocation::new("agena.mcp.servers.reconnect", input);
                let result = services
                    .tools
                    .execute_runtime_tool(&invocation, -1)
                    .await
                    .map_err(|error| AppError::Config(error.to_string()))?;
                let result = result.into_summary();
                render_serialized(
                    args.format,
                    &McpReconnectOutput {
                        title: result.title,
                        output_text: result.output_text,
                        payload: result.payload,
                    },
                )
            })
            .await?;
        println!("{output}");
        Ok(())
    }

    async fn run_mcp_login(self, args: McpLoginArgs) -> Result<(), AppError> {
        if args.browser {
            return self.run_mcp_oauth_login(args).await;
        }
        let token = read_mcp_login_token(&args)?;
        match args.store {
            McpCredentialStoreArg::Keyring => {
                agena_mcp_client::KeyringTokenStore::new()
                    .put_bearer(args.server.as_str(), token.as_str())
                    .map_err(|error| AppError::Config(error.to_string()))?;
            }
            McpCredentialStoreArg::File => {
                agena_mcp_client::FileTokenStore::open_default()
                    .and_then(|store| store.put_bearer(args.server.as_str(), token.as_str()))
                    .map_err(|error| AppError::Config(error.to_string()))?;
            }
        }
        println!(
            "MCP credential stored for {} ({})",
            args.server,
            store_label(args.store)
        );
        Ok(())
    }

    async fn run_mcp_oauth_login(self, args: McpLoginArgs) -> Result<(), AppError> {
        if args.token.is_some() || args.token_stdin {
            return Err(AppError::Config(
                "--browser is mutually exclusive with --token and --token-stdin".to_owned(),
            ));
        }
        if !matches!(args.store, McpCredentialStoreArg::Keyring) {
            return Err(AppError::Config(
                "MCP OAuth credentials are stored only in the system keyring".to_owned(),
            ));
        }
        let endpoint = args
            .url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .ok_or_else(|| AppError::Config("--browser requires --url MCP_ENDPOINT".to_owned()))?;
        let endpoint = url::Url::parse(endpoint)
            .map_err(|error| AppError::Config(format!("invalid MCP OAuth endpoint: {error}")))?;
        if !matches!(endpoint.scheme(), "http" | "https")
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
        {
            return Err(AppError::Config(
                "MCP OAuth endpoint must be an http(s) URL without embedded credentials".to_owned(),
            ));
        }
        let redirect_uri = format!("http://127.0.0.1:{}/callback", args.port);
        let session = agena_mcp_client::McpOAuthLoginSession::begin(
            args.server.as_str(),
            endpoint,
            args.scopes.as_slice(),
            redirect_uri.as_str(),
        )
        .await
        .map_err(|error| AppError::Config(error.to_string()))?;
        let authorization_url = session.authorization_url().to_owned();
        let expected_state = oauth_state(authorization_url.as_str())?;

        println!("Open this MCP OAuth authorization URL in a browser:\n{authorization_url}");
        println!("Waiting for the loopback callback at {redirect_uri} …");
        let port = args.port;
        let callback = tokio::task::spawn_blocking(move || {
            agena_runtime::wait_for_oauth_callback(
                port,
                expected_state.as_str(),
                StdDuration::from_secs(300),
            )
        })
        .await
        .map_err(|error| AppError::Config(format!("MCP OAuth callback task failed: {error}")))?
        .map_err(|error| AppError::Config(error.to_string()))?;
        session
            .complete(
                callback.code.as_str(),
                callback.state.as_str(),
                callback.issuer.as_deref(),
            )
            .await
            .map_err(|error| AppError::Config(error.to_string()))?;
        println!("MCP OAuth credential stored for {} (keyring)", args.server);
        Ok(())
    }

    async fn run_mcp_logout(self, args: McpLogoutArgs) -> Result<(), AppError> {
        if args.revoke && !args.oauth {
            return Err(AppError::Config(
                "--revoke requires --oauth; bearer credentials cannot be revoked through the MCP OAuth flow"
                    .to_owned(),
            ));
        }
        if args.url.is_some() && !args.revoke {
            return Err(AppError::Config(
                "--url is only valid together with --revoke".to_owned(),
            ));
        }
        if args.oauth {
            if !matches!(args.store, McpCredentialStoreArg::Keyring) {
                return Err(AppError::Config(
                    "MCP OAuth credentials are stored only in the system keyring".to_owned(),
                ));
            }
            if args.revoke {
                let endpoint = args
                    .url
                    .as_deref()
                    .map(str::trim)
                    .filter(|url| !url.is_empty())
                    .ok_or_else(|| {
                        AppError::Config(
                            "--revoke requires --url MCP_ENDPOINT for OAuth metadata discovery"
                                .to_owned(),
                        )
                    })?;
                let endpoint = url::Url::parse(endpoint).map_err(|error| {
                    AppError::Config(format!("invalid MCP OAuth endpoint: {error}"))
                })?;
                if !matches!(endpoint.scheme(), "http" | "https")
                    || endpoint.host_str().is_none()
                    || !endpoint.username().is_empty()
                    || endpoint.password().is_some()
                {
                    return Err(AppError::Config(
                        "MCP OAuth endpoint must be an http(s) URL without embedded credentials"
                            .to_owned(),
                    ));
                }
                agena_mcp_client::McpOAuthLoginSession::revoke_and_clear(
                    args.server.as_str(),
                    endpoint,
                )
                .await
                .map_err(|error| AppError::Config(error.to_string()))?;
                println!(
                    "MCP OAuth credential revoked remotely and removed for {} (keyring)",
                    args.server
                );
                return Ok(());
            }
            agena_mcp_client::KeyringOAuthCredentialStore::new(args.server.as_str())
                .and_then(|store| store.delete())
                .map_err(|error| AppError::Config(error.to_string()))?;
            println!("MCP OAuth credential removed for {} (keyring)", args.server);
            return Ok(());
        }
        match args.store {
            McpCredentialStoreArg::Keyring => {
                agena_mcp_client::KeyringTokenStore::new()
                    .delete(args.server.as_str())
                    .map_err(|error| AppError::Config(error.to_string()))?;
            }
            McpCredentialStoreArg::File => {
                agena_mcp_client::FileTokenStore::open_default()
                    .and_then(|store| store.delete(args.server.as_str()).map(|_| ()))
                    .map_err(|error| AppError::Config(error.to_string()))?;
            }
        }
        println!(
            "MCP credential removed for {} ({})",
            args.server,
            store_label(args.store)
        );
        Ok(())
    }

    pub(super) async fn run_review(self, args: ReviewArgs) -> Result<(), AppError> {
        let output = self.render_review_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_config(self, command: ConfigCommand) -> Result<(), AppError> {
        self.with_session_runtime_services(|services| async move {
            let configuration = services
                .configuration
                .runtime_configuration()
                .map_err(|error| AppError::Config(error.to_string()))?;
            match command
                .command
                .unwrap_or(ConfigSubcommand::Resolve(ConfigResolveArgs {
                    format: OutputFormat::Json,
                })) {
                ConfigSubcommand::Resolve(args) => match args.format {
                    OutputFormat::Json => {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&configuration.configuration_document)?
                        );
                    }
                },
                ConfigSubcommand::Validate => {
                    println!("config valid: path={}", configuration.config_path.display());
                }
            }
            Ok(())
        })
        .await
    }
}

#[derive(serde::Serialize)]
struct McpReconnectOutput {
    title: String,
    output_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<serde_json::Value>,
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
                    "MCP HTTP URL must not embed credentials; use mcp login or --auth-env"
                        .to_owned(),
                ));
            }
            let headers = parse_mcp_key_value_pairs(&args.headers, "--header")?;
            if headers
                .keys()
                .any(|name| name.eq_ignore_ascii_case("authorization"))
            {
                return Err(AppError::Config(
                    "Authorization headers are not accepted by mcp add; use --auth bearer-from-store, mcp login, or --auth bearer-from-env"
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
                    "--header, --auth, and --auth-env are valid only with --url".to_owned(),
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
        (Some(_), Some(_)) => Err(AppError::Config(
            "mcp add requires exactly one of --url or --command".to_owned(),
        )),
        (None, None) => Err(AppError::Config(
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

fn read_mcp_login_token(args: &McpLoginArgs) -> Result<String, AppError> {
    if args.url.is_some() || !args.scopes.is_empty() {
        return Err(AppError::Config(
            "--url and --scope require --browser".to_owned(),
        ));
    }
    if args.token.is_some() == args.token_stdin {
        return Err(AppError::Config(
            "mcp login requires exactly one of --token or --token-stdin".to_owned(),
        ));
    }
    let token = match args.token.as_ref() {
        Some(token) => token.clone(),
        None => {
            let mut token = String::new();
            std::io::stdin().read_to_string(&mut token)?;
            token
        }
    };
    if token.trim().is_empty() {
        return Err(AppError::Config(
            "MCP bearer token must not be empty".to_owned(),
        ));
    }
    Ok(token)
}

fn oauth_state(authorization_url: &str) -> Result<String, AppError> {
    url::Url::parse(authorization_url)
        .map_err(|error| AppError::Config(format!("invalid generated OAuth URL: {error}")))?
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| AppError::Config("generated OAuth URL has no CSRF state".to_owned()))
}

fn store_label(store: McpCredentialStoreArg) -> &'static str {
    match store {
        McpCredentialStoreArg::Keyring => "keyring",
        McpCredentialStoreArg::File => "file",
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::{Value, json};

    use super::{
        McpAddArgs, McpConfigMutation, McpHttpAuthArg, OutputFormat, apply_mcp_config_mutation,
        mcp_plugin_record, mcp_server_config_value,
    };

    #[test]
    fn http_add_uses_store_auth_without_serializing_a_bearer() {
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
            layer: Default::default(),
            force: false,
            dry_run: false,
            no_reload: false,
            format: OutputFormat::Json,
        };

        let value = mcp_server_config_value(&args).expect("HTTP configuration");
        assert_eq!(value["transport"], "http");
        assert_eq!(value["auth"], json!({ "kind": "bearer_from_store" }));
        assert!(value.to_string().contains("mcp.example.test"));
        assert!(!value.to_string().contains("Bearer "));
    }

    #[test]
    fn http_add_oauth_serializes_scopes_but_never_a_client_or_token() {
        let args = McpAddArgs {
            server: "oauth-server".to_owned(),
            url: Some("https://mcp.example.test/api".to_owned()),
            command: None,
            args: Vec::new(),
            env: Vec::new(),
            cwd: None,
            headers: vec!["X-Client=Agena".to_owned()],
            auth: McpHttpAuthArg::OAuth,
            scopes: vec!["mcp:read".to_owned(), "mcp:write".to_owned()],
            include_tools: vec!["repo_*".to_owned()],
            exclude_tools: vec!["repo_delete".to_owned()],
            auth_env: None,
            layer: Default::default(),
            force: false,
            dry_run: false,
            no_reload: false,
            format: OutputFormat::Json,
        };

        let value = mcp_server_config_value(&args).expect("OAuth configuration");
        assert_eq!(
            value["auth"],
            json!({"kind": "oauth", "scopes": ["mcp:read", "mcp:write"]})
        );
        assert_eq!(
            value["tools"],
            json!({"include": ["repo_*"], "exclude": ["repo_delete"]})
        );
        let rendered = value.to_string();
        assert!(!rendered.contains("client_id"));
        assert!(!rendered.contains("access_token"));
        assert!(!rendered.contains("refresh_token"));
    }

    #[test]
    fn stdio_add_preserves_static_plugin_contract_and_remove_is_scoped() {
        let mut record = mcp_plugin_record(Value::Null).expect("new record");
        apply_mcp_config_mutation(
            &mut record,
            McpConfigMutation::Add {
                server: "local".to_owned(),
                config: json!({
                    "transport": "stdio",
                    "process": {
                        "command": "node",
                        "args": ["server.js"],
                        "env": { "MODE": "test" },
                        "cwd": PathBuf::from("/tmp/workspace"),
                    }
                }),
                force: false,
            },
        )
        .expect("add server");
        assert_eq!(record["package"]["kind"], "static");
        assert_eq!(record["config"]["servers"]["local"]["transport"], "stdio");

        apply_mcp_config_mutation(
            &mut record,
            McpConfigMutation::Remove {
                server: "local".to_owned(),
            },
        )
        .expect("remove exact server");
        assert!(
            record["config"]["servers"]
                .as_object()
                .expect("servers object")
                .is_empty()
        );
        assert_eq!(record["package"]["kind"], "static");
    }

    #[test]
    fn http_add_rejects_embedded_or_header_credentials() {
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
            layer: Default::default(),
            force: false,
            dry_run: false,
            no_reload: false,
            format: OutputFormat::Json,
        };
        assert!(
            mcp_server_config_value(&args)
                .expect_err("URL credentials must fail")
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
}
