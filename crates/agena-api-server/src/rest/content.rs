use std::convert::Infallible;
use std::time::Duration;

use agena_domain::{ContentCursor, ContentId};
use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use serde::Deserialize;

use crate::{AppState, error::ServerError};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentReadQuery {
    pub epoch: Option<uuid::Uuid>,
    pub after: Option<u64>,
    pub max_bytes: Option<usize>,
}

impl ContentReadQuery {
    fn cursor(&self) -> Result<Option<ContentCursor>, ServerError> {
        match (self.epoch, self.after) {
            (Some(epoch), Some(sequence)) => Ok(Some(ContentCursor { epoch, sequence })),
            (None, None) => Ok(None),
            _ => Err(ServerError::bad_request(
                "A content cursor requires epoch and after.",
            )),
        }
    }

    fn byte_budget(&self) -> usize {
        self.max_bytes.unwrap_or(64 * 1024).clamp(1, 1024 * 1024)
    }
}

pub async fn read_content(
    State(state): State<AppState>,
    Path((session_id, id)): Path<(i64, ContentId)>,
    Query(query): Query<ContentReadQuery>,
) -> Result<impl axum::response::IntoResponse, ServerError> {
    let page = state
        .application()
        .read_content(session_id, id, query.cursor()?, query.byte_budget())
        .await?;
    crate::json_codec::response(page).await
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentTextReadQuery {
    pub epoch: Option<uuid::Uuid>,
    pub after: Option<u64>,
    pub max_bytes: Option<usize>,
    #[serde(default)]
    pub offset: usize,
}

pub async fn read_content_text(
    State(state): State<AppState>,
    Path((session_id, resource_id)): Path<(i64, ContentId)>,
    Query(query): Query<ContentTextReadQuery>,
) -> Result<impl axum::response::IntoResponse, ServerError> {
    let read = ContentReadQuery {
        epoch: query.epoch,
        after: query.after,
        max_bytes: query.max_bytes,
    };
    let cursor = read.cursor()?;
    if cursor.is_none() && query.offset != 0 {
        return Err(ServerError::bad_request(
            "A text offset requires epoch and after.",
        ));
    }
    let page = state
        .application()
        .read_content_text(agena_api::content::ReadContentTextParams {
            session_id,
            resource_id,
            position: cursor.map(|after| agena_domain::ContentTextPosition {
                after,
                offset: query.offset,
            }),
            max_bytes: read.max_bytes.unwrap_or(64 * 1024),
        })
        .await?;
    crate::json_codec::response(page).await
}

pub async fn stream_content(
    State(state): State<AppState>,
    Path((session_id, id)): Path<(i64, ContentId)>,
    Query(query): Query<ContentReadQuery>,
) -> Result<impl axum::response::IntoResponse, ServerError> {
    let watch = state.application().watch_content(session_id, id).await?;
    let mut delivery = watch.delivery(query.cursor()?, query.byte_budget());
    let output = async_stream::stream! {
        loop {
            match delivery.next().await {
                Ok(Some(page)) => yield Ok::<Event, Infallible>(Event::default().event("content")
                    .json_data(page).expect("content page serializes")),
                Ok(None) => break,
                Err(error) => {
                    yield Ok::<Event, Infallible>(Event::default().event("content_error")
                        .json_data(ServerError::from(error).into_api()).expect("API error serializes"));
                    break;
                }
            }
        }
    };
    Ok(Sse::new(output).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}
