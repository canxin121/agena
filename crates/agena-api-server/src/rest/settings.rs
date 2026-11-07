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

/// URL query fields stay flat so the query deserializer parses each boolean
/// before converting to the shared settings DTO. Deserializing its flattened
/// options directly buffers URL values as strings and rejects explicit booleans.
#[derive(Debug, serde::Deserialize)]
pub struct SettingsDeleteQuery {
    path: String,
    expected_revision: Option<String>,
    dry_run: Option<bool>,
    validate: Option<bool>,
    reload: Option<bool>,
}

impl From<SettingsDeleteQuery> for ConfigSettingsDeleteInput {
    fn from(query: SettingsDeleteQuery) -> Self {
        Self {
            path: query.path,
            options: agena_runtime::ConfigSettingsEditOptions {
                expected_revision: query.expected_revision,
                dry_run: query.dry_run.unwrap_or(false),
                validate: query.validate.unwrap_or(true),
                reload: query.reload.unwrap_or(true),
            },
        }
    }
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
    AxumQuery(query): AxumQuery<SettingsDeleteQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let input = ConfigSettingsDeleteInput::from(query);
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
    AxumQuery(query): AxumQuery<SettingsDeleteQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let input = ConfigSettingsDeleteInput::from(query);
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
mod tests {
    use super::*;

    fn parse_delete_query(uri: &str) -> Result<ConfigSettingsDeleteInput, String> {
        let uri = uri.parse::<axum::http::Uri>().unwrap();
        AxumQuery::<SettingsDeleteQuery>::try_from_uri(&uri)
            .map(|AxumQuery(query)| query.into())
            .map_err(|error| error.to_string())
    }

    #[test]
    fn delete_settings_url_query_preserves_explicit_booleans_and_revision() {
        for endpoint in [
            "/api/v1/settings",
            "/api/v1/settings/layers/global",
            "/api/v1/settings/layers/workspace",
        ] {
            for dry_run in [false, true] {
                for validate in [false, true] {
                    for reload in [false, true] {
                        let input = parse_delete_query(&format!(
                            "{endpoint}?path=plugins.list.%22test.echo%22&dry_run={dry_run}&validate={validate}&reload={reload}&expected_revision=revision%2Bwith%2Bsymbols"
                        )).expect("the real Axum query extractor must accept URL booleans");
                        assert_eq!(input.path, "plugins.list.\"test.echo\"");
                        assert_eq!(input.options.dry_run, dry_run);
                        assert_eq!(input.options.validate, validate);
                        assert_eq!(input.options.reload, reload);
                        assert_eq!(
                            input.options.expected_revision.as_deref(),
                            Some("revision+with+symbols")
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn delete_settings_url_query_keeps_defaults_and_rejects_invalid_options() {
        let input = parse_delete_query("/api/v1/settings?path=session.max_turns").unwrap();
        assert!(!input.options.dry_run);
        assert!(input.options.validate);
        assert!(input.options.reload);
        assert_eq!(input.options.expected_revision, None);
        for option in ["dry_run", "validate", "reload"] {
            assert!(
                parse_delete_query(&format!(
                    "/api/v1/settings?path=session.max_turns&{option}=not-a-boolean"
                ))
                .is_err()
            );
        }
        assert!(parse_delete_query("/api/v1/settings?reload=false").is_err());
    }
}
