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
use super::{
    AppState, AxumQuery, ConfigSettingsDeleteInput, ConfigSettingsGetInput,
    ConfigSettingsListInput, ConfigSettingsListResponse, ConfigSettingsPatchInput,
    ConfigSettingsReadResponse, ConfigSettingsSetInput, ConfigSettingsSource,
    ConfigSettingsValidateInput, IntoResponse, Json, ServerError, State, get_json_path,
    list_json_path, reload_settings_if_needed, settings_error,
};
