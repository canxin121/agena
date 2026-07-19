use std::{env, net::SocketAddr, path::PathBuf, sync::Arc};

use agena::config::ConfigLoader;
use agena::runtime::AgenaRuntime;
use agena::storage::StorageConfig;
use agena::tracing as tracing_config;
use agena_api_server::AppState as ApiV2State;
use anyhow::{Context, Result, anyhow};
use axum::{
    Json, Router,
    extract::Query,
    http::{
        HeaderValue, Method,
        header::{self, HeaderName},
    },
    middleware,
    routing::{get, post},
};
use axum_extra::extract::cookie::SameSite;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::RwLock;
use tower_http::{
    cors::{AllowOrigin, Any, CorsLayer},
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use url::Url;

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) ui_auth: crate::ui_auth::UiAuth,
    pub(crate) ui_cookie_same_site: SameSite,
    pub(crate) cors_allowed_origins: Vec<String>,
    pub(crate) cors_allow_all: bool,
    pub(crate) terminal: Arc<crate::terminal::TerminalManager>,
    pub(crate) attachment_cache: Arc<crate::attachment_cache::AttachmentCacheManager>,
    pub(crate) workspace_preview_registry:
        Arc<crate::workspace_preview_registry::WorkspacePreviewRegistry>,
    pub(crate) workspace_preview_runtime:
        Arc<crate::workspace_preview_runtime::WorkspacePreviewRuntime>,
    pub(crate) studio_db: Arc<crate::studio_db::StudioDb>,
    pub(crate) settings: Arc<RwLock<crate::settings::Settings>>,
    pub(crate) runtime: AgenaRuntime,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StudioHealthResponse {
    status: &'static str,
    generation: u64,
    loaded_at: String,
    workspace_root: String,
    config_path: String,
    config_found: bool,
    provider_ids: Vec<String>,
    session_runtime_available: bool,
}

