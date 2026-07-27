type SessionCompositionInputs<'a> = agena_runtime::SessionCompositionInputs<
    Option<Arc<SessionManager>>,
    &'a Arc<DatabaseConnection>,
    Arc<ProviderRegistry>,
    Arc<PluginHost>,
    crate::agents::SubagentRegistry,
    Option<Arc<agena_lsp::LspRegistry>>,
    &'a Path,
    &'a agena_runtime::RuntimeSessionBuildConfig,
>;

type ToolCompositionInputs<'a> = agena_runtime::ToolCompositionInputs<
    Arc<PluginHost>,
    crate::agents::SubagentRegistry,
    Option<Arc<agena_lsp::LspRegistry>>,
    &'a Path,
    agena_plugin_host::ToolPresentationConfig,
    Option<Arc<SessionManager>>,
    Arc<DatabaseConnection>,
>;

pub(super) fn build_or_reconfigure_session_manager(
    inputs: SessionCompositionInputs<'_>,
) -> Arc<SessionManager> {
    let agena_runtime::SessionCompositionInputs {
        existing,
        database: db,
        providers,
        plugins,
        agents,
        lsp_registry,
        workspace_root,
        config: build_config,
        mcp_manager,
    } = inputs;
    let permission_inspector = mcp_manager.map(|manager| {
        Arc::new(McpRiskPermissionInspector { manager })
            as Arc<dyn agena_runtime_tools::tool::ExecutionPermissionInspector>
    });
    let processor = SessionProcessor::new(
        providers,
        ContextGovernor::new(ContextPolicy::default()),
        Arc::clone(&plugins),
        workspace_root,
    );
    let config = agena_runtime::RuntimeSessionManagerConfig {
        default_selection: build_config.default_selection.clone(),
        default_agent: build_config.default_agent.clone(),
        permission: build_config.permission.clone(),
        auto_compaction: build_config.auto_compaction,
        cache_limits: build_config.cache_limits,
        max_concurrent_tools: build_config.max_concurrent_tools,
    };

    if let Some(manager) = existing {
        let executor = build_tool_executor(
            agena_runtime::ToolCompositionInputs {
                plugins,
                agents: agents.clone(),
                lsp_registry,
                workspace_root,
                tool_presentation: build_config.tool_presentation.clone(),
                session_manager: Some(Arc::clone(&manager)),
                database: Arc::clone(db),
            },
            permission_inspector,
        );
        manager.reconfigure(processor, executor, config);
        return manager;
    }

    let bootstrap_executor = build_tool_executor(
        agena_runtime::ToolCompositionInputs {
            plugins: Arc::clone(&plugins),
            agents: agents.clone(),
            lsp_registry: lsp_registry.clone(),
            workspace_root,
            tool_presentation: build_config.tool_presentation.clone(),
            session_manager: None,
            database: Arc::clone(db),
        },
        permission_inspector.clone(),
    );
    let manager = Arc::new(SessionManager::new(
        db.as_ref().clone(),
        processor.clone(),
        bootstrap_executor,
        config.clone(),
    ));
    let executor = build_tool_executor(
        agena_runtime::ToolCompositionInputs {
            plugins,
            agents,
            lsp_registry,
            workspace_root,
            tool_presentation: build_config.tool_presentation.clone(),
            session_manager: Some(Arc::clone(&manager)),
            database: Arc::clone(db),
        },
        permission_inspector,
    );
    manager.reconfigure(processor, executor, config);
    manager
}

pub(super) fn build_event_bridge(
    session_manager: Option<&Arc<SessionManager>>,
    plugins: &Arc<PluginHost>,
) -> Option<Arc<agena_runtime::AbortOnDrop>> {
    session_manager.map(|manager| {
        Arc::new(crate::event::bridge::spawn_event_bridge(
            manager.event_bus(),
            Arc::clone(plugins),
        ))
    })
}

