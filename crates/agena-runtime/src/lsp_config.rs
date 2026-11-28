#![allow(unused_imports)]

//! Runtime composition for the provider-neutral LSP configuration values.

use std::{path::Path, sync::Arc, time::Duration};

use agena_lsp::{LspClientOptions, PluginLspLaunch, PluginLspResolver};
use agena_plugin_host::{
    AgentPluginContext, PluginHost, PluginPackage, PluginsConfig, agent_plugin_data_path,
};
use agena_runtime_config::{
    LSP_PLUGIN_ID, LspConfig, LspServerConfig, LspServerDefaultsConfig, lsp_config_from_plugins,
};

pub(crate) use agena_runtime_config::{
    LspServerProcessConfig, LspServerRoutingConfig, LspServerSessionConfig,
};

/// Compose the optional LSP registry and its background registration guard.
pub fn compose_lsp_services(
    plugins: &PluginsConfig,
    workspace_root: &Path,
    originator: impl Into<String>,
    package_version: impl Into<String>,
) -> Result<
    (
        Option<Arc<agena_lsp::LspRegistry>>,
        Option<Arc<crate::AbortOnDrop>>,
    ),
    String,
> {
    let config = lsp_config_from_plugins(plugins)?;
    let enabled = plugins
        .list
        .get(LSP_PLUGIN_ID)
        .is_some_and(|entry| !entry.disabled());
    if !enabled {
        return Ok((None, None));
    }

    let registry = Arc::new(agena_lsp::LspRegistry::new(
        workspace_root.to_path_buf(),
        originator.into(),
        package_version.into(),
    ));
    let defaults = config.defaults;
    let entries = config.servers.into_iter().collect::<Vec<_>>();
    let registration_registry = Arc::clone(&registry);
    let registration = crate::spawn_registration_batch(entries, move |(name, entry)| {
        let registry = Arc::clone(&registration_registry);
        let defaults = defaults.clone();
        async move {
            registry
                .register(lsp_runtime_spec(name.clone(), &entry, &defaults))
                .await;
            tracing::info!(target: "agena::lsp", "registered LSP server '{name}' (lazy-spawn)");
        }
    });
    Ok((Some(registry), Some(Arc::new(registration))))
}

pub(crate) async fn register_agent_plugin_lsp(
    host: &PluginHost,
    registry: Option<&agena_lsp::LspRegistry>,
) {
    let Some(registry) = registry else {
        return;
    };
    for plugin in host.plugins() {
        let Some(package) = plugin.agent_plugin() else {
            continue;
        };
        let PluginPackage::AgentPlugin {
            environment,
            user_config_env,
            ..
        } = &plugin.configured_plugin.package
        else {
            continue;
        };
        let id = plugin.key().to_string();
        let context = Arc::new((
            package.root.clone(),
            id.clone(),
            environment.clone(),
            user_config_env.clone(),
        ));
        for server in &package.lsp_servers {
            let spec = agena_lsp::LspServerSpec {
                name: format!("{id}.{}", server.name),
                command: server.command.clone(),
                args: Vec::new(),
                env: Default::default(),
                file_extensions: server.language_ids.keys().cloned().collect(),
                root_markers: Vec::new(),
                initialization_options: server.initialization_options.clone(),
            };
            let server = server.clone();
            let spec_template = spec.clone();
            let context = Arc::clone(&context);
            let resolver: PluginLspResolver = Arc::new(move |workspace| {
                let (plugin_root, id, environment, user_config_env) = context.as_ref();
                let data = agent_plugin_data_path(workspace, id);
                let resolved = server.resolve(&AgentPluginContext {
                    root: plugin_root,
                    data: &data,
                    workspace,
                    environment,
                    user_config_env,
                })?;
                let mut reads = vec![plugin_root.clone()];
                if Path::new(&resolved.command).is_absolute() {
                    reads.push(resolved.command.as_str().into());
                }
                let mut spec = spec_template.clone();
                spec.command = resolved.command;
                spec.args = resolved.args;
                spec.env = resolved.env;
                Ok(PluginLspLaunch {
                    spec,
                    root: resolved.workspace,
                    reads,
                    options: LspClientOptions {
                        language_ids: server.language_ids.clone(),
                        settings: server.settings.clone(),
                        startup_timeout: server.startup_timeout.map(Duration::from_millis),
                        shutdown_timeout: server.shutdown_timeout.map(Duration::from_millis),
                        request_timeout: server.request_timeout.map(Duration::from_millis),
                    },
                })
            });
            registry.register_plugin(spec, resolver).await;
        }
    }
}

fn lsp_runtime_spec(
    name: String,
    config: &LspServerConfig,
    defaults: &LspServerDefaultsConfig,
) -> agena_lsp::LspServerSpec {
    let mut env = defaults.env.clone();
    env.extend(config.process.env.clone());
    let root_markers = if config.routing.root_markers.is_empty() {
        defaults.root_markers.clone()
    } else {
        config.routing.root_markers.clone()
    };
    agena_lsp::LspServerSpec {
        name,
        command: config.process.command.clone(),
        args: config.process.args.clone(),
        env,
        file_extensions: config.routing.file_extensions.clone(),
        root_markers,
        initialization_options: config
            .session
            .initialization_options
            .clone()
            .or_else(|| defaults.initialization_options.clone()),
    }
}
