use std::sync::Arc;

/// Typed inputs crossing from runtime snapshot resolution into model-catalog
/// composition. The runtime crate owns the shape; concrete provider, plugin,
/// and database implementations remain supplied by the composition owner.
pub(crate) struct ModelCatalogCompositionInputs<Providers, ConfigPath, Plugins, Store> {
    pub(crate) providers: Providers,
    pub(crate) config_path: ConfigPath,
    pub(crate) plugins: Plugins,
    pub(crate) store: Store,
}

/// Typed inputs for plugin-host composition. The runtime owns the boundary;
/// concrete host/configuration types remain supplied by the caller.
pub(crate) struct PluginCompositionInputs<
    PluginConfig,
    Workspace,
    PreviousHost,
    PreviousConfig,
    Mcp,
> {
    pub(crate) plugin_config: PluginConfig,
    pub(crate) sandbox_enabled: bool,
    pub(crate) workspace_root: Workspace,
    pub(crate) previous_host: PreviousHost,
    pub(crate) previous_config: PreviousConfig,
    pub(crate) mcp_manager: Mcp,
    pub(crate) callback_base_url: String,
    pub(crate) callback_dispatcher: agena_plugin_host::PluginCallbackDispatcher,
    pub(crate) host_client: Arc<dyn agena_plugin_sdk::host_api::HostClient>,
}

/// Typed inputs for session/service composition. Concrete session, provider,
/// permission, and tool implementations remain outside this contract crate.
pub(crate) struct SessionCompositionInputs<
    Existing,
    Database,
    Providers,
    Plugins,
    Lsp,
    Workspace,
    Config,
> {
    pub(crate) existing: Existing,
    pub(crate) database: Database,
    pub(crate) providers: Providers,
    pub(crate) plugins: Plugins,
    pub(crate) lsp_registry: Lsp,
    pub(crate) workspace_root: Workspace,
    pub(crate) config: Config,
    pub(crate) mcp_manager: Option<Arc<agena_mcp_client::McpConnectionManager>>,
    /// Background-process registry installed into every tool executor built
    /// for this session manager.
    pub(crate) monitor_registry: Option<Arc<dyn crate::MonitorService>>,
    /// Scheduler writer and reader. `None` uses the in-memory store.
    pub(crate) scheduler_database: Option<crate::scheduler_composition::SchedulerDatabase>,
}

/// Default upper bound for concurrently executing tools in one session.
pub(crate) const DEFAULT_MAX_CONCURRENT_TOOLS: usize = 32;

/// Resolved session behavior passed into a concrete session-manager adapter.
///
/// This is configuration data only. Runtime composes the concrete session
/// manager, provider registry, and tool executor from it.
#[derive(Debug, Clone)]
pub(crate) struct RuntimeSessionBuildConfig {
    pub(crate) default_selection: agena_domain::ExecutionSelection,
    pub(crate) auxiliary_selection: agena_domain::ExecutionSelection,
    pub(crate) prompts: agena_runtime_config::PromptLayersConfig,
    pub(crate) profiles:
        std::collections::BTreeMap<String, agena_runtime_config::AgentProfileConfig>,
    pub(crate) default_profile: Option<String>,
    pub(crate) default_subagent_profile: Option<String>,
    pub(crate) auto_title: agena_runtime_config::SessionAutoTitleConfig,
    pub(crate) permission: agena_domain::PermissionConfig,
    pub(crate) access_control: agena_domain::AccessControlConfig,
    pub(crate) sandbox: bool,
    pub(crate) resource_guard: bool,
    pub(crate) auto_compaction: agena_domain::SessionAutoCompactionConfig,
    pub(crate) max_concurrent_tools: usize,
    /// Cap on model turns within one stable run; `None` uses the session
    /// manager's fallback (`DEFAULT_MAX_MODEL_TURNS`). Not yet wired to TOML;
    /// runtime keeps the default so behavior matches the session default.
    pub(crate) max_turns: Option<usize>,
}

