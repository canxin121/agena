use std::{
    collections::HashSet,
    env,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use axum::{
    Json, Router,
    http::{StatusCode, Uri},
    routing::any,
};
use serde_json::json;
use tower_http::services::{ServeDir, ServeFile};

const INDEX_FILE: &str = "index.html";

/// Resolve the production frontend directory.
///
/// Explicit configuration is strict because silently falling back to API-only
/// mode would make a configured deployment appear healthy while its UI is
/// unavailable. Without `--ui-dir`, repository and packaged layouts are
/// probed so local `agena server start` needs no additional argument.
pub(crate) fn resolve_ui_dir(
    explicit: Option<&Path>,
    workspace_root: &Path,
) -> Result<Option<PathBuf>> {
    if let Some(directory) = explicit {
        return validate_ui_dir(directory).map(Some);
    }

    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    push_candidate(
        &mut candidates,
        &mut seen,
        workspace_root.join("packages/agena-web/dist"),
    );
    if let Ok(current_dir) = env::current_dir() {
        push_candidate(
            &mut candidates,
            &mut seen,
            current_dir.join("packages/agena-web/dist"),
        );
        push_candidate(&mut candidates, &mut seen, current_dir.join("web-dist"));
    }
    if let Ok(executable) = env::current_exe()
        && let Some(install_root) = executable.parent().and_then(Path::parent)
    {
        push_candidate(&mut candidates, &mut seen, install_root.join("web-dist"));
    }

    for directory in candidates {
        if !directory.join(INDEX_FILE).is_file() {
            continue;
        }
        match std::fs::canonicalize(&directory) {
            Ok(directory) => return Ok(Some(directory)),
            Err(error) => {
                tracing::warn!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        format!("resolve discovered web UI directory {}", directory.display()),
                        &error,
                    ),
                    "discovered web frontend could not be used"
                );
            }
        }
    }
    Ok(None)
}

fn validate_ui_dir(directory: &Path) -> Result<PathBuf> {
    if !directory.join(INDEX_FILE).is_file() {
        bail!(
            "web UI directory {} does not contain {INDEX_FILE}; run the frontend production build first",
            directory.display()
        );
    }
    std::fs::canonicalize(directory)
        .with_context(|| format!("failed to resolve web UI directory {}", directory.display()))
}

fn push_candidate(candidates: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>, candidate: PathBuf) {
    if seen.insert(candidate.clone()) {
        candidates.push(candidate);
    }
}

pub(crate) fn attach(app: Router, ui_dir: Option<PathBuf>) -> Router {
    // Unknown server endpoints must stay 404 in both UI and API-only mode.
    // Otherwise a removed OAuth endpoint can look successful by serving the
    // SPA shell (or the API-only landing response) with HTTP 200.
    let app = app
        .route("/api", any(api_not_found))
        .route("/api/{*path}", any(api_not_found))
        .route("/auth", any(api_not_found))
        .route("/auth/{*path}", any(api_not_found))
        .route("/oauth", any(api_not_found))
        .route("/oauth/{*path}", any(api_not_found))
        .route("/.well-known", any(api_not_found))
        .route("/.well-known/{*path}", any(api_not_found));
    match ui_dir {
        Some(directory) => {
            tracing::info!(
                target: "agena.runtime",
                ui_dir = %directory.display(),
                "serving the web frontend from the Agena server"
            );
            let index = directory.join(INDEX_FILE);
            app.fallback_service(ServeDir::new(directory).fallback(ServeFile::new(index)))
        }
        None => {
            tracing::warn!(
                target: "agena.runtime",
                "web frontend build not found; server is running in API-only mode"
            );
            app.fallback(api_only_fallback)
        }
    }
}

pub(crate) async fn api_not_found(uri: Uri) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(json!({
            "error": {
                "code": "route_not_found",
                "message": format!("No server route matches {}", uri.path()),
            }
        })),
    )
}

async fn api_only_fallback() -> Json<serde_json::Value> {
    Json(json!({
        "service": "agena",
        "message": "Agena server is running in API-only mode because no built web frontend was found.",
    }))
}
