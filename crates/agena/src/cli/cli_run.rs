use super::{
    AgenaCli, AgenaCommand, AgenaRuntime, AgentsCommand, AppError, ApplyArgs, AuthCommand,
    AuthManager, CommitArgs, CompletionArgs, ConfigCommand, ConfigLoader, ConfigOutputFormat,
    ConfigResolveArgs, ConfigSubcommand, ContinueArgs, CostArgs, DebugCommand, DiagnosticsArgs,
    Duration, ExecArgs, ForkArgs, GitArgs, LoginArgs, LogoutArgs, McpServerArgs, MemoryCommand,
    PermissionsArgs, PluginCommand, PluginInspectOutput, PluginLogOutputFormat, PluginLogsOutput,
    PluginStatusOutput, PluginSubcommand, PrArgs, ProcessEnvironment, ProviderAuthConfig,
    ProviderCommand, ProviderConfigCredentialStore, ProviderDeviceAuthTarget, ProviderOAuthTarget,
    ResumeArgs, ReviewArgs, SessionsCommand, SnapshotArgs, TracingConfig,
    TracingFilterReloadHandle, UsageArgs, browser_login_redirect_uri,
    complete_browser_callback_login, complete_polled_login, copilot_deployment_from_domain,
    format_plugin_logs_output, normalize_login_provider, prompt_device_login,
    render_completion_command, render_plugin_validate_output, render_serialized,
    resolve_login_device_target, resolve_login_oauth_target, validate_plugin_target,
};

impl AgenaCli {
    pub fn resolved_tracing_config(&self) -> TracingConfig {
        ConfigLoader::default()
            .load(&self.load_request())
            .map(|resolution| resolution.config.tracing)
            .unwrap_or_default()
    }

    pub async fn run(
        self,
        tracing_reload_handle: Option<TracingFilterReloadHandle>,
    ) -> Result<(), AppError> {
        let loader = ConfigLoader::new(ProcessEnvironment);

        match self.command.clone() {
            Some(AgenaCommand::AppServer(_)) => Err(AppError::Config(
                "app-server command must be handled by the agena-cli binary".to_owned(),
            )),
            Some(AgenaCommand::Agents(command)) => self.run_agents(command).await,
            Some(AgenaCommand::Apply(args)) => self.run_apply(args),
            Some(AgenaCommand::Auth(command)) => self.run_auth(loader, command).await,
            Some(AgenaCommand::Completion(args)) => self.run_completion(args),
            Some(AgenaCommand::Config(command)) => self.run_config(loader, command),
            Some(AgenaCommand::Continue(args)) => self.run_continue(args).await,
            Some(AgenaCommand::Commit(args)) => self.run_commit(args).await,
            Some(AgenaCommand::Pr(args)) => self.run_pr(args).await,
            Some(AgenaCommand::Debug(command)) => self.run_debug(command).await,
            Some(AgenaCommand::Diagnostics(args)) => self.run_diagnostics(loader, args),
            Some(AgenaCommand::Exec(args)) => self.run_exec(args).await,
            Some(AgenaCommand::Fork(args)) => self.run_fork(args).await,
            Some(AgenaCommand::Cost(args)) => self.run_cost(args).await,
            Some(AgenaCommand::Usage(args)) => self.run_usage(args).await,
            Some(AgenaCommand::Git(args)) => self.run_git(args).await,
            Some(AgenaCommand::Login(args)) => self.run_login(loader, args).await,
            Some(AgenaCommand::Logout(args)) => self.run_logout(loader, args).await,
            Some(AgenaCommand::Memory(command)) => self.run_memory(command),
            Some(AgenaCommand::McpServer(args)) => self.run_mcp_server(args).await,
            Some(AgenaCommand::Permissions(args)) => self.run_permissions(args).await,
            Some(AgenaCommand::Provider(command)) => self.run_provider(loader, command).await,
            Some(AgenaCommand::Plugin(command)) => self.run_plugin(command).await,
            Some(AgenaCommand::Resume(args)) => self.run_resume(args).await,
            Some(AgenaCommand::Review(args)) => self.run_review(args).await,
            Some(AgenaCommand::Sessions(command)) => self.run_sessions(command).await,
            Some(AgenaCommand::Tui(_)) => Err(AppError::Config(
                "tui command must be handled by the agena-cli binary".to_owned(),
            )),
            Some(AgenaCommand::Snapshot(args)) => self.run_snapshot(args).await,
            None => self.run_default(loader, tracing_reload_handle).await,
        }
    }

