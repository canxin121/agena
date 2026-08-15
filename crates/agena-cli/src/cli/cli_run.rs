use super::{
    AgenaCli, AgenaCommand, AppError, ApplyArgs, AuthCommand, CommitArgs, CompletionArgs,
    ConfigCommand, ContinueArgs, CostArgs, DebugCommand, DiagnosticsArgs, ExecArgs, ForkArgs,
    GitArgs, InspectArgs, LoginArgs, LogoutArgs, McpCommand, McpGetArgs, McpServerArgs,
    McpStatusArgs, McpSubcommand, MemoryCommand, OutputFormat, PermissionsArgs, PluginCommand,
    PluginSubcommand, PrArgs, ProviderCommand, ResumeArgs, ReviewArgs, SessionsCommand,
    SnapshotArgs, UsageArgs, render_completion_command, render_plugin_validate_output,
    validate_plugin_target,
};
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
        let output = self.render_server_apply_command(args).await?;
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
                println!("{}", self.render_server_plugin_status(args).await?);
                Ok(())
            }
            PluginSubcommand::Inspect(args) => {
                println!("{}", self.render_server_plugin_inspect(args).await?);
                Ok(())
            }
            PluginSubcommand::Logs(args) => {
                println!("{}", self.render_server_plugin_logs(args).await?);
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
        let output = self.render_server_auth_command(command).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_login(self, args: LoginArgs) -> Result<(), AppError> {
        self.run_server_login(args).await
    }

    pub(super) async fn run_logout(self, args: LogoutArgs) -> Result<(), AppError> {
        self.run_server_logout(args).await
    }

    pub(super) async fn run_provider(self, command: ProviderCommand) -> Result<(), AppError> {
        let output = self.render_server_provider_command(command).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_memory(self, command: MemoryCommand) -> Result<(), AppError> {
        let output = self.render_server_memory_command(command).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_sessions(self, command: SessionsCommand) -> Result<(), AppError> {
        let output = self.render_server_sessions_command(command).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_resume(self, args: ResumeArgs) -> Result<(), AppError> {
        let output = self.render_server_resume_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_continue(self, args: ContinueArgs) -> Result<(), AppError> {
        let output = self.render_server_continue_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) fn run_completion(self, args: CompletionArgs) -> Result<(), AppError> {
        print!("{}", render_completion_command(args)?);
        Ok(())
    }

    pub(super) async fn run_cost(self, args: CostArgs) -> Result<(), AppError> {
        let output = self.render_server_cost_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_usage(self, args: UsageArgs) -> Result<(), AppError> {
        let output = self.render_server_usage_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_permissions(self, args: PermissionsArgs) -> Result<(), AppError> {
        let output = self.render_server_permissions_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_snapshot(self, args: SnapshotArgs) -> Result<(), AppError> {
        let output = self.render_server_snapshot_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_git(self, args: GitArgs) -> Result<(), AppError> {
        let output = self.render_server_git_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_commit(self, args: CommitArgs) -> Result<(), AppError> {
        let output = self.render_server_commit_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_pr(self, args: PrArgs) -> Result<(), AppError> {
        let output = self.render_server_pr_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_debug(self, command: DebugCommand) -> Result<(), AppError> {
        let output = self.render_server_debug_command(command).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_diagnostics(self, args: DiagnosticsArgs) -> Result<(), AppError> {
        let output = self.render_server_diagnostics_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_exec(self, args: ExecArgs) -> Result<(), AppError> {
        let output = self.render_server_exec_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_fork(self, args: ForkArgs) -> Result<(), AppError> {
        let output = self.render_server_fork_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_mcp_server(self, args: McpServerArgs) -> Result<(), AppError> {
        let backend = self.server_mcp_backend(args).await?;
        agena_mcp_server::serve_tools_stdio(backend)
            .await
            .map_err(|error| AppError::Config(error.to_string()))
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
            McpSubcommand::Add(args) => {
                println!("{}", self.render_server_mcp_add(args).await?);
                Ok(())
            }
            McpSubcommand::Remove(args) => {
                println!("{}", self.render_server_mcp_remove(args).await?);
                Ok(())
            }
            McpSubcommand::Enable(args) => {
                println!("{}", self.render_server_mcp_toggle(args, true).await?);
                Ok(())
            }
            McpSubcommand::Disable(args) => {
                println!("{}", self.render_server_mcp_toggle(args, false).await?);
                Ok(())
            }
            McpSubcommand::Reconnect(args) => {
                println!("{}", self.render_server_mcp_reconnect(args).await?);
                Ok(())
            }
            McpSubcommand::Login(args) => self.run_server_mcp_login(args).await,
            McpSubcommand::Logout(args) => self.run_server_mcp_logout(args).await,
        }
    }

    async fn run_mcp_status(
        self,
        args: McpStatusArgs,
        server: Option<String>,
    ) -> Result<(), AppError> {
        let output = self.render_server_mcp_status(args, server).await?;
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

    pub(super) async fn run_review(self, args: ReviewArgs) -> Result<(), AppError> {
        let output = self.render_server_review_command(args).await?;
        println!("{output}");
        Ok(())
    }

    pub(super) async fn run_config(self, command: ConfigCommand) -> Result<(), AppError> {
        println!("{}", self.render_server_config_command(command).await?);
        Ok(())
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