async fn health(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> Json<StudioHealthResponse> {
    let snapshot = state.runtime.current_snapshot();
    let resolution = snapshot.config_resolution();

    Json(StudioHealthResponse {
        status: "ok",
        generation: snapshot.generation(),
        loaded_at: snapshot.loaded_at().to_rfc3339(),
        workspace_root: state.runtime.workspace_root().display().to_string(),
        config_path: resolution.meta.config_path.display().to_string(),
        config_found: resolution.meta.config_found,
        provider_ids: resolution.config.providers.keys().cloned().collect(),
        session_runtime_available: state.runtime.session_manager().is_some(),
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticsQuery {
    directory: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticPathRecord {
    path: String,
    exists: bool,
}

fn diag_entry(path: PathBuf) -> DiagnosticPathRecord {
    let text = path.to_string_lossy().into_owned();
    let exists = std::fs::metadata(&path).is_ok();
    DiagnosticPathRecord { path: text, exists }
}

async fn agena_studio_diagnostics(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Query(query): Query<DiagnosticsQuery>,
) -> Json<Value> {
    let snapshot = state.runtime.current_snapshot();
    let resolution = snapshot.config_resolution();
    let normalized_directory = query
        .directory
        .as_deref()
        .map(crate::path_utils::normalize_directory_path)
        .and_then(|text| {
            let trimmed = text.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(PathBuf::from(trimmed))
            }
        });

    Json(json!({
        "timestamp": time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default(),
        "runtime": {
            "generation": snapshot.generation(),
            "loadedAt": snapshot.loaded_at().to_rfc3339(),
            "workspaceRoot": state.runtime.workspace_root().display().to_string(),
            "configPath": resolution.meta.config_path.display().to_string(),
            "configFound": resolution.meta.config_found,
            "providerIds": resolution.config.providers.keys().cloned().collect::<Vec<_>>(),
            "sessionRuntimeAvailable": state.runtime.session_manager().is_some(),
        },
        "studio": {
            "apiSurface": "agena-native",
        },
        "paths": {
            "input": {
                "directory": query.directory,
                "normalizedDirectory": normalized_directory.as_ref().map(|path| path.to_string_lossy().into_owned())
            },
            "studio": {
                "dataDirCandidates": crate::persistence_paths::studio_data_dir_candidates().into_iter().map(diag_entry).collect::<Vec<_>>(),
                "dbPath": diag_entry(crate::persistence_paths::studio_db_path()),
                "dbCandidates": crate::persistence_paths::studio_db_path_candidates().into_iter().map(diag_entry).collect::<Vec<_>>(),
                "settingsPath": diag_entry(crate::persistence_paths::studio_settings_path()),
                "settingsCandidates": crate::persistence_paths::studio_settings_path_candidates().into_iter().map(diag_entry).collect::<Vec<_>>(),
                "terminalUiStatePath": diag_entry(crate::persistence_paths::terminal_ui_state_path()),
                "terminalUiStateCandidates": crate::persistence_paths::terminal_ui_state_path_candidates().into_iter().map(diag_entry).collect::<Vec<_>>(),
                "terminalRegistryPath": diag_entry(crate::persistence_paths::terminal_session_registry_path()),
                "terminalRegistryCandidates": crate::persistence_paths::terminal_session_registry_path_candidates().into_iter().map(diag_entry).collect::<Vec<_>>()
            }
        },
        "environment": {
            "HOME": std::env::var("HOME").ok(),
            "USERPROFILE": std::env::var("USERPROFILE").ok(),
            "APPDATA": std::env::var("APPDATA").ok(),
            "LOCALAPPDATA": std::env::var("LOCALAPPDATA").ok(),
            "AGENA_STUDIO_DATA_DIR": std::env::var("AGENA_STUDIO_DATA_DIR").ok(),
            "AGENA_STUDIO_HOST": std::env::var("AGENA_STUDIO_HOST").ok(),
            "AGENA_STUDIO_PORT": std::env::var("AGENA_STUDIO_PORT").ok(),
        }
    }))
}

fn fs_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/fs/home", get(crate::fs::fs_home))
        .route("/api/fs/list", get(crate::fs::fs_list))
        .route("/api/fs/search", get(crate::fs::fs_search))
        .route("/api/fs/search-content", post(crate::fs::fs_content_search))
        .route(
            "/api/fs/replace-content",
            post(crate::fs::fs_content_replace),
        )
        .route("/api/fs/read", get(crate::fs::fs_read))
        .route("/api/fs/read-chunk", get(crate::fs::fs_read_chunk))
        .route("/api/fs/write", post(crate::fs::fs_write))
        .route("/api/fs/mkdir", post(crate::fs::fs_mkdir))
        .route("/api/fs/delete", post(crate::fs::fs_delete))
        .route("/api/fs/rename", post(crate::fs::fs_rename))
        .route("/api/fs/raw", get(crate::fs::fs_raw))
        .route("/api/fs/download", get(crate::fs::fs_download))
}

fn normalize_origin_str(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let Ok(url) = Url::parse(trimmed) else {
        return None;
    };
    let scheme = url.scheme();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    Some(url.origin().ascii_serialization())
}

fn build_cors_layer(origins: &[String], allow_all: bool) -> Option<CorsLayer> {
    let allow_headers = [
        header::ACCEPT,
        header::CONTENT_TYPE,
        header::AUTHORIZATION,
        header::IF_MATCH,
        header::IF_NONE_MATCH,
        HeaderName::from_static("last-event-id"),
    ];
    let allow_methods = [
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::DELETE,
        Method::PATCH,
        Method::OPTIONS,
    ];

    if allow_all {
        return Some(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_credentials(false)
                .allow_headers(allow_headers)
                .allow_methods(allow_methods)
                .max_age(std::time::Duration::from_secs(60 * 60)),
        );
    }

    if origins.is_empty() {
        return None;
    }

    let mut values: Vec<HeaderValue> = Vec::new();
    for origin in origins {
        let Ok(value) = HeaderValue::from_str(origin) else {
            tracing::warn!(origin = %origin, "ignoring invalid CORS origin");
            continue;
        };
        values.push(value);
    }

    if values.is_empty() {
        return None;
    }

    Some(
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(values))
            .allow_credentials(true)
            .allow_headers(allow_headers)
            .allow_methods(allow_methods)
            .max_age(std::time::Duration::from_secs(60 * 60)),
    )
}

fn resolve_same_site(mode: crate::UiCookieSameSite, has_cross_origin: bool) -> SameSite {
    match mode {
        crate::UiCookieSameSite::Strict => SameSite::Strict,
        crate::UiCookieSameSite::Lax => SameSite::Lax,
        crate::UiCookieSameSite::None => SameSite::None,
        crate::UiCookieSameSite::Auto => {
            if has_cross_origin {
                SameSite::None
            } else {
                SameSite::Strict
            }
        }
    }
}

pub(crate) async fn run(args: crate::Args) -> Result<()> {
    let mut normalized_cors_origins = Vec::<String>::new();
    for raw in &args.cors_origin {
        let Some(origin) = normalize_origin_str(raw) else {
            tracing::warn!(origin = %raw, "ignoring invalid CORS origin");
            continue;
        };
        normalized_cors_origins.push(origin);
    }
    normalized_cors_origins.sort();
    normalized_cors_origins.dedup();

    let database_url = StorageConfig {
        database_url: args.database_url.clone(),
        database_path: args.database_path.clone(),
    }
    .resolve_url()
    .map_err(|e| anyhow!("{e}"))?;
    StorageConfig::ensure_parent(database_url.as_str()).map_err(|e| anyhow!("{e}"))?;

    let workspace_root = args
        .workspace_root
        .clone()
        .unwrap_or(env::current_dir().context("failed to resolve current working directory")?);
    let tracing = ConfigLoader::default()
        .load(&args.load_request())
        .map(|resolution| resolution.config.tracing)
        .unwrap_or_default();
    let db = Arc::new(
        tracing_config::connect_database(database_url.as_str(), &tracing)
            .await
            .with_context(|| format!("failed to connect to database {database_url}"))?,
    );

    let runtime = AgenaRuntime::new(agena::runtime::AgenaRuntimeConfig {
        load_request: args.load_request(),
        workspace_root: Some(workspace_root),
        database_connection: Some(Arc::clone(&db)),
        database_url: None,
        initialize_schema: true,
        tracing_reload_handle: None,
    })
    .await
    .context("failed to build agena runtime")?;

    let ui_auth = crate::ui_auth::init_ui_auth(args.ui_password.clone());
    let studio_db = Arc::new(
        crate::studio_db::StudioDb::open()
            .await
            .map_err(|error| anyhow!("failed to open agena studio database: {error}"))?,
    );
    let settings_value = crate::settings::init_settings(studio_db.as_ref()).await;
    let terminal = Arc::new(crate::terminal::TerminalManager::new(studio_db.clone()).await);
    terminal.clone().spawn_cleanup_task();
    let attachment_cache = Arc::new(crate::attachment_cache::AttachmentCacheManager::new(
        studio_db.clone(),
    ));
    let workspace_preview_registry = Arc::new(
        crate::workspace_preview_registry::WorkspacePreviewRegistry::new(studio_db.clone()),
    );
    let workspace_preview_runtime = Arc::new(
        crate::workspace_preview_runtime::WorkspacePreviewRuntime::new(
            workspace_preview_registry.clone(),
        ),
    );

    let shared_state = Arc::new(AppState {
        ui_auth: ui_auth.clone(),
        ui_cookie_same_site: resolve_same_site(
            args.ui_cookie_samesite.clone(),
            args.cors_allow_all || !normalized_cors_origins.is_empty(),
        ),
        cors_allowed_origins: normalized_cors_origins.clone(),
        cors_allow_all: args.cors_allow_all,
        terminal,
        attachment_cache,
        workspace_preview_registry,
        workspace_preview_runtime,
        studio_db,
        settings: Arc::new(RwLock::new(settings_value)),
        runtime: runtime.clone(),
    });
    crate::ui_auth::spawn_cleanup_sessions_task_if_enabled(&shared_state.ui_auth);

    tracing::info!(
        target: "agena_studio.runtime",
        "Agena Studio is serving the native /api/v1 runtime API"
    );

    let public_router = Router::new()
        .route("/health", get(health))
        .route(
            "/auth/session",
            get(crate::ui_auth::auth_session_status).post(crate::ui_auth::auth_session_create),
        )
        .with_state(shared_state.clone());

    let agena_api = agena_api_server::router(ApiV2State::new(runtime.clone(), db.clone())).layer(
        middleware::from_fn_with_state(shared_state.clone(), crate::ui_auth::require_ui_auth),
    );
    let studio_api_routes = fs_router()
        .route("/api/fs/upload", post(crate::fs::fs_upload))
        .route(
            "/api/config/settings",
            get(crate::config::config_settings_get).put(crate::config::config_settings_put),
        )
        .route(
            "/api/provider/env/check",
            post(crate::providers::env_check_post),
        )
        .route(
            "/api/config/settings/events",
            get(crate::settings_events::config_settings_events),
        )
        .route(
            "/api/workspace/preview",
            get(crate::workspace_preview::workspace_preview_get),
        )
        .route(
            "/api/workspace/preview-url",
            get(crate::workspace_preview::workspace_preview_url_get),
        )
        .route(
            "/api/workspace/preview/proxy",
            get(crate::workspace_preview::workspace_preview_proxy_get),
        )
        .route(
            "/api/workspace/preview/sessions",
            get(crate::workspace_preview::workspace_preview_sessions_get)
                .post(crate::workspace_preview::workspace_preview_sessions_post),
        )
        .route(
            "/api/workspace/preview/sessions/{id}",
            get(crate::workspace_preview::workspace_preview_sessions_by_id_get)
                .delete(crate::workspace_preview::workspace_preview_sessions_delete)
                .put(crate::workspace_preview::workspace_preview_sessions_put),
        )
        .route(
            "/api/workspace/preview/sessions/{id}/rename",
            post(crate::workspace_preview::workspace_preview_sessions_rename_post),
        )
        .route(
            "/api/workspace/preview/sessions/{id}/start",
            post(crate::workspace_preview::workspace_preview_sessions_start_post),
        )
        .route(
            "/api/workspace/preview/sessions/{id}/stop",
            post(crate::workspace_preview::workspace_preview_sessions_stop_post),
        )
        .route(
            "/api/workspace/preview/sessions/discover",
            post(crate::workspace_preview::workspace_preview_sessions_discover_post),
        )
        .route(
            "/api/workspace/preview/s/{id}",
            axum::routing::any(crate::workspace_preview::workspace_preview_session_proxy_root),
        )
        .route(
            "/api/workspace/preview/s/{id}/",
            axum::routing::any(crate::workspace_preview::workspace_preview_session_proxy_root),
        )
        .route(
            "/api/workspace/preview/s/{id}/{*path}",
            axum::routing::any(crate::workspace_preview::workspace_preview_session_proxy_path),
        )
        .route(
            "/api/agena-studio/diagnostics",
            get(agena_studio_diagnostics),
        )
        .route("/api/git/status", get(crate::git::git_status))
        .route("/api/git/watch", get(crate::git::git_watch))
        .route("/api/git/blame", get(crate::git::git_blame))
        .route("/api/git/diff", get(crate::git::git_diff))
        .route("/api/git/file-diff", get(crate::git::git_file_diff))
        .route(
            "/api/git/commit-file-diff",
            get(crate::git::git_commit_file_diff),
        )
        .route(
            "/api/git/commit-file-content",
            get(crate::git::git_commit_file_content),
        )
        .route(
            "/api/git/conflicts/file",
            get(crate::git::git_conflict_file),
        )
        .route(
            "/api/git/conflicts/resolve",
            post(crate::git::git_conflict_resolve),
        )
        .route("/api/git/patch", post(crate::git::git_apply_patch))
        .route("/api/git/check", get(crate::git::git_check))
        .route("/api/git/repos", get(crate::git::git_repos))
        .route(
            "/api/git/safe-directory",
            post(crate::git::git_safe_directory),
        )
        .route("/api/git/init", post(crate::git::git_init))
        .route("/api/git/clone", post(crate::git::git_clone))
        .route(
            "/api/git/gpg/enable-preset-passphrase",
            post(crate::git::git_gpg_enable_preset_passphrase),
        )
        .route(
            "/api/git/gpg/disable-signing",
            post(crate::git::git_gpg_disable_signing),
        )
        .route(
            "/api/git/gpg/set-signing-key",
            post(crate::git::git_gpg_set_signing_key),
        )
        .route("/api/git/remote-info", get(crate::git::git_remote_info))
        .route(
            "/api/git/remotes",
            post(crate::git::git_remote_add)
                .put(crate::git::git_remote_rename)
                .delete(crate::git::git_remote_remove),
        )
        .route(
            "/api/git/remotes/set-url",
            post(crate::git::git_remote_set_url),
        )
        .route("/api/git/signing-info", get(crate::git::git_signing_info))
        .route("/api/git/state", get(crate::git::git_state))
        .route("/api/git/merge/abort", post(crate::git::git_merge_abort))
        .route("/api/git/rebase/abort", post(crate::git::git_rebase_abort))
        .route("/api/git/stash", get(crate::git::git_stash_list))
        .route("/api/git/stash/show", get(crate::git::git_stash_show))
        .route("/api/git/stash/push", post(crate::git::git_stash_push))
        .route("/api/git/stash/apply", post(crate::git::git_stash_apply))
        .route("/api/git/stash/pop", post(crate::git::git_stash_pop))
        .route("/api/git/stash/drop", post(crate::git::git_stash_drop))
        .route(
            "/api/git/stash/drop-all",
            post(crate::git::git_stash_drop_all),
        )
        .route("/api/git/stash/branch", post(crate::git::git_stash_branch))
        .route(
            "/api/git/rebase/continue",
            post(crate::git::git_rebase_continue),
        )
        .route("/api/git/rebase/skip", post(crate::git::git_rebase_skip))
        .route(
            "/api/git/cherry-pick/abort",
            post(crate::git::git_cherry_pick_abort),
        )
        .route(
            "/api/git/cherry-pick/continue",
            post(crate::git::git_cherry_pick_continue),
        )
        .route(
            "/api/git/cherry-pick/skip",
            post(crate::git::git_cherry_pick_skip),
        )
        .route("/api/git/cherry-pick", post(crate::git::git_cherry_pick))
        .route("/api/git/revert/abort", post(crate::git::git_revert_abort))
        .route(
            "/api/git/revert/continue",
            post(crate::git::git_revert_continue),
        )
        .route("/api/git/revert/skip", post(crate::git::git_revert_skip))
        .route(
            "/api/git/revert-commit",
            post(crate::git::git_revert_commit),
        )
        .route("/api/git/merge", post(crate::git::git_merge))
        .route("/api/git/rebase", post(crate::git::git_rebase))
        .route(
            "/api/git/remote-branches",
            get(crate::git::git_remote_branches_list),
        )
        .route("/api/git/compare", get(crate::git::git_compare))
        .route("/api/git/lfs", get(crate::git::git_lfs_status))
        .route("/api/git/lfs/install", post(crate::git::git_lfs_install))
        .route("/api/git/lfs/track", post(crate::git::git_lfs_track))
        .route("/api/git/lfs/locks", get(crate::git::git_lfs_locks))
        .route("/api/git/lfs/lock", post(crate::git::git_lfs_lock))
        .route("/api/git/lfs/unlock", post(crate::git::git_lfs_unlock))
        .route("/api/git/submodules", get(crate::git::git_submodules))
        .route(
            "/api/git/submodules/add",
            post(crate::git::git_submodule_add),
        )
        .route(
            "/api/git/submodules/init",
            post(crate::git::git_submodule_init),
        )
        .route(
            "/api/git/submodules/update",
            post(crate::git::git_submodule_update),
        )
        .route("/api/git/log", get(crate::git::git_log))
        .route("/api/git/commit-diff", get(crate::git::git_commit_diff))
        .route("/api/git/commit-files", get(crate::git::git_commit_files))
        .route("/api/git/stage", post(crate::git::git_stage))
        .route("/api/git/clean", post(crate::git::git_clean))
        .route("/api/git/ignore", post(crate::git::git_ignore))
        .route("/api/git/rename", post(crate::git::git_rename))
        .route("/api/git/delete", post(crate::git::git_delete))
        .route("/api/git/unstage", post(crate::git::git_unstage))
        .route("/api/git/revert", post(crate::git::git_revert))
        .route("/api/git/pull", post(crate::git::git_pull))
        .route("/api/git/push", post(crate::git::git_push))
        .route(
            "/api/git/create-github-repo-and-push",
            post(crate::git::git_create_github_repo_and_push),
        )
        .route("/api/git/fetch", post(crate::git::git_fetch))
        .route("/api/git/commit", post(crate::git::git_commit))
        .route("/api/git/undo-commit", post(crate::git::git_undo_commit))
        .route("/api/git/reset", post(crate::git::git_reset_commit))
        .route(
            "/api/git/commit-template",
            get(crate::git::git_commit_template),
        )
        .route("/api/git/conflicts", get(crate::git::git_conflicts_list))
        .route(
            "/api/git/branches",
            get(crate::git::git_branches)
                .post(crate::git::git_create_branch)
                .delete(crate::git::git_delete_branch),
        )
        .route(
            "/api/git/branches/rename",
            post(crate::git::git_rename_branch),
        )
        .route(
            "/api/git/branches/delete-remote",
            post(crate::git::git_delete_remote_branch),
        )
        .route("/api/git/tags", get(crate::git::git_tags_list))
        .route("/api/git/tags", post(crate::git::git_tags_create))
        .route(
            "/api/git/tags",
            axum::routing::delete(crate::git::git_tags_delete),
        )
        .route(
            "/api/git/tags/delete-remote",
            post(crate::git::git_tags_delete_remote),
        )
        .route("/api/git/checkout", post(crate::git::git_checkout))
        .route(
            "/api/git/checkout-detached",
            post(crate::git::git_checkout_detached),
        )
        .route(
            "/api/git/branches/create-from",
            post(crate::git::git_create_branch_from),
        )
        .route(
            "/api/git/worktrees",
            get(crate::git::git_worktrees)
                .post(crate::git::git_worktree_add)
                .delete(crate::git::git_worktree_remove),
        )
        .route(
            "/api/git/worktrees/prune",
            post(crate::git::git_worktree_prune),
        )
        .route(
            "/api/git/worktrees/migrate",
            post(crate::git::git_worktree_migrate),
        )
        .route(
            "/api/ui/terminal/state",
            get(crate::terminal_ui_state::terminal_ui_state_get)
                .put(crate::terminal_ui_state::terminal_ui_state_put),
        )
        .route(
            "/api/ui/terminal/state/events",
            get(crate::terminal_ui_state::terminal_ui_state_events),
        )
        .route(
            "/api/terminal/create",
            post(crate::terminal::terminal_create),
        )
        .route(
            "/api/terminal/{session_id}",
            get(crate::terminal::terminal_get).delete(crate::terminal::terminal_delete),
        )
        .route(
            "/api/terminal/{session_id}/stream",
            get(crate::terminal::terminal_stream),
        )
        .route(
            "/api/terminal/{session_id}/input",
            post(crate::terminal::terminal_input),
        )
        .route(
            "/api/terminal/{session_id}/resize",
            post(crate::terminal::terminal_resize),
        )
        .route(
            "/api/terminal/{session_id}/start",
            post(crate::terminal::terminal_start),
        )
        .route(
            "/api/terminal/{session_id}/stop",
            post(crate::terminal::terminal_stop),
        )
        .route(
            "/api/terminal/{session_id}/restart",
            post(crate::terminal::terminal_restart),
        );

    let studio_api_routes =
        studio_api_routes
            .with_state(shared_state.clone())
            .layer(middleware::from_fn_with_state(
                shared_state.clone(),
                crate::ui_auth::require_ui_auth,
            ));

    let ui_dir_path = args.ui_dir.as_ref().map(PathBuf::from);
    let (has_ui, asset_files, static_files) = match &ui_dir_path {
        None => {
            tracing::info!("UI disabled (API-only mode)");
            (false, None, None)
        }
        Some(dir) => {
            let index_file = dir.join("index.html");
            let has_ui = index_file.is_file();
            tracing::info!(
                "UI dir resolved to {} (index.html exists: {})",
                dir.display(),
                has_ui
            );

            let asset_files = ServeDir::new(dir.join("assets"));
            let static_files = ServeDir::new(dir).fallback(ServeFile::new(index_file));
            (has_ui, Some(asset_files), Some(static_files))
        }
    };

    let mut app = public_router
        .merge(agena_api)
        .merge(studio_api_routes)
        .layer(TraceLayer::new_for_http());

    if let Some(cors) = build_cors_layer(&normalized_cors_origins, args.cors_allow_all) {
        if args.cors_allow_all {
            tracing::info!("CORS enabled (allow all)");
        } else {
            tracing::info!(origins = %normalized_cors_origins.len(), "CORS enabled");
        }
        app = app.layer(cors);
    }

    app = if has_ui {
        app.nest_service("/assets", asset_files.expect("assets service"))
            .fallback_service(static_files.expect("static service"))
    } else {
        app.fallback(|| async {
            Json(serde_json::json!({
                "service": "agena-studio",
                "ui": false,
                "message": "Agena Studio server is running in API-only mode. Pass --ui-dir <dist> to serve the bundled UI.",
            }))
        })
    };

    let addr: SocketAddr = format!("{}:{}", args.host, args.port)
        .parse()
        .map_err(|error| anyhow!("invalid bind address {}:{}: {error}", args.host, args.port))?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind listener on {addr}"))?;

    tracing::info!("Agena Studio listening on http://{addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            runtime.shutdown();
        })
        .await
        .context("server exited unexpectedly")
}
