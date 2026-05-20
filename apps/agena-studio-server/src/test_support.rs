use std::{
    path::Path,
    sync::{Arc, LazyLock, Mutex},
};

use agena::config::LoadConfigRequest;
use agena::runtime::AgenaRuntime;
use axum_extra::extract::cookie::SameSite;
use sea_orm::Database;

pub(crate) static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

pub(crate) async fn build_test_app_state(
    workspace_root: &Path,
    settings: crate::settings::Settings,
) -> Arc<crate::AppState> {
    std::fs::create_dir_all(workspace_root).expect("test workspace should exist");

    let config_path = workspace_root.join("empty-agena-test.json");
    if !config_path.exists() {
        std::fs::write(&config_path, "{}").expect("test config should be written");
    }

    let compat_db = Arc::new(
        Database::connect("sqlite::memory:")
            .await
            .expect("compat db should open"),
    );
    let runtime = AgenaRuntime::builder()
        .with_load_request(LoadConfigRequest {
            config_path: Some(config_path),
            overrides: Vec::new(),
        })
        .with_workspace_root(workspace_root)
        .with_database_connection(compat_db.as_ref().clone())
        .build()
        .await
        .expect("runtime should build");

    let studio_db = Arc::new(
        crate::studio_db::StudioDb::open_at_path(workspace_root.join(".agena-studio-test.db"))
            .await
            .expect("studio db should open"),
    );
    let workspace_preview_registry = Arc::new(
        crate::workspace_preview_registry::WorkspacePreviewRegistry::new(studio_db.clone()),
    );
    let workspace_preview_runtime = Arc::new(
        crate::workspace_preview_runtime::WorkspacePreviewRuntime::new(
            workspace_preview_registry.clone(),
        ),
    );
    let terminal = Arc::new(crate::terminal::TerminalManager::new(studio_db.clone()).await);

    Arc::new(crate::AppState {
        ui_auth: crate::ui_auth::UiAuth::Disabled,
        ui_cookie_same_site: SameSite::Strict,
        cors_allowed_origins: Vec::new(),
        cors_allow_all: false,
        terminal,
        attachment_cache: Arc::new(crate::attachment_cache::AttachmentCacheManager::new(
            studio_db.clone(),
        )),
        workspace_preview_registry,
        workspace_preview_runtime,
        studio_db,
        settings: Arc::new(tokio::sync::RwLock::new(settings)),
        runtime,
    })
}
