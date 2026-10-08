use super::{
    AppState, AxumQuery, ConfigSettingsDeleteInput, ConfigSettingsGetInput, ConfigSettingsLayer,
    ConfigSettingsListInput, ConfigSettingsListResponse, ConfigSettingsPatchInput,
    ConfigSettingsReadResponse, ConfigSettingsSetInput, ConfigSettingsSource,
    ConfigSettingsValidateInput, IntoResponse, Json, Path, ServerError, State, get_json_path,
    json_http, list_json_path, reload_settings_if_needed, settings_error,
};

static SETTINGS_WORK: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

#[derive(serde::Deserialize)]
pub struct SettingsSourcesQuery {
    paths: String,
}

#[derive(serde::Serialize)]
pub struct SettingSources {
    effective: ConfigSettingsReadResponse,
    file: ConfigSettingsReadResponse,
    global: ConfigSettingsReadResponse,
    workspace: ConfigSettingsReadResponse,
}

/// Dense settings panels read each source document once per bounded batch.
/// Select leaves on the server so credentials and unrelated large documents
/// never have to be transferred just to display a few scalar settings.
pub async fn get_settings_sources(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<SettingsSourcesQuery>,
) -> Result<axum::response::Response, ServerError> {
    if query.paths.len() > 16_384 {
        return Err(ServerError::bad_request(
            "The settings path batch is too large.",
        ));
    }
    let paths: Vec<String> = serde_json::from_str(&query.paths)
        .map_err(|error| ServerError::bad_request_error(&error))?;
    if paths.is_empty()
        || paths.len() > 64
        || paths
            .iter()
            .any(|path| path.trim().is_empty() || path.len() > 512)
    {
        return Err(ServerError::bad_request(
            "The settings batch needs 1 to 64 non-empty paths.",
        ));
    }
    for path in &paths {
        agena_domain::parse_json_path(path).map_err(|error| {
            ServerError::bad_request_with_diagnostic("The settings path is invalid.", error)
        })?;
    }
    let response = blocking_settings(move || {
        let configuration = state.config_json_sources().map_err(ServerError::from)?;
        let effective = ConfigSettingsReadResponse {
            revision: None,
            config_path: configuration.config_path,
            config_found: configuration.config_found,
            source: ConfigSettingsSource::Effective,
            path: None,
            value: configuration.effective,
        };
        let input = ConfigSettingsGetInput {
            source: ConfigSettingsSource::File,
            ..Default::default()
        };
        let global = state
            .runtime_config_settings()
            .read_file_settings(input.clone())
            .map_err(settings_error)?;
        let workspace = state
            .runtime_config_settings()
            .read_project_file_settings(input)
            .map_err(settings_error)?;
        let leaf = |root: &ConfigSettingsReadResponse,
                    path: &str|
         -> Result<ConfigSettingsReadResponse, ServerError> {
            Ok(ConfigSettingsReadResponse {
                revision: root.revision.clone(),
                config_path: root.config_path.clone(),
                config_found: root.config_found,
                source: root.source,
                path: Some(path.to_owned()),
                value: get_json_path(&root.value, Some(path)).map_err(|error| {
                    ServerError::bad_request_with_diagnostic("The settings path is invalid.", error)
                })?,
            })
        };
        let mut result = std::collections::BTreeMap::new();
        for path in paths {
            if result.contains_key(&path) {
                continue;
            }
            let global = leaf(&global, &path)?;
            result.insert(
                path.clone(),
                SettingSources {
                    effective: leaf(&effective, &path)?,
                    file: global.clone(),
                    global,
                    workspace: leaf(&workspace, &path)?,
                },
            );
        }
        Ok(result)
    })
    .await?;
    crate::json_codec::response(response).await
}

pub async fn get_settings(
    State(state): State<AppState>,
    AxumQuery(input): AxumQuery<ConfigSettingsGetInput>,
) -> Result<impl IntoResponse, ServerError> {
    let response = blocking_settings(move || {
        let path = input.target.path.clone();
        match input.source {
            ConfigSettingsSource::File => state
                .runtime_config_settings()
                .read_file_settings(input)
                .map_err(settings_error),
            ConfigSettingsSource::Effective => {
                let configuration = state.config_json_sources().map_err(ServerError::from)?;
                let value =
                    get_json_path(&configuration.effective, path.as_deref()).map_err(|error| {
                        ServerError::bad_request_with_diagnostic(
                            "The settings path is invalid.",
                            error,
                        )
                    })?;
                Ok(ConfigSettingsReadResponse {
                    revision: None,
                    config_path: configuration.config_path,
                    config_found: configuration.config_found,
                    source: ConfigSettingsSource::Effective,
                    path,
                    value,
                })
            }
        }
    })
    .await?;
    crate::json_codec::response(response).await
}

