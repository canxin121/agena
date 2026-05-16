use std::sync::Arc;

use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};

use crate::ApiResult;

#[derive(Debug, Deserialize)]
pub struct EnvCheckRequest {
    pub vars: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvCheckResponse {
    pub present: Vec<String>,
    pub missing: Vec<String>,
}

fn is_safe_env_name(name: &str) -> bool {
    let s = name.trim();
    if s.is_empty() || s.len() > 80 {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

pub async fn env_check_post(
    State(_state): State<Arc<crate::AppState>>,
    Json(body): Json<EnvCheckRequest>,
) -> ApiResult<Json<EnvCheckResponse>> {
    let mut present = Vec::<String>::new();
    let mut missing = Vec::<String>::new();

    // Avoid turning this endpoint into an arbitrary environment oracle.
    // This is still local-only, but restrict size and allowed characters.
    let mut uniq = std::collections::BTreeSet::<String>::new();
    for name in body.vars.into_iter().take(200) {
        let n = name.trim().to_string();
        if !is_safe_env_name(&n) {
            continue;
        }
        uniq.insert(n);
    }

    for name in uniq {
        let ok = std::env::var_os(&name)
            .and_then(|v| v.into_string().ok())
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false);
        if ok {
            present.push(name);
        } else {
            missing.push(name);
        }
    }

    Ok(Json(EnvCheckResponse { present, missing }))
}
