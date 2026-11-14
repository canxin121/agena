use super::{AppState, AxumQuery, IntoResponse, Json, Path, ServerError, State, json_http};
use agena_api::management::{SettingsCommand, SettingsQuery, settings::*};

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

pub async fn get_settings_sources(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<SettingsSourcesQuery>,
) -> Result<impl IntoResponse, ServerError> {
    if query.paths.len() > 16_384 {
        return Err(ServerError::bad_request(
            "The settings path batch is too large.",
        ));
    }
    let paths = serde_json::from_str(&query.paths)
        .map_err(|error| ServerError::bad_request_error(&error))?;
    json_http(
        state
            .application()
            .query_settings(SettingsQuery::Sources { paths }),
    )
    .await
}

pub async fn get_settings(
    State(state): State<AppState>,
    AxumQuery(input): AxumQuery<ConfigSettingsGetInput>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .query_settings(SettingsQuery::Get(input)),
    )
    .await
}

pub async fn get_layer_settings(
    State(state): State<AppState>,
    Path(layer): Path<String>,
    AxumQuery(input): AxumQuery<ConfigSettingsGetInput>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().query_settings(SettingsQuery::GetLayer {
        layer: parse_settings_layer(&layer)?,
        input,
    }))
    .await
}

pub async fn get_resolved_config(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().query_settings(SettingsQuery::Resolved)).await
}

pub async fn list_settings(
    State(state): State<AppState>,
    AxumQuery(input): AxumQuery<ConfigSettingsListInput>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .query_settings(SettingsQuery::List(input)),
    )
    .await
}

pub async fn set_settings(
    State(state): State<AppState>,
    Json(input): Json<ConfigSettingsSetInput>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .command_settings(SettingsCommand::Set(input)),
    )
    .await
}

pub async fn set_layer_settings(
    State(state): State<AppState>,
    Path(layer): Path<String>,
    Json(input): Json<ConfigSettingsSetInput>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .command_settings(SettingsCommand::SetLayer {
                layer: parse_settings_layer(&layer)?,
                input,
            }),
    )
    .await
}

pub async fn patch_settings(
    State(state): State<AppState>,
    Json(input): Json<ConfigSettingsPatchInput>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .command_settings(SettingsCommand::Patch(input)),
    )
    .await
}

pub async fn delete_settings(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<SettingsDeleteQuery>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .command_settings(SettingsCommand::Delete(query.into())),
    )
    .await
}

pub async fn delete_layer_settings(
    State(state): State<AppState>,
    Path(layer): Path<String>,
    AxumQuery(query): AxumQuery<SettingsDeleteQuery>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .command_settings(SettingsCommand::DeleteLayer {
                layer: parse_settings_layer(&layer)?,
                input: query.into(),
            }),
    )
    .await
}

pub async fn validate_settings(
    State(state): State<AppState>,
    input: Option<Json<ConfigSettingsValidateInput>>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().query_settings(SettingsQuery::Validate(
        input.map(|Json(input)| input).unwrap_or_default(),
    )))
    .await
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
