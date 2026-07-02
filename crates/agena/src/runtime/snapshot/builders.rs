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
    let processor = build_session_processor(providers, Arc::clone(&plugins), workspace_root);
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
    workspace_root: &Path,
) -> SessionProcessor {
    SessionProcessor::new(providers, ContextGovernor::new(ContextPolicy::default()))
        .with_plugin_host(plugins)
        .with_workspace_root(workspace_root)
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
    let snapshot_registry = crate::tool::snapshot_registry_for_executor();

    // Drop any orphan snapshots left over from a previously-crashed
    // session so a clean startup does not accumulate
    // managed snapshot directories indefinitely. Stale = no live
    // session and not registered with `git worktree list`.
    let pruned = crate::tool::snapshot_prune_stale(workspace_root, &snapshot_registry);
    if !pruned.is_empty() {
        tracing::info!(
            target: "agena::runtime::snapshot",
            removed = pruned.len(),
            "pruned stale snapshot directories at startup"
        );
    }

    let mut executor = ToolExecutor::new(workspace_root.to_path_buf(), agent)
        .with_plugin_manager(plugins)
        .with_tool_presentation(resolution.config.plugins.policy.tool_presentation.clone())
        .with_subagent_registry(agents)
        .with_snapshot_registry(snapshot_registry);

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
    config: &crate::plugins::provided::lsp::LspConfig,
) -> Arc<agena_lsp::LspRegistry> {
    use agena_lsp::{LspRegistry, LspServerSpec};

    let registry = Arc::new(LspRegistry::new(
        workspace_root.to_path_buf(),
        crate::provider::CODEX_ORIGINATOR,
        crate::provider::CODEX_PACKAGE_VERSION,
    ));

    let registry_for_register = registry.clone();
    let defaults = config.defaults.clone();
    let entries: Vec<(String, crate::plugins::provided::lsp::LspServerConfig)> = config
        .servers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    tokio::spawn(async move {
        for (name, entry) in entries {
            let spec: LspServerSpec = entry.runtime_spec(name.clone(), &defaults);
            registry_for_register.register(spec).await;
            tracing::info!(
                target: "agena::lsp",
                "registered LSP server '{name}' (lazy-spawn)"
            );
        }
    });

    registry
}

pub(super) fn session_manager_config(resolution: &ConfigResolution) -> SessionManagerConfig {
    let defaults = SessionManagerConfig::default();
    SessionManagerConfig {
        cache_max_sessions: resolution.config.runtime.session.cache.max_sessions,
        cache_ttl: Duration::from_secs(resolution.config.runtime.session.cache.ttl_secs),
        cache_max_bytes: resolution.config.runtime.session.cache.max_bytes,
        doom_loop: defaults.doom_loop,
        default_selection: resolution.config.default_selection.clone(),
        default_agent: resolution.config.default_agent.clone(),
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
                permission: config.permission.clone(),
                defaults: config.defaults.clone(),
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
    push_watch_path(&mut paths, resolution.meta.project_config_path.clone());

    let base_dir = resolution
        .meta
        .config_path
        .parent()
        .unwrap_or_else(|| Path::new("."));

    for entry in resolution.config.plugins.list.values() {
        if let crate::plugin::PluginPackage::Cdylib { path, .. } = &entry.package {
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
