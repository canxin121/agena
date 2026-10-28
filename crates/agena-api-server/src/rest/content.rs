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

/// Resolve only an immutable prepared result belonging to this session.
pub async fn read_inline_ui_reference(
    State(state): State<AppState>,
    Path((session_id, reference)): Path<(i64, String)>,
) -> Result<impl axum::response::IntoResponse, ServerError> {
    let document = state
        .application()
        .read_inline_ui_reference(session_id, &reference)
        .await?;
    let mut response = crate::json_codec::response(document).await?;
    // Membership may be revoked by rewind/deletion; do not lend an old authorized
    // response through browser or intermediary caches.
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("private, no-store"),
    );
    Ok(response)
}

pub async fn read_inline_ui_snapshot(
    State(state): State<AppState>,
    Path((session_id, reference)): Path<(i64, String)>,
) -> Result<impl axum::response::IntoResponse, ServerError> {
    let snapshot = state
        .application()
        .read_inline_ui_snapshot(session_id, &reference)
        .await?;
    let mut response = crate::json_codec::response(snapshot).await?;
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("private, no-store"),
    );
    Ok(response)
}

pub async fn stream_inline_ui(
    State(state): State<AppState>,
    Path((session_id, reference)): Path<(i64, String)>,
) -> Result<impl axum::response::IntoResponse, ServerError> {
    let app = state.application().clone();
    let mut subscription = app.watch_inline_ui(session_id, &reference).await?;
    let output = async_stream::stream! {
        let mut revision=0;
        // In-process writes notify immediately. Periodic rereads also recover
        // cross-process writes and revoke access after session deletion/rewind.
        let mut recovery=tokio::time::interval(std::time::Duration::from_secs(2));
        loop {
            match app.read_inline_ui_snapshot(session_id,&reference).await {
                Ok(snapshot) => if snapshot.revision!=revision {
                    revision=snapshot.revision;
                    yield Ok::<Event,Infallible>(Event::default().event("inline_ui").id(revision.to_string()).json_data(snapshot).expect("snapshot serializes"));
                },
                Err(error)=>{yield Ok(Event::default().event("inline_ui_error").json_data(ServerError::from(error).into_api()).expect("error serializes"));break;}
            }
            tokio::select!{
                result=subscription.changed()=>if result.is_err(){break;},
                _=recovery.tick()=>{},
            }
        }
    };
    use axum::response::IntoResponse as _;
    let mut response = axum::response::Sse::new(output)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("private, no-store, no-transform"),
    );
    response.headers_mut().insert(
        axum::http::header::HeaderName::from_static("x-accel-buffering"),
        axum::http::HeaderValue::from_static("no"),
    );
    Ok(response)
}
