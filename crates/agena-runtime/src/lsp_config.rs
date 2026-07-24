#![allow(unused_imports)]

//! Runtime composition for the provider-neutral LSP configuration values.

use std::{path::Path, sync::Arc};

use agena_plugin_host::PluginsConfig;
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
