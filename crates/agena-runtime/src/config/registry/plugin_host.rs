use std::path::Path;

use agena_plugin_host::PluginHost;
use agena_provider::ModelCatalogSnapshot;
use agena_runtime_config::{ConfigError, ProcessEnvironment, ResolvedProviderConfig};
use agena_runtime_provider::ProviderRegistry;

pub async fn build_provider_registry_from_inputs(
    providers: &std::collections::BTreeMap<String, ResolvedProviderConfig>,
    config_path: Option<&Path>,
    plugins: &PluginHost,
    catalog: Option<&ModelCatalogSnapshot>,
) -> Result<ProviderRegistry, ConfigError> {
    let mut registry =
        agena_runtime_provider::config_support::registry::build_provider_registry_from_configs(
            providers,
            catalog,
            &ProcessEnvironment,
            config_path,
        )?;
    let current = agena_runtime::provider_descriptors_from_ids(registry.provider_ids());
    let Some(patch) = agena_runtime::dispatch_provider_list_patch(plugins, current)
        .await
        .map_err(|err| ConfigError::Validation(format!("plugin provider.list: {err}")))?
    else {
        return Ok(registry);
    };
    agena_runtime::apply_provider_list_patch(&mut registry, patch)
        .map_err(|error| ConfigError::Validation(error.to_string()))?;
    Ok(registry)
}