pub async fn get_layer_settings(
    State(state): State<AppState>,
    Path(layer): Path<String>,
    AxumQuery(mut input): AxumQuery<ConfigSettingsGetInput>,
) -> Result<impl IntoResponse, ServerError> {
    input.source = ConfigSettingsSource::File;
    let response = blocking_settings(move || {
        match parse_settings_layer(layer.as_str())? {
            ConfigSettingsLayer::Global => {
                state.runtime_config_settings().read_file_settings(input)
            }
            ConfigSettingsLayer::Workspace => state
                .runtime_config_settings()
                .read_project_file_settings(input),
        }
        .map_err(settings_error)
    })
    .await?;
    crate::json_codec::response(response).await
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
    let response = blocking_settings(move || {
        let path = input.target.path.clone();
        match input.source {
            ConfigSettingsSource::File => state
                .runtime_config_settings()
                .list_file_settings(input)
                .map_err(settings_error),
            ConfigSettingsSource::Effective => {
                let configuration = state.config_json_sources().map_err(ServerError::from)?;
                let items =
                    list_json_path(&configuration.effective, path.as_deref(), input.recursive)
                        .map_err(settings_error)?;
                Ok(ConfigSettingsListResponse {
                    revision: None,
                    config_path: configuration.config_path,
                    config_found: configuration.config_found,
                    source: ConfigSettingsSource::Effective,
                    path,
                    items,
                })
            }
        }
    })
    .await?;
    crate::json_codec::response(response).await
}

pub async fn set_settings(
    State(state): State<AppState>,
    Json(input): Json<ConfigSettingsSetInput>,
) -> Result<impl IntoResponse, ServerError> {
    let write_state = state.clone();
    let mut response = blocking_settings(move || {
        write_state
            .runtime_config_settings()
            .set_file_setting(input)
            .map_err(settings_error)
    })
    .await?;
    reload_settings_if_needed(&state, &mut response).await?;
    crate::json_codec::response(response).await
}

pub async fn set_layer_settings(
    State(state): State<AppState>,
    Path(layer): Path<String>,
    Json(input): Json<ConfigSettingsSetInput>,
) -> Result<impl IntoResponse, ServerError> {
    let write_state = state.clone();
    let mut response = blocking_settings(move || {
        match parse_settings_layer(layer.as_str())? {
            ConfigSettingsLayer::Global => write_state
                .runtime_config_settings()
                .set_file_setting(input),
            ConfigSettingsLayer::Workspace => write_state
                .runtime_config_settings()
                .set_project_file_setting(input),
        }
        .map_err(settings_error)
    })
    .await?;
    reload_settings_if_needed(&state, &mut response).await?;
    crate::json_codec::response(response).await
}

pub async fn patch_settings(
    State(state): State<AppState>,
    Json(input): Json<ConfigSettingsPatchInput>,
) -> Result<impl IntoResponse, ServerError> {
    let write_state = state.clone();
    let mut response = blocking_settings(move || {
        write_state
            .runtime_config_settings()
            .patch_file_settings(input)
            .map_err(settings_error)
    })
    .await?;
    reload_settings_if_needed(&state, &mut response).await?;
    crate::json_codec::response(response).await
}

pub async fn delete_settings(
    State(state): State<AppState>,
    AxumQuery(input): AxumQuery<ConfigSettingsDeleteInput>,
) -> Result<impl IntoResponse, ServerError> {
    let write_state = state.clone();
    let mut response = blocking_settings(move || {
        write_state
            .runtime_config_settings()
            .delete_file_setting(input)
            .map_err(settings_error)
    })
    .await?;
    reload_settings_if_needed(&state, &mut response).await?;
    crate::json_codec::response(response).await
}

pub async fn delete_layer_settings(
    State(state): State<AppState>,
    Path(layer): Path<String>,
    AxumQuery(input): AxumQuery<ConfigSettingsDeleteInput>,
) -> Result<impl IntoResponse, ServerError> {
    let write_state = state.clone();
    let mut response = blocking_settings(move || {
        match parse_settings_layer(layer.as_str())? {
            ConfigSettingsLayer::Global => write_state
                .runtime_config_settings()
                .delete_file_setting(input),
            ConfigSettingsLayer::Workspace => write_state
                .runtime_config_settings()
                .delete_project_file_setting(input),
        }
        .map_err(settings_error)
    })
    .await?;
    reload_settings_if_needed(&state, &mut response).await?;
    crate::json_codec::response(response).await
}

pub async fn validate_settings(
    State(state): State<AppState>,
    _input: Option<Json<ConfigSettingsValidateInput>>,
) -> Result<impl IntoResponse, ServerError> {
    let response = blocking_settings(move || {
        state
            .runtime_config_settings()
            .validate_file_settings(ConfigSettingsValidateInput::default())
            .map_err(settings_error)
    })
    .await?;
    crate::json_codec::response(response).await
}

async fn blocking_settings<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, ServerError> + Send + 'static,
) -> Result<T, ServerError> {
    SETTINGS_WORK
        .run(work)
        .await
        .map_err(|error| ServerError::internal_error(&error))?
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

#[cfg(test)]
mod blocking_tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn slow_settings_io_does_not_stop_the_async_executor() {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let read = tokio::spawn(blocking_settings(move || {
            started_tx.send(()).unwrap();
            release_rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
            Ok(())
        }));
        started_rx.await.unwrap();
        // The only async executor thread must run while synchronous IO waits.
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        release_tx.send(()).unwrap();
        read.await.unwrap().unwrap();
    }
}