pub(super) async fn resume_session_state(
    session_manager: Option<&Arc<SessionManager>>,
    reusing: bool,
) -> Result<(), crate::AppError> {
    if reusing {
        return Ok(());
    }
    if let Some(manager) = session_manager {
        manager
            .event_publisher()
            .resume_from_store()
            .await
            .map_err(|err| {
                crate::AppError::Internal(format!("resume event sequence failed: {err}"))
            })?;
        manager.reconcile_interrupted_executions().await?;
    }
    Ok(())
}

pub(super) async fn build_model_catalog_services(
    inputs: agena_runtime::ModelCatalogCompositionInputs<
        &std::collections::BTreeMap<String, agena_runtime::ResolvedProviderConfig>,
        &Path,
        &PluginHost,
        Option<Arc<DatabaseConnection>>,
    >,
) -> Result<
    (
        Arc<ProviderRegistry>,
        Arc<agena_runtime::ModelCatalogService>,
    ),
    crate::AppError,
> {
    let agena_runtime::ModelCatalogCompositionInputs {
        providers,
        config_path,
        plugins,
        database,
    } = inputs;
    let catalog_source_providers = Arc::new(
        crate::config::build_provider_registry_from_inputs(
            providers,
            Some(config_path),
            plugins,
            None,
        )
        .await?,
    );
    let model_catalog = agena_runtime::ModelCatalogService::compose_default_optional(database)
        .await
        .map_err(|error| crate::AppError::Config(error.to_string()))?;
    Ok((catalog_source_providers, model_catalog))
}

pub(super) async fn build_runtime_provider_registry(
    providers: &std::collections::BTreeMap<String, agena_runtime::ResolvedProviderConfig>,
    config_path: &Path,
    plugins: &PluginHost,
    catalog_snapshot: &agena_provider::ModelCatalogSnapshot,
) -> Result<Arc<ProviderRegistry>, crate::AppError> {
    Ok(Arc::new(
        crate::config::build_provider_registry_from_inputs(
            providers,
            Some(config_path),
            plugins,
            Some(catalog_snapshot),
        )
        .await?,
    ))
}

pub(super) async fn build_plugin_services(
    inputs: agena_runtime::PluginCompositionInputs<
        agena_plugin_host::PluginsConfig,
        &Path,
        Option<Arc<PluginHost>>,
        Option<agena_plugin_host::PluginsConfig>,
        Option<Arc<agena_mcp_client::McpConnectionManager>>,
    >,
) -> Result<Arc<PluginHost>, crate::AppError> {
    let agena_runtime::PluginCompositionInputs {
        plugin_config,
        workspace_root,
        previous_host,
        previous_config,
        mcp_manager,
    } = inputs;
    let static_plugins =
        agena_bundled_plugins::plugins::sources::static_plugin_registrations(mcp_manager);
    let plugins = agena_runtime::compose_and_install_plugin_host(
        static_plugins,
        plugin_config,
        workspace_root,
        previous_host,
        previous_config.as_ref(),
        agena_runtime::codex_package_version(),
    )
    .await
    .map_err(|error| crate::AppError::Config(format!("plugin host: {error}")))?;
    Ok(plugins)
}

pub(super) fn build_agent_registry(
    workspace_root: &Path,
    config_parent: Option<&Path>,
    configured_agents: &std::collections::BTreeMap<String, agena_runtime::AgentConfig>,
) -> crate::agents::SubagentRegistry {
    let agents = crate::agents::SubagentRegistry::discover(workspace_root, config_parent);
    register_config_agents(&agents, configured_agents);
    agents
}

pub(super) fn build_tool_executor(
    inputs: ToolCompositionInputs<'_>,
    permission_inspector: Option<Arc<dyn agena_runtime_tools::tool::ExecutionPermissionInspector>>,
) -> ToolExecutor {
    let agena_runtime::ToolCompositionInputs {
        plugins,
        agents,
        lsp_registry,
        workspace_root,
        tool_presentation,
        session_manager,
        database,
    } = inputs;
    let agent = build_profile_agent("build", crate::agent::PermissionConfig::default());
    let snapshot_registry = crate::tool::snapshot_registry_for_executor();

    // Drop any orphan snapshots left over from a previously-crashed
    // session so a clean startup does not accumulate
    // managed snapshot directories indefinitely. Stale = no live
    // session and not registered with `git worktree list`.
    let pruned = agena_runtime::prune_stale_managed_snapshots(workspace_root, &snapshot_registry);
    if !pruned.is_empty() {
        tracing::info!(
            target: "agena::runtime::snapshot",
            removed = pruned.len(),
            "pruned stale snapshot directories at startup"
        );
    }

    let scheduler = session_manager
        .map(|session_manager| build_scheduler(session_manager, database.as_ref().clone()));
    ToolExecutor::new(
        workspace_root.to_path_buf(),
        agent,
        agents,
        plugins,
        Some(snapshot_registry),
        scheduler,
        lsp_registry,
        tool_presentation,
    )
    .with_permission_inspector(permission_inspector)
}

