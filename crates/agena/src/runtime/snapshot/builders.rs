use super::*;

pub(super) fn build_or_reconfigure_session_manager(
    existing: Option<Arc<SessionManager>>,
    db: &Arc<DatabaseConnection>,
    providers: Arc<ProviderRegistry>,
    plugins: Arc<PluginHost>,
    agents: crate::agents::SubagentRegistry,
    lsp_registry: Option<Arc<agena_lsp::LspRegistry>>,
    workspace_root: &Path,
    resolution: &ConfigResolution,
) -> Arc<SessionManager> {
    let processor = build_session_processor(providers, Arc::clone(&plugins));
    let config = session_manager_config(resolution);

    if let Some(manager) = existing {
        let executor = build_tool_executor(
            plugins,
            agents.clone(),
            lsp_registry,
            workspace_root,
            resolution,
            Some(Arc::clone(&manager)),
        );
        manager.reconfigure(processor, executor, config);
        return manager;
    }

    let bootstrap_executor = build_tool_executor(
        Arc::clone(&plugins),
        agents.clone(),
        lsp_registry.clone(),
        workspace_root,
        resolution,
        None,
    );
    let manager = Arc::new(
        SessionManager::new(db.as_ref().clone(), processor.clone(), bootstrap_executor)
            .with_config(config.clone()),
    );
    let executor = build_tool_executor(
        plugins,
        agents,
        lsp_registry,
        workspace_root,
        resolution,
        Some(Arc::clone(&manager)),
    );
    manager.reconfigure(processor, executor, config);
    manager
}

pub(super) fn build_session_processor(
    providers: Arc<ProviderRegistry>,
    plugins: Arc<PluginHost>,
) -> SessionProcessor {
    SessionProcessor::new(providers, ContextGovernor::new(ContextPolicy::default()))
        .with_plugin_host(plugins)
}

pub(super) fn build_tool_executor(
    plugins: Arc<PluginHost>,
    agents: crate::agents::SubagentRegistry,
    lsp_registry: Option<Arc<agena_lsp::LspRegistry>>,
    workspace_root: &Path,
    resolution: &ConfigResolution,
    session_manager: Option<Arc<SessionManager>>,
) -> ToolExecutor {
    let agent = build_profile_agent(
        "build",
        crate::agent::PermissionConfig::default(),
        resolution,
    );
    let worktree_registry = crate::tool::worktree_registry_for_executor();

    // Drop any orphan worktrees left over from a previously-crashed
    // session so a clean startup does not accumulate
    // .agena/worktrees/<slug> directories indefinitely. Stale = no live
    // session and not registered with `git worktree list`.
    let pruned = crate::tool::worktree_prune_stale(workspace_root, &worktree_registry);
    if !pruned.is_empty() {
        tracing::info!(
            target: "agena::runtime::worktree",
            removed = pruned.len(),
            "pruned stale worktree directories at startup"
        );
    }

    let mut executor = ToolExecutor::new(workspace_root.to_path_buf(), agent)
        .with_plugin_manager(plugins)
        .with_tool_presentation(resolution.config.plugins.tool_presentation.clone())
        .with_subagent_registry(agents)
        .with_web_search_backend(resolution.config.web.search.resolve())
        .with_plan_registry(crate::tool::plan_registry_for_executor())
        .with_worktree_registry(worktree_registry);

    if let Some(manager) = session_manager {
        executor = executor.with_scheduler(build_scheduler(manager));
    }

    if let Some(registry) = lsp_registry {
        executor = executor.with_lsp_registry(registry);
    }

    executor
}

