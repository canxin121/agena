use agena_storage::kernel::{
    ConfirmStorageCleanup, SessionStorageDetailQuery, SessionStorageQuery, StorageSelection,
};

use super::*;

pub async fn list_session_storage(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<SessionStorageQuery>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.service().list_session_storage(query)).await
}

pub async fn session_storage(
    State(state): State<AppState>,
    Path(session): Path<i64>,
    AxumQuery(query): AxumQuery<SessionStorageDetailQuery>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.service().session_storage(session, query)).await
}

pub async fn session_storage_operation(
    State(state): State<AppState>,
    Path(session): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.service().session_storage_operation(session)).await
}

pub async fn preview_session_storage(
    State(state): State<AppState>,
    Path(session): Path<i64>,
    Json(selection): Json<StorageSelection>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.service().preview_session_storage(session, selection)).await
}

pub async fn confirm_session_storage(
    State(state): State<AppState>,
    Path(session): Path<i64>,
    Json(confirmation): Json<ConfirmStorageCleanup>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .service()
            .confirm_session_storage(session, confirmation),
    )
    .await
}

pub async fn reclaim_database_storage(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.service().reclaim_database_storage()).await
}
