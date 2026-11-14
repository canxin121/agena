//! HTTP extraction and response encoding for shared marketplace operations.
use super::{AppState, IntoResponse, Json, ServerError, State, json_http};
use agena_application::dto::*;

pub async fn search_marketplace_plugins(
    State(state): State<AppState>,
    Json(request): Json<MarketplaceSearchRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().search_marketplace_plugins(request)).await
}

pub async fn sync_marketplace_registry(
    State(state): State<AppState>,
    Json(request): Json<MarketplaceRegistryRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().sync_marketplace_registry(request)).await
}

pub async fn list_marketplace_installed_plugins(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().list_marketplace_installed_plugins()).await
}

pub async fn list_marketplace_outdated_plugins(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().list_marketplace_outdated_plugins()).await
}

pub async fn install_marketplace_plugin(
    State(state): State<AppState>,
    Json(request): Json<MarketplaceInstallRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().install_marketplace_plugin(request)).await
}

pub async fn uninstall_marketplace_plugin(
    State(state): State<AppState>,
    Json(request): Json<MarketplaceUninstallRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().uninstall_marketplace_plugin(request)).await
}

pub async fn upgrade_marketplace_plugins(
    State(state): State<AppState>,
    Json(request): Json<MarketplaceUpgradeRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().upgrade_marketplace_plugins(request)).await
}
