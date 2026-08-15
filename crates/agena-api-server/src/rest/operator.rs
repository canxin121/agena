pub async fn list_operator_tools(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(state.application().list_operator_tools().await))
}

pub async fn invoke_operator_tool(
    State(state): State<AppState>,
    Json(request): Json<OperatorToolInvokeRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let call_id = state.next_operator_call_id();
    json_http(state.application().invoke_operator_tool(
        request.workspace_id,
        request.tool.as_str(),
        request.input,
        call_id,
    ))
    .await
}

use super::{
    AppState, IntoResponse, Json, OperatorToolInvokeRequest, ServerError, State, json_http,
};