pub(super) fn build_lsp_registry(
    workspace_root: &Path,
    config: &crate::config::LspConfig,
) -> Arc<agena_lsp::LspRegistry> {
    use agena_lsp::{LspRegistry, LspServerSpec};

    let registry = Arc::new(LspRegistry::new(
        workspace_root.to_path_buf(),
        crate::provider::CODEX_ORIGINATOR,
        crate::provider::CODEX_PACKAGE_VERSION,
    ));

    let registry_for_register = registry.clone();
    let entries: Vec<(String, crate::config::LspServerConfig)> = config
        .servers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    tokio::spawn(async move {
        for (name, entry) in entries {
            let env = entry
                .env
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let spec = LspServerSpec {
                name: name.clone(),
                command: entry.command.clone(),
                args: entry.args.clone(),
                env,
                file_extensions: entry.file_extensions.clone(),
                root_markers: entry.root_markers.clone(),
                initialization_options: entry.initialization_options.clone(),
            };
            registry_for_register.register(spec).await;
            tracing::info!(
                target: "agena::lsp",
                "registered LSP server '{name}' (lazy-spawn)"
            );
        }
    });

    registry
}

pub(super) async fn build_mcp_manager(
    config: &crate::config::McpConfig,
) -> Arc<agena_mcp_client::McpConnectionManager> {
    use agena_mcp_client::{
        FileTokenStore, HttpTransportMode, McpConnectionManager, ServerSpec, TokenStore,
    };

    let mut manager = McpConnectionManager::new(
        crate::provider::CODEX_MCP_CLIENT_NAME,
        crate::provider::CODEX_PACKAGE_VERSION,
    );

    // Best-effort: open the on-disk token store so HttpAuth::BearerFromStore
    // can resolve. A missing file is fine; a corrupt one is logged and the
    // store is left unset (runtime continues, just without token lookup).
    match FileTokenStore::open_default() {
        Ok(store) => {
            manager.set_token_store(Arc::new(store) as Arc<dyn TokenStore>);
        }
        Err(err) => {
            tracing::warn!(
                target: "agena::mcp",
                "failed to open default token store: {err}"
            );
        }
    }

    let manager = Arc::new(manager);
    // Failures only disable that one server — the rest of the runtime keeps booting.
    for (name, entry) in &config.servers {
        let manager = manager.clone();
        let name = name.clone();
        let spec = match entry {
            crate::config::McpServerConfig::Stdio {
                command,
                args,
                env,
                cwd,
            } => ServerSpec::Stdio {
                command: command.clone(),
                args: args.clone(),
                env: env.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                cwd: cwd.clone(),
            },
            crate::config::McpServerConfig::Http {
                url,
                mode,
                headers,
                auth,
            } => {
                let Some(parsed) = parse_mcp_server_url(name.as_str(), url.as_str()) else {
                    continue;
                };
                let mode = match mode {
                    crate::config::McpHttpMode::Sse => HttpTransportMode::Sse,
                    crate::config::McpHttpMode::StreamableHttp => HttpTransportMode::StreamableHttp,
                };
                let auth = map_mcp_auth(auth.as_ref());
                ServerSpec::Http {
                    url: parsed,
                    mode,
                    headers: headers
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                    auth,
                }
            }
            crate::config::McpServerConfig::Ws { url, headers, auth } => {
                let Some(parsed) = parse_mcp_server_url(name.as_str(), url.as_str()) else {
                    continue;
                };
                ServerSpec::Ws {
                    url: parsed,
                    headers: headers
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                    auth: map_mcp_auth(auth.as_ref()),
                }
            }
        };
        if let Err(e) = manager.add_server(&name, spec).await {
            tracing::warn!(
                target: "agena::mcp",
                "failed to connect MCP server '{name}': {e}"
            );
        } else {
            tracing::info!(target: "agena::mcp", "connected MCP server '{name}'");
        }
    }
    manager
}

fn parse_mcp_server_url(name: &str, url: &str) -> Option<url::Url> {
    match url::Url::parse(url) {
        Ok(parsed) => Some(parsed),
        Err(err) => {
            tracing::warn!(
                target: "agena::mcp",
                "skipping mcp server '{name}': invalid url '{url}': {err}"
            );
            None
        }
    }
}

