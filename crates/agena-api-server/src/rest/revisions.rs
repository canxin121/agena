use super::{AppState, AxumQuery, Json, ServerError, State};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
pub struct RevisionQuery {
    resources: String,
}

pub async fn resource_revisions(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<RevisionQuery>,
) -> Result<Json<BTreeMap<String, String>>, ServerError> {
    let keys: Vec<String> = serde_json::from_str(&query.resources)
        .map_err(|_| ServerError::bad_request("resources must be a JSON array of resource keys"))?;
    if keys.len() > 128 || keys.iter().any(|key| !valid_key(key)) {
        return Err(ServerError::bad_request(
            "At most 128 valid resource keys are allowed",
        ));
    }
    let index = state.revisions()?;
    index.refresh_durable(&state).await?;
    Ok(Json(
        keys.into_iter()
            .map(|key| {
                let token = index.token(&key);
                (key, token)
            })
            .collect(),
    ))
}

fn valid_key(key: &str) -> bool {
    if matches!(
        key,
        "sessions" | "workspaces" | "workspaces:catalog" | "activities"
    ) {
        return true;
    }
    if let Some(bucket) = key.strip_prefix("sessions:bucket:") {
        let bucket = bucket.strip_suffix(":count").unwrap_or(bucket);
        return matches!(
            bucket,
            "pinned" | "favorite" | "running" | "attention" | "recent"
        );
    }
    if let Some(id) = key.strip_prefix("activity:") {
        let id = id.strip_suffix(":logs").unwrap_or(id);
        return !id.is_empty()
            && id.len() <= 192
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'));
    }
    let fields: Vec<_> = key.split(':').collect();
    let id = fields
        .get(1)
        .and_then(|id| id.parse::<i64>().ok())
        .is_some_and(|id| id > 0);
    id && match fields.as_slice() {
        ["workspace", _, "sessions" | "stats"]
        | ["session", _, "state" | "files" | "plan" | "transcript"]
        | ["part", _] => true,
        ["part", _, "input" | "metadata" | "output"] => true,
        ["workspace", _, "sessions", "roots"] => true,
        ["workspace", _, "sessions", "parent", parent] => {
            parent.parse::<i64>().is_ok_and(|id| id > 0)
        }
        ["workspace", _, "sessions", "bucket", bucket] => matches!(
            *bucket,
            "pinned" | "favorite" | "running" | "attention" | "recent"
        ),
        _ => false,
    }
}
