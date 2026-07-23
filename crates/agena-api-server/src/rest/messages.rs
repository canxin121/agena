pub async fn list_messages(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    AxumQuery(query): AxumQuery<MessageListQuery>,
) -> Result<impl IntoResponse, ServerError> {
    query_json(
        &state,
        agena_api::queries::Query::ListMessages(agena_api::queries::ListMessagesParams {
            session_id,
            cursor: query.pagination.cursor,
            limit: query.pagination.limit,
            parts: query.parts,
        }),
        |result| match result {
            agena_api::queries::QueryResult::Messages(value) => Some(value),
            _ => None,
        },
        "list messages returned unexpected query result",
    )
    .await
}

pub async fn get_message(
    State(state): State<AppState>,
    Path(message_id): Path<i64>,
    AxumQuery(query): AxumQuery<MessageDetailQuery>,
) -> Result<impl IntoResponse, ServerError> {
    query_json(
        &state,
        agena_api::queries::Query::GetMessage(agena_api::queries::GetMessageParams {
            message_id,
            parts: query.parts,
        }),
        |result| match result {
            agena_api::queries::QueryResult::Message(value) => Some(value),
            _ => None,
        },
        "get message returned unexpected query result",
    )
    .await
}

pub async fn list_message_parts(
    State(state): State<AppState>,
    Path(message_id): Path<i64>,
    AxumQuery(query): AxumQuery<MessagePartsQuery>,
) -> Result<impl IntoResponse, ServerError> {
    query_json(
        &state,
        agena_api::queries::Query::ListMessageParts(agena_api::queries::ListMessagePartsParams {
            message_id,
            mode: query.mode,
        }),
        |result| match result {
            agena_api::queries::QueryResult::MessageParts(value) => Some(value),
            _ => None,
        },
        "list message parts returned unexpected query result",
    )
    .await
}

pub async fn get_message_part(
    State(state): State<AppState>,
    Path(part_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    query_json(
        &state,
        agena_api::queries::Query::GetMessagePart(agena_api::queries::GetMessagePartParams {
            part_id,
        }),
        |result| match result {
            agena_api::queries::QueryResult::MessagePart(value) => Some(value),
            _ => None,
        },
        "get message part returned unexpected query result",
    )
    .await
}
use super::{
    AppState, AxumQuery, IntoResponse, MessageDetailQuery, MessageListQuery, MessagePartsQuery,
    Path, ServerError, State, query_json,
};