fn map_mcp_auth(
    auth: Option<&crate::config::McpHttpAuthConfig>,
) -> Option<agena_mcp_client::HttpAuth> {
    auth.map(|cfg| match cfg {
        crate::config::McpHttpAuthConfig::Bearer { token } => {
            agena_mcp_client::HttpAuth::Bearer(token.clone())
        }
        crate::config::McpHttpAuthConfig::BearerFromEnv { env } => {
            agena_mcp_client::HttpAuth::BearerFromEnv(env.clone())
        }
        crate::config::McpHttpAuthConfig::BearerFromStore => {
            agena_mcp_client::HttpAuth::BearerFromStore
        }
        crate::config::McpHttpAuthConfig::Custom { headers } => agena_mcp_client::HttpAuth::Custom(
            headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ),
    })
}

pub(super) fn session_manager_config(resolution: &ConfigResolution) -> SessionManagerConfig {
    let defaults = SessionManagerConfig::default();
    SessionManagerConfig {
        cache_max_sessions: resolution.config.runtime.session_cache.max_sessions,
        cache_ttl: Duration::from_secs(resolution.config.runtime.session_cache.ttl_secs),
        cache_max_bytes: resolution.config.runtime.session_cache.max_bytes,
        doom_loop: defaults.doom_loop,
        default_selection: resolution.config.default.clone(),
        default_agent: resolution.config.default.agent.clone(),
        permission: resolution.config.permission.clone(),
        auto_compaction: crate::session::SessionAutoCompactionConfig {
            enabled: resolution.config.session.compaction.auto,
            reserved_tokens: resolution.config.session.compaction.reserved_tokens,
        },
    }
}

pub(super) fn register_config_agents(
    registry: &crate::agents::SubagentRegistry,
    _: &ConfigResolution,
    agents: &std::collections::BTreeMap<String, crate::config::AgentConfig>,
) {
    for (name, config) in agents {
        if config.disabled || name.trim().is_empty() {
            continue;
        }
        registry.register_runtime(crate::agents::AgentProfile {
            name: name.trim().to_string(),
            frontmatter: crate::agents::AgentFrontmatter {
                description: config.description.clone(),
                mode: config.mode,
                hidden: config.hidden,
                color: config.color.clone(),
                temperature: config.temperature,
                max_output_tokens: config.max_output_tokens,
                steps: config.steps,
                allowed_tools: config.allowed_tools.clone(),
                permission: config.permission.clone(),
                default: config.default.clone(),
                aliases: config.aliases.clone(),
            },
            prompt: config.prompt.trim().to_string(),
            source_path: None,
            scope: crate::agents::AgentScope::Project,
        });
    }
}

pub(super) fn build_profile_agent(
    name: impl Into<String>,
    permission: crate::agent::PermissionConfig,
    _: &ConfigResolution,
) -> Agent {
    let agent = Agent::new(name, crate::permission::PermissionPolicy::allow_all());
    match agent.try_with_permission_config(&permission) {
        Ok(agent) => agent,
        Err(err) => {
            tracing::warn!(
                target: "agena::config::permission",
                "ignoring invalid agent permission config: {err}; falling back to allow_all"
            );
            Agent::new("build", crate::permission::PermissionPolicy::allow_all())
        }
    }
}

pub(super) fn collect_watch_paths(resolution: &ConfigResolution) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    push_watch_path(&mut paths, resolution.meta.config_path.clone());

    let base_dir = resolution
        .meta
        .config_path
        .parent()
        .unwrap_or_else(|| Path::new("."));

    for entry in resolution.config.plugins.list.values() {
        if let crate::plugin::PluginEntry::Cdylib { path, .. } = entry {
            let resolved = if path.is_absolute() {
                path.clone()
            } else {
                base_dir.join(path)
            };
            push_watch_path(&mut paths, resolved);
        }
    }

    paths
}