/// Project the resolved configuration values needed by session composition.
/// Runtime owns this value-only mapping so snapshot code does not reconstruct
/// session policy.
pub(crate) fn session_build_config_from_resolved(
    config: &crate::ResolvedConfig,
    host: &agena_plugin_host::PluginHost,
) -> RuntimeSessionBuildConfig {
    RuntimeSessionBuildConfig {
        default_selection: config.default_selection.clone(),
        auxiliary_selection: config
            .runtime
            .auxiliary_model
            .as_ref()
            .map(|model| {
                let mut selection = agena_domain::ExecutionSelection {
                    provider: model.provider.clone(),
                    adapter: model.adapter.clone(),
                    model: model.model.clone(),
                    thinking_mode: model.thinking_mode.clone(),
                    speed_mode: model.speed_mode.clone(),
                    verbosity: model.verbosity.clone(),
                    parallel_tool_calls: model.parallel_tool_calls,
                    ..Default::default()
                };
                selection.normalize();
                selection
            })
            .unwrap_or_default(),
        prompts: config.prompts.clone(),
        profiles: profiles_with_agent_plugins(config, host),
        default_profile: config.default_profile.clone(),
        default_subagent_profile: config.default_subagent_profile.clone(),
        auto_title: config.session.auto_title.clone(),
        permission: config.permission.clone(),
        access_control: config.access_control.clone(),
        sandbox: config.sandbox,
        resource_guard: config.resource_guard,
        auto_compaction: agena_domain::SessionAutoCompactionConfig {
            enabled: config.session.compaction.auto,
            reserved_tokens: config.session.compaction.reserved_tokens,
        },
        max_concurrent_tools: DEFAULT_MAX_CONCURRENT_TOOLS,
        // `None` falls back to `DEFAULT_MAX_MODEL_TURNS` (500) in
        // agena-runtime-session; `Some(0)` means unlimited (handled in
        // `replies_execution.rs`).
        max_turns: config.session.max_turns,
    }
}

fn profiles_with_agent_plugins(
    config: &crate::ResolvedConfig,
    host: &agena_plugin_host::PluginHost,
) -> std::collections::BTreeMap<String, agena_runtime_config::AgentProfileConfig> {
    use agena_prompt_layers::{FragmentId, PromptFragment, PromptLayer};
    use agena_runtime_config::{AgentProfileConfig, AgentProfileMode};

    let mut profiles = config.profiles.clone();
    for plugin in host.plugins() {
        let Some(package) = plugin.agent_plugin() else {
            continue;
        };
        let key = plugin.key();
        for role in &package.roles {
            let name = if role.output_style {
                format!("{key}.style.{}", role.name)
            } else {
                format!("{key}.{}", role.name)
            };
            let make_profile = || -> Result<AgentProfileConfig, String> {
                let id = FragmentId::parse(&name).map_err(|error| error.to_string())?;
                let prompts = PromptLayer {
                    order: vec![id.clone()],
                    inline: vec![PromptFragment::new(id, &role.instructions)],
                    ..Default::default()
                };
                prompts.validate().map_err(|error| error.to_string())?;
                let model = role.model.as_deref().and_then(|requested| {
                    let route = resolve_imported_claude_model(config, requested);
                    if route.is_none() {
                        host.append_plugin_log(
                            key.to_string(),
                            "warn",
                            "agent_plugin.profile",
                            format!("Agent {name} inherits its model because {requested:?} has no unique enabled Agena route"),
                            serde_json::Value::Null,
                        );
                    }
                    route
                });
                Ok(AgentProfileConfig {
                    title: role.name.clone(),
                    model,
                    description: role.description.clone(),
                    mode: Some(if role.output_style {
                        AgentProfileMode::Primary
                    } else {
                        AgentProfileMode::Subagent
                    }),
                    prompts,
                    max_turns: role.max_turns,
                    worktree: role.worktree,
                    tool_allowlist: role.tool_allowlist.clone(),
                    tool_denials: role.tool_denials.clone(),
                    claude_agent: (!role.output_style)
                        .then(|| (key.clone(), role.claude_options.clone())),
                    ..Default::default()
                })
            };
            let profile = if profiles.contains_key(&name) {
                Err(format!("profile {name:?} already exists"))
            } else if profiles.len() >= agena_runtime_config::agent_profiles::MAX_AGENT_PROFILES {
                Err(format!(
                    "profile {name:?} exceeds the agent profile catalog limit"
                ))
            } else {
                make_profile()
            };
            match profile {
                Ok(profile) => {
                    profiles.insert(name, profile);
                }
                Err(error) => {
                    host.append_plugin_log(
                        key.to_string(),
                        "warn",
                        "agent_plugin.profile",
                        error,
                        serde_json::Value::Null,
                    );
                }
            }
        }
    }
    profiles
}

