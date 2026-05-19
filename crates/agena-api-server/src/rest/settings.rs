use super::*;

pub async fn get_settings(
    State(state): State<AppState>,
    AxumQuery(input): AxumQuery<ConfigSettingsGetInput>,
) -> Result<impl IntoResponse, ServerError> {
    let resolution = state.runtime().config_resolution();
    let response = match input.source {
        ConfigSettingsSource::File => {
            read_file_setting(resolution.meta.config_path.clone(), input).map_err(settings_error)?
        }
        ConfigSettingsSource::Effective => {
            let value = resolved_config_json(&resolution.config)?;
            let value = get_json_path(&value, input.path.as_deref()).map_err(settings_error)?;
            ConfigSettingsReadResponse {
                config_path: resolution.meta.config_path.clone(),
                config_found: resolution.meta.config_found,
                source: ConfigSettingsSource::Effective,
                path: input.path,
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
    let resolution = state.runtime().config_resolution();
    let response = match input.source {
        ConfigSettingsSource::File => {
            list_file_settings(resolution.meta.config_path.clone(), input)
                .map_err(settings_error)?
        }
        ConfigSettingsSource::Effective => {
            let value = resolved_config_json(&resolution.config)?;
            let entries = list_json_path(&value, input.path.as_deref(), input.recursive)
                .map_err(settings_error)?;
            ConfigSettingsListResponse {
                config_path: resolution.meta.config_path.clone(),
                config_found: resolution.meta.config_found,
                source: ConfigSettingsSource::Effective,
                path: input.path,
                entries,
            }
        }
    };
    Ok(Json(response))
}

pub async fn set_settings(
    State(state): State<AppState>,
    Json(input): Json<ConfigSettingsSetInput>,
) -> Result<impl IntoResponse, ServerError> {
    let config_path = state.runtime().config_resolution().meta.config_path.clone();
    let mut response = set_file_setting(config_path, input).map_err(settings_error)?;
    reload_settings_if_needed(&state, &mut response).await?;
    Ok(Json(response))
}

pub async fn patch_settings(
    State(state): State<AppState>,
    Json(input): Json<ConfigSettingsPatchInput>,
) -> Result<impl IntoResponse, ServerError> {
    let config_path = state.runtime().config_resolution().meta.config_path.clone();
    let mut response = patch_file_settings(config_path, input).map_err(settings_error)?;
    reload_settings_if_needed(&state, &mut response).await?;
    Ok(Json(response))
}

pub async fn delete_settings(
    State(state): State<AppState>,
    AxumQuery(input): AxumQuery<ConfigSettingsDeleteInput>,
) -> Result<impl IntoResponse, ServerError> {
    let config_path = state.runtime().config_resolution().meta.config_path.clone();
    let mut response = delete_file_setting(config_path, input).map_err(settings_error)?;
    reload_settings_if_needed(&state, &mut response).await?;
    Ok(Json(response))
}

pub async fn validate_settings(
    State(state): State<AppState>,
    _input: Option<Json<ConfigSettingsValidateInput>>,
) -> Result<impl IntoResponse, ServerError> {
    let config_path = state.runtime().config_resolution().meta.config_path.clone();
    let response = validate_file_settings(config_path).map_err(settings_error)?;
    Ok(Json(response))
}
