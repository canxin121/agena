//! Generic run membership windows. Display policies belong to clients.
use crate::{error::ServerError, state::AppState};
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunWindowQuery {
    pub sections: Option<String>,
    pub limit: Option<u64>,
    pub part_limit: Option<u64>,
    pub cursor: Option<String>,
    /// UI language for human headlines. Stored part data stays English.
    #[serde(default)]
    pub locale: Option<String>,
}

pub async fn read_session_runs(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    Query(query): Query<RunWindowQuery>,
    headers: HeaderMap,
) -> Result<impl axum::response::IntoResponse, ServerError> {
    let read =
        crate::revisions::ConditionalRead::new(&state, &format!("session:{session_id}:parts"))
            .await?;
    if let Some(response) = read.not_modified(&headers) {
        return Ok(response);
    }
    let mut resource = state
        .application()
        .read_runs(agena_api::queries::ReadRunsParams {
            session_id,
            limit: query.limit,
            part_limit: query.part_limit,
            cursor: query.cursor,
            sections: query
                .sections
                .as_deref()
                .map(|v| v.split(',').map(str::parse).collect::<Result<Vec<_>, _>>())
                .transpose()
                .map_err(|_| ServerError::bad_request("Unknown Part section."))?
                .unwrap_or_default(),
        })
        .await?;
    agena_application::part_locale::localize_part_headlines(
        &mut resource.parts,
        query.locale.as_deref(),
    );
    read.json(resource).await
}