/// Select an existing route for a Claude role's exact model ID or family alias.
/// An equal ranking leaves the choice with the inherited Agena session route.
fn resolve_imported_claude_model(
    config: &crate::ResolvedConfig,
    requested: &str,
) -> Option<agena_domain::ModelSelectionConfig> {
    let family = matches!(requested, "sonnet" | "opus" | "haiku").then_some(requested);
    let preferred = &config.default_selection;
    let mut best_rank = None;
    let mut best = None;
    for (provider_id, provider) in config
        .providers
        .iter()
        .filter(|(_, provider)| provider.enabled)
    {
        for (route, definition) in provider.models.iter().filter(|(_, model)| model.enabled) {
            let Some((adapter, model)) = route.split_once('/') else {
                continue;
            };
            if !provider
                .adapters
                .get(adapter)
                .is_some_and(|adapter| adapter.enabled)
            {
                continue;
            }
            let catalog_id = definition
                .definition
                .catalog_model_id
                .as_deref()
                .unwrap_or(model);
            let exact = requested == model || requested == catalog_id;
            let matches_family = family.is_some_and(|family| {
                let mut parts = catalog_id.split(|ch: char| !ch.is_ascii_alphanumeric());
                parts.any(|part| part == "claude") && parts.any(|part| part == family)
            });
            if !exact && !matches_family {
                continue;
            }
            let rank = (
                exact,
                preferred.provider.as_ref() == Some(provider_id),
                preferred.adapter.as_deref() == Some(adapter),
                preferred.model.as_deref() == Some(model),
            );
            match best_rank {
                Some(previous) if previous > rank => {}
                Some(previous) if previous == rank => best = None,
                _ => {
                    best_rank = Some(rank);
                    best = Some((provider_id, adapter, model));
                }
            }
        }
    }
    let (provider, adapter, model) = best?;
    Some(agena_domain::ModelSelectionConfig {
        provider: Some(provider.clone()),
        adapter: Some(adapter.to_owned()),
        model: Some(model.to_owned()),
        ..Default::default()
    })
}

/// Typed inputs for tool-executor construction.
pub(crate) struct ToolCompositionInputs<Plugins, Lsp, Workspace, Session> {
    pub(crate) sandbox: bool,
    pub(crate) resource_guard: bool,
    pub(crate) plugins: Plugins,
    pub(crate) lsp_registry: Lsp,
    pub(crate) workspace_root: Workspace,
    pub(crate) session_manager: Session,
    /// Background-process registry installed into the tool executor. When
    /// `None` the executor lazily builds its default registry.
    pub(crate) monitor_registry: Option<Arc<dyn crate::MonitorService>>,
    /// Scheduler writer and reader. `None` uses the in-memory store.
    pub(crate) scheduler_database: Option<crate::scheduler_composition::SchedulerDatabase>,
}

/// Typed inputs for database connection/schema composition. Runtime owns the
/// shape and ownership/initialization choreography; concrete URL resolution,
/// tracing, and schema code remain with the composition owner.
pub(crate) struct DatabaseCompositionInputs {
    pub(crate) database_url: Option<String>,
    pub(crate) database_path: Option<std::path::PathBuf>,
}

/// Fully assembled inputs for one immutable runtime snapshot. The Runtime owns
/// snapshot metadata/state assembly while a concrete composition adapter
/// supplies its resolved configuration and service implementations.
pub(crate) struct RuntimeSnapshotCompositionInputs<Resolution, Services> {
    pub(crate) generation: u64,
    pub(crate) resolution: Resolution,
    pub(crate) services: Services,
    pub(crate) tasks: crate::RuntimeTaskState,
}

pub(crate) fn compose_runtime_snapshot_state<Resolution, Services>(
    inputs: RuntimeSnapshotCompositionInputs<Resolution, Services>,
) -> crate::RuntimeSnapshotState<Resolution, Services> {
    crate::RuntimeSnapshotState::new(
        inputs.generation,
        inputs.resolution,
        inputs.services,
        inputs.tasks,
    )
}

/// Start the long-lived maintenance loops owned by one composed runtime.
/// Concrete adapters supply the loop futures; Runtime owns their task-control
/// registration and shared shutdown lifecycle.
pub(crate) fn spawn_runtime_maintenance_loops<Janitor, Reload, ClientVersions, ModelCatalog>(
    task_control: &crate::TaskControl,
    janitor: Janitor,
    reload: Reload,
    client_versions: ClientVersions,
    model_catalog: ModelCatalog,
) where
    Janitor: std::future::Future<Output = ()> + Send + 'static,
    Reload: std::future::Future<Output = ()> + Send + 'static,
    ClientVersions: std::future::Future<Output = ()> + Send + 'static,
    ModelCatalog: std::future::Future<Output = ()> + Send + 'static,
{
    task_control.spawn(janitor);
    task_control.spawn(reload);
    task_control.spawn(client_versions);
    task_control.spawn(model_catalog);
}

/// Runtime policy for model-catalog cache freshness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ModelCatalogRuntimeConfig {
    pub(crate) cache_max_age_secs: u64,
}

impl Default for ModelCatalogRuntimeConfig {
    fn default() -> Self {
        Self {
            cache_max_age_secs: 60 * 60 * 24 * 7,
        }
    }
}
