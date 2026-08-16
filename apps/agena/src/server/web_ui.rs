use std::{
    collections::HashSet,
    env,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use axum::{
    Json, Router,
    http::{StatusCode, Uri},
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

    Ok(candidates
        .into_iter()
        .find(|directory| directory.join(INDEX_FILE).is_file())
        .and_then(|directory| std::fs::canonicalize(directory).ok()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, header},
        routing::any,
    };
    use tower::ServiceExt;

    fn fixture() -> tempfile::TempDir {
        let directory = tempfile::tempdir().expect("temporary UI directory");
        std::fs::create_dir_all(directory.path().join("assets")).expect("create assets directory");
        std::fs::write(
            directory.path().join(INDEX_FILE),
            "<!doctype html><title>Agena UI</title>",
        )
        .expect("write index");
        std::fs::write(
            directory.path().join("assets/app.js"),
            "window.AGENA_UI = true;",
        )
        .expect("write asset");
        directory
    }

    async fn request(app: Router, path: &str) -> axum::response::Response {
        app.oneshot(
            Request::builder()
                .uri(path)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response")
    }

    async fn body(response: axum::response::Response) -> String {
        String::from_utf8(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("response body")
                .to_vec(),
        )
        .expect("UTF-8 response")
    }

    #[tokio::test]
    async fn serves_assets_and_history_routes_from_one_router() {
        let fixture = fixture();
        let app = attach(Router::new(), Some(fixture.path().to_path_buf()));

        let root = request(app.clone(), "/").await;
        assert_eq!(root.status(), StatusCode::OK);
        assert_eq!(
            root.headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/html")
        );
        assert!(body(root).await.contains("Agena UI"));

        let history_route = request(app.clone(), "/settings/providers").await;
        assert_eq!(history_route.status(), StatusCode::OK);
        assert!(body(history_route).await.contains("Agena UI"));

        let asset = request(app, "/assets/app.js").await;
        assert_eq!(asset.status(), StatusCode::OK);
        assert!(body(asset).await.contains("AGENA_UI"));
    }

    #[tokio::test]
    async fn server_namespaces_keep_json_not_found_responses() {
        let fixture = fixture();
        let app = attach(
            Router::new()
                .route("/api/{*path}", any(api_not_found))
                .route("/auth/{*path}", any(api_not_found)),
            Some(fixture.path().to_path_buf()),
        );

        for path in ["/api/v1/missing", "/auth/missing"] {
            let response = request(app.clone(), path).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            assert!(body(response).await.contains("route_not_found"));
        }
    }

    #[test]
    fn explicit_directory_is_strict_and_workspace_build_is_auto_detected() {
        let fixture = fixture();
        let resolved = resolve_ui_dir(Some(fixture.path()), Path::new("/unused"))
            .expect("resolve explicit UI directory")
            .expect("explicit UI directory");
        assert_eq!(
            resolved,
            fixture.path().canonicalize().expect("canonical UI path")
        );

        let invalid = tempfile::tempdir().expect("invalid UI directory");
        assert!(resolve_ui_dir(Some(invalid.path()), Path::new("/unused")).is_err());

        let workspace = tempfile::tempdir().expect("workspace");
        let dist = workspace.path().join("packages/agena-web/dist");
        std::fs::create_dir_all(&dist).expect("create workspace dist");
        std::fs::write(dist.join(INDEX_FILE), "workspace UI").expect("write workspace index");
        assert_eq!(
            resolve_ui_dir(None, workspace.path()).expect("auto-detect UI directory"),
            Some(dist.canonicalize().expect("canonical workspace dist"))
        );
    }
}