pub(super) fn register_config_agents(
    registry: &crate::agents::SubagentRegistry,
    agents: &std::collections::BTreeMap<String, agena_runtime::AgentConfig>,
) {
    for registration in agena_runtime::configured_agent_registrations(agents) {
        let name = registration.name;
        let config = registration.config;
        if config.disabled {
            registry.disable_runtime(&name);
            continue;
        }
        let agent_name = name.clone();
        let result = registry.register_runtime(crate::agents::AgentProfile {
            name,
            frontmatter: crate::agents::AgentFrontmatter {
                description: config.description.clone(),
                permission: config.permission.clone(),
                defaults: config.defaults.clone(),
                tools: config.tools.clone(),
            },
            prompt: config.prompt.trim().to_string(),
            source_path: None,
            scope: agena_domain::AgentScope::Project,
        });
        if let Err(error) = result {
            tracing::error!(
                target: "agena::agents",
                agent = %agent_name,
                "failed to register configured agent: {error}"
            );
        }
    }
}

pub(super) fn build_profile_agent(
    name: impl Into<String>,
    permission: crate::agent::PermissionConfig,
) -> Agent {
    let agent = Agent::new(
        name,
        crate::permission::PermissionPolicy::allow_all(),
        crate::permission::ToolPermissionPolicy::allow_all(),
    );
    match agent.try_apply_permission_config(&permission) {
        Ok(agent) => agent,
        Err(err) => {
            tracing::error!(
                target: "agena::config::permission",
                "refusing invalid agent permission config: {err}"
            );
            let mut denied = Agent::new(
                "build",
                crate::permission::PermissionPolicy::allow_all(),
                crate::permission::ToolPermissionPolicy::allow_all(),
            );
            denied.disable = true;
            denied
        }
    }
}

