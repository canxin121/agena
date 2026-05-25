use agena::config::{
    ProcessEnvironment, draft_provider_adapter_models_target,
    list_provider_adapter_models_with_config, saved_provider_adapter_models_target,
};
use agena_api::{
    queries::{ListProviderAdapterModelsParams, ListSavedProviderAdapterModelsParams},
    resource::{
        ProviderAdapterModelsResponse, ProviderAdapterSummaryResource, ProviderDefaultsResource,
        ProviderModelsResponse, ProviderNativeToolBindingResource,
        ProviderNativeToolsSummaryResource, ProviderSummaryResource,
    },
};

use crate::{error::ServerError, state::AppState};

pub fn list_providers_response(state: &AppState) -> Vec<ProviderSummaryResource> {
    let snapshot = state.runtime().current_snapshot();
    let registry = snapshot.provider_registry();
    let mut providers = registry
        .provider_ids()
        .into_iter()
        .filter_map(|provider_id| {
            registry.get(provider_id.as_str()).map(|provider| {
                let provider_config = snapshot
                    .config_resolution()
                    .config
                    .providers
                    .get(provider_id.as_str());
                let adapters = provider_config
                    .map(|provider| {
                        provider
                            .adapters
                            .iter()
                            .map(|(adapter_id, adapter)| ProviderAdapterSummaryResource {
                                adapter_id: adapter_id.clone(),
                                enabled: adapter.enabled,
                                configured_model_count: provider
                                    .models
                                    .keys()
                                    .filter(|model_id| {
                                        model_id
                                            .split_once('/')
                                            .map(|(route_adapter_id, _)| {
                                                route_adapter_id == adapter_id
                                            })
                                            .unwrap_or(false)
                                    })
                                    .count(),
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                ProviderSummaryResource {
                    defaults: ProviderDefaultsResource {
                        adapter: provider.default_adapter().map(ToString::to_string),
                        model: provider.default_model().to_string(),
                    },
                    adapters,
                    native_tools: provider_config.map(provider_native_tools_summary_resource),
                    provider_id,
                }
            })
        })
        .collect::<Vec<_>>();
    providers.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
    providers
}

fn provider_native_tools_summary_resource(
    provider: &agena::config::ResolvedProviderConfig,
) -> ProviderNativeToolsSummaryResource {
    let (enabled, bindings) = provider
        .defaults
        .adapter
        .as_ref()
        .zip(provider.defaults.model.as_ref())
        .and_then(|(adapter_id, model_id)| {
            provider
                .models
                .get(format!("{adapter_id}/{model_id}").as_str())
        })
        .map(|model| (model.native_tools.enabled, model.native_tool_bindings()))
        .unwrap_or((false, Vec::new()));
    ProviderNativeToolsSummaryResource {
        enabled,
        model_count: provider
            .models
            .values()
            .filter(|model| model.native_tools.enabled)
            .count(),
        bindings: bindings
            .into_iter()
            .map(|binding| ProviderNativeToolBindingResource {
                tool: binding.tool.config_key().to_owned(),
                route: serde_json::to_string(&binding.route)
                    .unwrap_or_default()
                    .trim_matches('"')
                    .to_owned(),
            })
            .collect(),
    }
}

pub async fn list_provider_models_response(
    state: &AppState,
    provider_id: String,
) -> Result<ProviderModelsResponse, ServerError> {
    let snapshot = state.runtime().current_snapshot();
    if snapshot
        .provider_registry()
        .get(provider_id.as_str())
        .is_none()
    {
        return Err(ServerError::NotFound(format!(
            "provider {provider_id} not found"
        )));
    }

    let models = snapshot
        .list_provider_models(provider_id.as_str())
        .await
        .map_err(ServerError::Core)?;
    Ok(ProviderModelsResponse {
        provider_id,
        models,
    })
}

pub async fn list_provider_adapter_models_response(
    state: &AppState,
    params: ListProviderAdapterModelsParams,
) -> Result<ProviderAdapterModelsResponse, ServerError> {
    let target = draft_provider_adapter_models_target(
        params.provider_id.as_deref(),
        params.base_url.as_str(),
        params.protocol_paths,
        params.api_key.as_deref(),
        params.api_key_env.as_deref(),
        &params.adapter_ids,
    )
    .map_err(map_provider_adapter_models_config_error)?;
    list_provider_adapter_models(state, target).await
}

pub async fn list_saved_provider_adapter_models_response(
    state: &AppState,
    params: ListSavedProviderAdapterModelsParams,
) -> Result<ProviderAdapterModelsResponse, ServerError> {
    let snapshot = state.runtime().current_snapshot();
    let resolved = snapshot
        .config_resolution()
        .config
        .providers
        .get(params.provider_id.as_str())
        .ok_or_else(|| {
            ServerError::NotFound(format!("provider {} not found", params.provider_id))
        })?;
    let target = saved_provider_adapter_models_target(
        params.provider_id.as_str(),
        resolved,
        &params.adapter_ids,
    )
    .map_err(map_provider_adapter_models_config_error)?;
    list_provider_adapter_models(state, target).await
}

async fn list_provider_adapter_models(
    state: &AppState,
    target: agena::config::ProviderAdapterModelsTarget,
) -> Result<ProviderAdapterModelsResponse, ServerError> {
    let resolution = state.runtime().config_resolution();
    let adapter_models =
        list_provider_adapter_models_with_config(&resolution.config, &target, &ProcessEnvironment)
            .await
            .map_err(ServerError::Core)?;
    Ok(ProviderAdapterModelsResponse {
        provider_id: adapter_models.provider_id,
        adapters: adapter_models
            .adapters
            .into_iter()
            .map(Into::into)
            .collect(),
    })
}

fn map_provider_adapter_models_config_error(error: agena::config::ConfigError) -> ServerError {
    match error {
        agena::config::ConfigError::Validation(message)
        | agena::config::ConfigError::App(agena::AppError::Config(message)) => {
            ServerError::BadRequest(message)
        }
        other => ServerError::BadRequest(other.to_string()),
    }
}
