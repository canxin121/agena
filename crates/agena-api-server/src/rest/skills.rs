use agena_api::skills::{
    SkillCreateRequest, SkillEnabledRequest, SkillInstallRequest, SkillResourceWriteRequest,
    SkillUpdateRequest, SkillValidationRequest,
};

use super::*;

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SkillWorkspaceQuery {
    pub workspace_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillRevisionQuery {
    pub workspace_id: Option<i64>,
    pub expected_revision: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillResourceQuery {
    pub workspace_id: Option<i64>,
    pub path: String,
    pub expected_revision: Option<String>,
}

pub async fn list_skills(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<SkillWorkspaceQuery>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().list_skills(query.workspace_id)).await
}

pub async fn get_skill(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<SkillWorkspaceQuery>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.application().get_skill(query.workspace_id, id)).await
}

pub async fn create_skill(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<SkillWorkspaceQuery>,
    Json(request): Json<SkillCreateRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .create_skill(query.workspace_id, request),
    )
    .await
}

pub async fn update_skill(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<SkillWorkspaceQuery>,
    Path(id): Path<String>,
    Json(request): Json<SkillUpdateRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .update_skill(query.workspace_id, id, request),
    )
    .await
}

pub async fn delete_skill(
    State(state): State<AppState>,
    Path(id): Path<String>,
    AxumQuery(query): AxumQuery<SkillRevisionQuery>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .delete_skill(query.workspace_id, id, query.expected_revision),
    )
    .await
}

pub async fn set_skill_enabled(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<SkillWorkspaceQuery>,
    Path(id): Path<String>,
    Json(request): Json<SkillEnabledRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .set_skill_enabled(query.workspace_id, id, request),
    )
    .await
}

pub async fn get_skill_resource(
    State(state): State<AppState>,
    Path(id): Path<String>,
    AxumQuery(query): AxumQuery<SkillResourceQuery>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .get_skill_resource(query.workspace_id, id, query.path),
    )
    .await
}

pub async fn write_skill_resource(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<SkillWorkspaceQuery>,
    Path(id): Path<String>,
    Json(request): Json<SkillResourceWriteRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .write_skill_resource(query.workspace_id, id, request),
    )
    .await
}

pub async fn delete_skill_resource(
    State(state): State<AppState>,
    Path(id): Path<String>,
    AxumQuery(query): AxumQuery<SkillResourceQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let revision = query.expected_revision.ok_or_else(|| {
        ServerError::bad_request(
            "A package or resource revision is required. Refresh before deleting.",
        )
    })?;
    json_http(state.application().delete_skill_resource(
        query.workspace_id,
        id,
        query.path,
        revision,
    ))
    .await
}

pub async fn validate_skill(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<SkillWorkspaceQuery>,
    Json(request): Json<SkillValidationRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .validate_skill(query.workspace_id, request),
    )
    .await
}

pub async fn preview_skill_install(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<SkillWorkspaceQuery>,
    Json(request): Json<SkillInstallRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .preview_skill_install(query.workspace_id, request),
    )
    .await
}

pub async fn install_skills(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<SkillWorkspaceQuery>,
    Json(request): Json<SkillInstallRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(
        state
            .application()
            .install_skills(query.workspace_id, request),
    )
    .await
}