/// Build a process-wide cron scheduler backed by the active SessionManager.
pub(super) fn build_scheduler(
    session_manager: Arc<SessionManager>,
    database: DatabaseConnection,
) -> Arc<agena_scheduler::Scheduler> {
    struct SessionSink {
        session_manager: std::sync::Weak<SessionManager>,
    }

    impl SessionSink {
        fn notify_job_result(
            &self,
            session_manager: &SessionManager,
            job: &agena_scheduler::ScheduledJob,
            delivery: &agena_scheduler::JobDeliveryAttempt,
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
                "delivery_key": delivery.delivery_key,
                "delivery_attempt": delivery.attempt,
                "scheduled_for": delivery.scheduled_for,
                "prompt": job.prompt,
                "owner_session_id": job.owner_session_id,
                "status": status,
                "error_message": result.error_message,
                "next_fire_at": job.next_fire_at,
                "last_fired_at": job.last_fired_at,
            });
            session_manager.tool_executor().broadcast_notification(
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
            delivery: &agena_scheduler::JobDeliveryAttempt,
        ) -> agena_scheduler::JobDeliveryResult {
            let Some(session_manager) = self.session_manager.upgrade() else {
                return agena_scheduler::JobDeliveryResult::skipped(
                    None,
                    "runtime session manager is no longer available",
                );
            };
            let result = if let Some(session_id) = job.owner_session_id {
                match session_manager
                    .has_user_message_idempotency_key(session_id, &delivery.delivery_key)
                    .await
                {
                    Ok(true) => agena_scheduler::JobDeliveryResult::skipped(
                        Some(session_id),
                        "delivery key is already present in session history",
                    ),
                    Err(err) => agena_scheduler::JobDeliveryResult::failed(
                        Some(session_id),
                        format!("check scheduler delivery key: {err}"),
                    ),
                    Ok(false) if session_manager.is_run_active(session_id).await => {
                        agena_scheduler::JobDeliveryResult::skipped(
                            Some(session_id),
                            "session already has an active run",
                        )
                    }
                    Ok(false) => {
                        let session = match session_manager.get_session(session_id).await {
                            Ok(session) => session,
                            Err(err) => {
                                let result = agena_scheduler::JobDeliveryResult::failed(
                                    Some(session_id),
                                    err.to_string(),
                                );
                                self.notify_job_result(&session_manager, job, delivery, &result);
                                return result;
                            }
                        };

                        if session.blocked() {
                            agena_scheduler::JobDeliveryResult::skipped(
                                Some(session_id),
                                "session is blocked on permission or user input",
                            )
                        } else {
                            let options = match session_manager
                                .resolve_scheduled_run_options(session_id)
                                .await
                            {
                                Ok(options) => options,
                                Err(err) => {
                                    let result = agena_scheduler::JobDeliveryResult::failed(
                                        Some(session_id),
                                        err.to_string(),
                                    );
                                    self.notify_job_result(
                                        &session_manager,
                                        job,
                                        delivery,
                                        &result,
                                    );
                                    return result;
                                }
                            };

                            match session_manager
                                .submit_user_message(
                                    agena_runtime::SessionUserMessageRequest::new(
                                        session_id,
                                        options,
                                        vec![crate::message::PartContent::text(job.prompt.clone())],
                                    )
                                    .with_idempotency_key(delivery.delivery_key.clone()),
                                )
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
                }
            } else {
                agena_scheduler::JobDeliveryResult::failed(
                    None,
                    "scheduled job has no owner_session_id",
                )
            };
            self.notify_job_result(&session_manager, job, delivery, &result);
            result
        }
    }

    agena_runtime::compose_scheduler(
        database,
        Arc::new(SessionSink {
            session_manager: Arc::downgrade(&session_manager),
        }),
    )
}
use super::{
    Agent, Arc, ContextGovernor, DatabaseConnection, Path, PluginHost, ProviderRegistry,
    SessionManager, SessionProcessor, ToolExecutor,
};
use agena_domain::{ContextPolicy, PermissionAction, PermissionDecision, ToolInvocation};
use agena_runtime_contracts::agent::Agent as PermissionAgent;
use agena_tool::ToolPermissionCheck;

#[derive(Clone)]
struct McpRiskPermissionInspector {
    manager: Arc<agena_mcp_client::McpConnectionManager>,
}

impl agena_runtime_tools::tool::ExecutionPermissionInspector for McpRiskPermissionInspector {
    fn additional_checks(
        &self,
        invocation: &ToolInvocation,
        _agent: &PermissionAgent,
    ) -> Result<Vec<ToolPermissionCheck>, agena_runtime_tools::tool::ToolError> {
        if invocation.name != "agena.mcp.tools.call" {
            return Ok(Vec::new());
        }
        let value = serde_json::to_value(&invocation.input).map_err(|error| {
            agena_runtime_tools::tool::ToolError::InvalidInput(error.to_string())
        })?;
        let Some(server) = value.get("server").and_then(serde_json::Value::as_str) else {
            return Ok(Vec::new());
        };
        let Some(tool) = value.get("name").and_then(serde_json::Value::as_str) else {
            return Ok(Vec::new());
        };
        if !matches!(
            self.manager.cached_tool_risk(server, tool),
            agena_mcp_client::McpToolRisk::High
        ) {
            return Ok(Vec::new());
        }
        Ok(vec![ToolPermissionCheck {
            action: PermissionAction::Tool {
                tool_name: "agena.mcp.high_risk".to_owned(),
                qualifier: Some(format!("{server}/{tool}")),
            },
            decision: PermissionDecision::Ask {
                reason: format!(
                    "MCP tool '{server}/{tool}' advertises destructive or open-world effects"
                ),
            },
        }])
    }
}