pub(super) fn push_watch_path(paths: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !paths.iter().any(|existing| existing == &candidate) {
        paths.push(candidate);
    }
}

/// Build a process-wide cron scheduler backed by the active SessionManager.
pub(super) fn build_scheduler(
    session_manager: Arc<SessionManager>,
) -> Arc<agena_scheduler::Scheduler> {
    use std::time::Duration;

    struct SessionSink {
        session_manager: Arc<SessionManager>,
    }

    impl SessionSink {
        fn notify_job_result(
            &self,
            job: &agena_scheduler::ScheduledJob,
            result: &agena_scheduler::JobDeliveryResult,
        ) {
            let status = match result.status {
                agena_scheduler::JobRunStatus::Submitted => "submitted",
                agena_scheduler::JobRunStatus::Skipped => "skipped",
                agena_scheduler::JobRunStatus::Failed => "failed",
            };
            let title = format!("Scheduled job {status}");
            let message = result
                .error_message
                .clone()
                .unwrap_or_else(|| job.prompt.clone());
            let payload = serde_json::json!({
                "job_id": job.id,
                "prompt": job.prompt,
                "owner_session_id": job.owner_session_id,
                "status": status,
                "error_message": result.error_message,
                "next_fire_at": job.next_fire_at,
                "last_fired_at": job.last_fired_at,
            });
            self.session_manager.tool_executor().broadcast_notification(
                "scheduled_job",
                result.session_id.or(job.owner_session_id),
                title,
                message,
                payload,
            );
        }
    }

    #[async_trait::async_trait]
    impl agena_scheduler::JobSink for SessionSink {
        async fn deliver(
            &self,
            job: &agena_scheduler::ScheduledJob,
        ) -> agena_scheduler::JobDeliveryResult {
            let result = if let Some(session_id) = job.owner_session_id {
                if self.session_manager.is_run_active(session_id).await {
                    agena_scheduler::JobDeliveryResult::skipped(
                        Some(session_id),
                        "session already has an active run",
                    )
                } else {
                    let session = match self.session_manager.get_session(session_id).await {
                        Ok(session) => session,
                        Err(err) => {
                            let result = agena_scheduler::JobDeliveryResult::failed(
                                Some(session_id),
                                err.to_string(),
                            );
                            self.notify_job_result(job, &result);
                            return result;
                        }
                    };

                    if session.blocked() {
                        agena_scheduler::JobDeliveryResult::skipped(
                            Some(session_id),
                            "session is blocked on permission or user input",
                        )
                    } else {
                        let options = match self
                            .session_manager
                            .resolve_scheduled_run_options(session_id)
                            .await
                        {
                            Ok(options) => options,
                            Err(err) => {
                                let result = agena_scheduler::JobDeliveryResult::failed(
                                    Some(session_id),
                                    err.to_string(),
                                );
                                self.notify_job_result(job, &result);
                                return result;
                            }
                        };

                        match self
                            .session_manager
                            .submit_user_message(crate::session::SessionUserMessageRequest::new(
                                session_id,
                                options,
                                vec![crate::message::PartContent::text(job.prompt.clone())],
                            ))
                            .await
                        {
                            Ok(_) => {
                                agena_scheduler::JobDeliveryResult::submitted(Some(session_id))
                            }
                            Err(err) => agena_scheduler::JobDeliveryResult::failed(
                                Some(session_id),
                                err.to_string(),
                            ),
                        }
                    }
                }
            } else {
                agena_scheduler::JobDeliveryResult::failed(
                    None,
                    "scheduled job has no owner_session_id",
                )
            };
            self.notify_job_result(job, &result);
            result
        }
    }

    let sched = agena_scheduler::scheduler::build_in_memory(
        Arc::new(SessionSink { session_manager }),
        Duration::from_secs(10),
    );
    sched.start();
    sched
}