    pub(super) async fn run_default(
        self,
        loader: ConfigLoader<ProcessEnvironment>,
        tracing_reload_handle: Option<TracingFilterReloadHandle>,
    ) -> Result<(), AppError> {
        loader.load(&self.load_request())?;
        let runtime = AgenaRuntime::new(crate::runtime::AgenaRuntimeConfig {
            load_request: self.load_request(),
            workspace_root: None,
            database_connection: None,
            database_url: None,
            initialize_schema: true,
            tracing_reload_handle,
        })
        .await?;
        let snapshot = runtime.current_snapshot();
        tracing::info!(
            generation = snapshot.generation(),
            providers = snapshot.provider_registry().provider_ids().len(),
            plugins = snapshot.plugin_manager().plugins().len(),
            sessions = snapshot.session_manager().is_some(),
            "Agena started with resolved configuration"
        );
        Ok(())
    }

    pub(super) fn run_apply(self, args: ApplyArgs) -> Result<(), AppError> {
        let output = self.render_apply_command(args)?;
        println!("{output}");
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
                let output = PluginStatusOutput {
                    statuses: runtime
                        .current_snapshot()
                        .plugin_manager()
                        .plugin_statuses(),
                };
                println!("{}", render_serialized(args.format, &output)?);
                Ok(())
            }
            PluginSubcommand::Inspect(args) => {
                let runtime = self.session_runtime().await?;
                let plugin = runtime
                    .current_snapshot()
                    .plugin_manager()
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
                let plugin_manager = runtime.current_snapshot().plugin_manager();
                if plugin_manager
                    .plugin_status(args.plugin_id.as_str())
                    .is_none()
                {
                    return Err(AppError::Config(format!(
                        "plugin not found: {}",
                        args.plugin_id
                    )));
                }
                let output = PluginLogsOutput {
                    plugin_id: args.plugin_id.clone(),
                    logs: plugin_manager.plugin_logs(
                        args.plugin_id.as_str(),
                        args.after_seq,
                        args.limit,
                    ),
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
                let config_path = ConfigLoader::default().default_config_path();
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

    pub(super) async fn run_auth(
        self,
        loader: ConfigLoader<ProcessEnvironment>,
        command: AuthCommand,
    ) -> Result<(), AppError> {
        let output = self.render_auth_command(&loader, command).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_login(
        self,
        loader: ConfigLoader<ProcessEnvironment>,
        args: LoginArgs,
    ) -> Result<(), AppError> {
        let resolution = loader.load(&self.load_request())?;
        let manager = AuthManager::new(ProviderConfigCredentialStore::new(
            resolution.meta.config_path.clone(),
        ));
        let provider_id = normalize_login_provider(args.provider_id.as_str());
        let resolved = resolution
            .config
            .providers
            .get(provider_id.as_str())
            .ok_or_else(|| AppError::Config(format!("provider not found: {provider_id}")))?;
        let method_count = usize::from(args.api_key.is_some())
            + usize::from(args.browser)
            + usize::from(args.device);
        if method_count != 1 {
            return Err(AppError::Config(
                "login requires exactly one of --api-key, --browser, or --device".to_owned(),
            ));
        }

        if let Some(api_key) = args.api_key {
            if !matches!(resolved.auth, ProviderAuthConfig::Api(_)) {
                return Err(AppError::Config(format!(
                    "{provider_id} does not support api key login"
                )));
            }
            manager.set_api_key(provider_id.as_str(), api_key)?;
            println!("logged in: {provider_id}");
            return Ok(());
        }

        if args.browser {
            let timeout = Duration::from_secs(args.timeout_secs);
            match resolve_login_oauth_target(provider_id.as_str(), resolved)? {
                ProviderOAuthTarget::OpenAi => {
                    let redirect_uri = browser_login_redirect_uri(args.port);
                    let start = manager.start_openai_browser_login(redirect_uri.clone())?;
                    let pkce_verifier = start.pkce_verifier.clone();
                    let callback_provider_id = provider_id.clone();
                    complete_browser_callback_login(
                        args.port,
                        timeout,
                        &start,
                        |callback| async move {
                            manager
                                .finish_openai_browser_login(
                                    callback_provider_id.as_str(),
                                    callback.code,
                                    pkce_verifier,
                                    redirect_uri,
                                )
                                .await?;
                            Ok(())
                        },
                    )
                    .await?;
                }
                ProviderOAuthTarget::Gitlab { instance_url } => {
                    let redirect_uri = browser_login_redirect_uri(args.port);
                    let start =
                        manager.start_gitlab_login(instance_url.clone(), redirect_uri.clone())?;
                    let pkce_verifier = start.pkce_verifier.clone();
                    let callback_provider_id = provider_id.clone();
                    complete_browser_callback_login(
                        args.port,
                        timeout,
                        &start,
                        |callback| async move {
                            manager
                                .finish_gitlab_login(
                                    callback_provider_id.as_str(),
                                    instance_url,
                                    callback.code,
                                    pkce_verifier,
                                    redirect_uri,
                                )
                                .await?;
                            Ok(())
                        },
                    )
                    .await?;
                }
            }
            println!("logged in: {provider_id}");
            return Ok(());
        }

        if args.device {
            let timeout = Duration::from_secs(args.timeout_secs);
            match resolve_login_device_target(provider_id.as_str(), resolved)? {
                ProviderDeviceAuthTarget::OpenAi => {
                    let start = manager.start_openai_headless_login().await?;
                    let device_code = start.device_code.clone();
                    let user_code = start.user_code.clone();
                    complete_polled_login(
                        timeout,
                        Duration::from_secs(start.interval_seconds.max(1)),
                        "openai device login timed out",
                        || prompt_device_login(&start),
                        || {
                            manager.poll_openai_headless_login(
                                provider_id.as_str(),
                                device_code.clone(),
                                user_code.clone(),
                            )
                        },
                    )
                    .await?;
                }
                ProviderDeviceAuthTarget::Copilot => {
                    let deployment =
                        copilot_deployment_from_domain(args.enterprise_domain.as_deref());
                    let start = manager.start_copilot_login(deployment.clone()).await?;
                    let device_code = start.device_code.clone();
                    complete_polled_login(
                        timeout,
                        Duration::from_secs(start.interval_seconds.max(1)),
                        "copilot device login timed out",
                        || prompt_device_login(&start),
                        || {
                            manager.poll_copilot_login(
                                provider_id.as_str(),
                                device_code.clone(),
                                deployment.clone(),
                            )
                        },
                    )
                    .await?;
                }
            }
            println!("logged in: {provider_id}");
        }

        Ok(())
    }

    pub(super) async fn run_logout(
        self,
        loader: ConfigLoader<ProcessEnvironment>,
        args: LogoutArgs,
    ) -> Result<(), AppError> {
        let resolution = loader.load(&self.load_request())?;
        let manager = AuthManager::new(ProviderConfigCredentialStore::new(
            resolution.meta.config_path,
        ));
        let provider_id = normalize_login_provider(args.provider_id.as_str());
        let revoke_warning = manager.logout(provider_id.as_str()).await?;
        if let Some(warning) = revoke_warning {
            eprintln!("warning: {warning}");
        }
        println!("logged out: {provider_id}");
        Ok(())
    }

    pub(super) async fn run_provider(
        self,
        loader: ConfigLoader<ProcessEnvironment>,
        command: ProviderCommand,
    ) -> Result<(), AppError> {
        let output = self.render_provider_command(&loader, command).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_agents(self, command: AgentsCommand) -> Result<(), AppError> {
        let output = self.render_agents_command(command).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) fn run_memory(self, command: MemoryCommand) -> Result<(), AppError> {
        let output = self.render_memory_command(command)?;
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
        let output = self.render_commit_command(args)?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_pr(self, args: PrArgs) -> Result<(), AppError> {
        let output = self.render_pr_command(args)?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_debug(self, command: DebugCommand) -> Result<(), AppError> {
        let output = self.render_debug_command(command).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) fn run_diagnostics(
        self,
        loader: ConfigLoader<ProcessEnvironment>,
        args: DiagnosticsArgs,
    ) -> Result<(), AppError> {
        let output = self.render_diagnostics_command(&loader, args)?;
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
        agena_mcp_server::serve_stdio(backend)
            .await
            .map_err(|err| AppError::Config(err.to_string()))
    }

    pub(super) async fn run_review(self, args: ReviewArgs) -> Result<(), AppError> {
        let output = self.render_review_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) fn run_config(
        self,
        loader: ConfigLoader<ProcessEnvironment>,
        command: ConfigCommand,
    ) -> Result<(), AppError> {
        match command
            .command
            .unwrap_or(ConfigSubcommand::Resolve(ConfigResolveArgs {
                format: ConfigOutputFormat::Json,
            })) {
            ConfigSubcommand::Resolve(args) => {
                let resolution = loader.load(&self.load_request())?;
                println!("{}", resolution.render(args.format)?);
            }
            ConfigSubcommand::Validate => {
                let resolution = loader.load(&self.load_request())?;
                println!(
                    "config valid: path={}",
                    resolution.meta.config_path.display()
                );
            }
        }

        Ok(())
    }
}
