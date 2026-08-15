pub async fn get_settings(
    State(state): State<AppState>,
    AxumQuery(input): AxumQuery<ConfigSettingsGetInput>,
) -> Result<impl IntoResponse, ServerError> {
    let configuration = state.config_json_sources().map_err(ServerError::from)?;
    let path = input.target.path.clone();
    let response = match input.source {
        ConfigSettingsSource::File => state
            .runtime_config_settings()
            .read_file_settings(input)
            .map_err(settings_error)?,
        ConfigSettingsSource::Effective => {
            let value = configuration.effective;
            let value = get_json_path(&value, path.as_deref()).map_err(|error| {
                ServerError::bad_request_with_diagnostic("The settings path is invalid.", error)
            })?;
            ConfigSettingsReadResponse {
                config_path: configuration.config_path,
                config_found: configuration.config_found,
                source: ConfigSettingsSource::Effective,
                path,
                value,
            }
        }
    };
    Ok(Json(response))
}

pub async fn get_layer_settings(
    State(state): State<AppState>,
    Path(layer): Path<String>,
    AxumQuery(mut input): AxumQuery<ConfigSettingsGetInput>,
) -> Result<impl IntoResponse, ServerError> {
    input.source = ConfigSettingsSource::File;
    let response = match parse_settings_layer(layer.as_str())? {
        ConfigSettingsLayer::Global => state.runtime_config_settings().read_file_settings(input),
        ConfigSettingsLayer::Workspace => state
            .runtime_config_settings()
            .read_project_file_settings(input),
    }
    .map_err(settings_error)?;
    Ok(Json(response))
}

pub async fn get_resolved_config(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(async { state.application().resolved_configuration_document() }).await
}

pub async fn list_settings(
    State(state): State<AppState>,
    AxumQuery(input): AxumQuery<ConfigSettingsListInput>,
) -> Result<impl IntoResponse, ServerError> {
    let configuration = state.config_json_sources().map_err(ServerError::from)?;
    let path = input.target.path.clone();
    let response = match input.source {
        ConfigSettingsSource::File => state
            .runtime_config_settings()
            .list_file_settings(input)
            .map_err(settings_error)?,
        ConfigSettingsSource::Effective => {
            let value = configuration.effective;
            let items =
                list_json_path(&value, path.as_deref(), input.recursive).map_err(settings_error)?;
            ConfigSettingsListResponse {
                config_path: configuration.config_path,
                config_found: configuration.config_found,
                source: ConfigSettingsSource::Effective,
                path,
                items,
            }
        }
    };
    Ok(Json(response))
}

pub async fn set_settings(
    State(state): State<AppState>,
    Json(input): Json<ConfigSettingsSetInput>,
) -> Result<impl IntoResponse, ServerError> {
    let mut response = state
        .runtime_config_settings()
        .set_file_setting(input)
        .map_err(settings_error)?;
    reload_settings_if_needed(&state, &mut response).await?;
    Ok(Json(response))
}

pub async fn set_layer_settings(
    State(state): State<AppState>,
    Path(layer): Path<String>,
    Json(input): Json<ConfigSettingsSetInput>,
) -> Result<impl IntoResponse, ServerError> {
    let mut response = match parse_settings_layer(layer.as_str())? {
        ConfigSettingsLayer::Global => state.runtime_config_settings().set_file_setting(input),
        ConfigSettingsLayer::Workspace => state
            .runtime_config_settings()
            .set_project_file_setting(input),
    }
    .map_err(settings_error)?;
    reload_settings_if_needed(&state, &mut response).await?;
    Ok(Json(response))
}

pub async fn patch_settings(
    State(state): State<AppState>,
    Json(input): Json<ConfigSettingsPatchInput>,
) -> Result<impl IntoResponse, ServerError> {
    let mut response = state
        .runtime_config_settings()
        .patch_file_settings(input)
        .map_err(settings_error)?;
    reload_settings_if_needed(&state, &mut response).await?;
    Ok(Json(response))
}

pub async fn delete_settings(
    State(state): State<AppState>,
    AxumQuery(input): AxumQuery<ConfigSettingsDeleteInput>,
) -> Result<impl IntoResponse, ServerError> {
    let mut response = state
        .runtime_config_settings()
        .delete_file_setting(input)
        .map_err(settings_error)?;
    reload_settings_if_needed(&state, &mut response).await?;
    Ok(Json(response))
}

pub async fn delete_layer_settings(
    State(state): State<AppState>,
    Path(layer): Path<String>,
    AxumQuery(input): AxumQuery<ConfigSettingsDeleteInput>,
) -> Result<impl IntoResponse, ServerError> {
    let mut response = match parse_settings_layer(layer.as_str())? {
        ConfigSettingsLayer::Global => state.runtime_config_settings().delete_file_setting(input),
        ConfigSettingsLayer::Workspace => state
            .runtime_config_settings()
            .delete_project_file_setting(input),
    }
    .map_err(settings_error)?;
    reload_settings_if_needed(&state, &mut response).await?;
    Ok(Json(response))
}

pub async fn validate_settings(
    State(state): State<AppState>,
    _input: Option<Json<ConfigSettingsValidateInput>>,
) -> Result<impl IntoResponse, ServerError> {
    let response = state
        .runtime_config_settings()
        .validate_file_settings(ConfigSettingsValidateInput::default())
        .map_err(settings_error)?;
    Ok(Json(response))
}

fn parse_settings_layer(layer: &str) -> Result<ConfigSettingsLayer, ServerError> {
    match layer.trim() {
        "global" => Ok(ConfigSettingsLayer::Global),
        "workspace" => Ok(ConfigSettingsLayer::Workspace),
        other => Err(ServerError::bad_request_with_diagnostic(
            "The settings layer must be `global` or `workspace`.",
            other,
        )),
    }
}
use super::{
    AppState, AxumQuery, ConfigSettingsDeleteInput, ConfigSettingsGetInput, ConfigSettingsLayer,
    ConfigSettingsListInput, ConfigSettingsListResponse, ConfigSettingsPatchInput,
    ConfigSettingsReadResponse, ConfigSettingsSetInput, ConfigSettingsSource,
    ConfigSettingsValidateInput, IntoResponse, Json, Path, ServerError, State, get_json_path,
    json_http, list_json_path, reload_settings_if_needed, settings_error,
};
