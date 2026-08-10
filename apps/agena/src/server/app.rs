use std::{env, future::Future, net::SocketAddr, path::PathBuf, pin::Pin, sync::Arc};

use agena_api_server::AppState as ApiV2State;
use agena_application::Application;
use agena_runtime::bootstrap_application_services;
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

use crate::server::{
    cors::{build_cors_layer, normalize_origin_str, resolve_same_site},
    diagnostics::{agena_diagnostics, health},
    state::AppState,
};

fn fs_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/fs/home", get(crate::server::fs::fs_home))
        .route("/api/fs/list", get(crate::server::fs::fs_list))
        .route("/api/fs/search", get(crate::server::fs::fs_search))
        .route(
            "/api/fs/search-content",
            post(crate::server::fs::fs_content_search),
        )
        .route(
            "/api/fs/replace-content",
            post(crate::server::fs::fs_content_replace),
        )
        .route("/api/fs/read", get(crate::server::fs::fs_read))
        .route("/api/fs/read-chunk", get(crate::server::fs::fs_read_chunk))
        .route("/api/fs/write", post(crate::server::fs::fs_write))
        .route("/api/fs/mkdir", post(crate::server::fs::fs_mkdir))
        .route("/api/fs/delete", post(crate::server::fs::fs_delete))
        .route("/api/fs/rename", post(crate::server::fs::fs_rename))
        .route("/api/fs/raw", get(crate::server::fs::fs_raw))
        .route("/api/fs/download", get(crate::server::fs::fs_download))
}

pub(crate) async fn run(args: crate::server::ServerArgs) -> Result<()> {
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

    let workspace_root = args
        .workspace_root
        .clone()
        .unwrap_or(env::current_dir().context("failed to resolve current working directory")?);
    let runtime = bootstrap_application_services(agena_runtime::RuntimeBootstrapRequest {
        workspace_root: Some(workspace_root),
        config_override_expressions: args.overrides.clone(),
        database_url: args.database_url.clone(),
        database_path: args.database_path.clone(),
        scheduler_database_url: None,
        scheduler_database_path: None,
        initialize_schema: true,
        tracing_reload_handle: None,
    })
    .await
    .context("failed to build agena runtime")?;

    let ui_auth = crate::server::auth::init_ui_auth(args.ui_password.clone());
    let server_state_db = Arc::new(
        crate::server::persistence::db::ServerStateDb::open()
            .await
            .map_err(|error| anyhow!("failed to open agena server database: {error}"))?,
    );
    let settings_value = crate::server::settings::init_settings(server_state_db.as_ref()).await;
    let terminal = Arc::new(
        crate::server::terminal::manager::TerminalManager::new(server_state_db.clone()).await,
    );
    terminal.clone().spawn_cleanup_task();
    let attachment_cache = Arc::new(crate::server::attachment::AttachmentCacheManager::new(
        server_state_db.clone(),
    ));
    let workspace_preview_registry = Arc::new(
        crate::server::preview::registry::WorkspacePreviewRegistry::new(server_state_db.clone()),
    );
    let workspace_preview_runtime = Arc::new(
        crate::server::preview::runtime::WorkspacePreviewRuntime::new(
            workspace_preview_registry.clone(),
        ),
    );

    let application =
        Application::from_composed_runtime_services(runtime.application_services())
            .map_err(|error| anyhow!("failed to compose application services: {error}"))?;
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
        server_state_db,
        settings: Arc::new(RwLock::new(settings_value)),
        application: application.clone(),
    });
    crate::server::auth::spawn_cleanup_sessions_task_if_enabled(&shared_state.ui_auth);

    tracing::info!(
        target: "agena.runtime",
        "Agena server is serving the native /api/v1 runtime API"
    );

    let public_router = Router::new()
        .route("/health", get(health))
        .route(
            "/auth/session",
            get(crate::server::auth::auth_session_status)
                .post(crate::server::auth::auth_session_create),
        )
        .with_state(shared_state.clone());

    let agena_api = agena_api_server::router(ApiV2State::from_application(application)).layer(
        middleware::from_fn_with_state(shared_state.clone(), crate::server::auth::require_ui_auth),
    );
    let server_api_routes = fs_router()
        .route("/api/fs/upload", post(crate::server::fs::fs_upload))
        .route(
            "/api/config/settings",
            get(crate::server::config::config_settings_get)
                .put(crate::server::config::config_settings_put),
        )
        .route(
            "/api/provider/env/check",
            post(crate::server::providers::env_check_post),
        )
        .route(
            "/api/config/settings/events",
            get(crate::server::settings::events::config_settings_events),
        )
        .route(
            "/api/workspace/preview",
            get(crate::server::preview::routes::workspace_preview_get),
        )
        .route(
            "/api/workspace/preview-url",
            get(crate::server::preview::routes::workspace_preview_url_get),
        )
        .route(
            "/api/workspace/preview/proxy",
            get(crate::server::preview::routes::workspace_preview_proxy_get),
        )
        .route(
            "/api/workspace/preview/sessions",
            get(crate::server::preview::routes::workspace_preview_sessions_get)
                .post(crate::server::preview::routes::workspace_preview_sessions_post),
        )
        .route(
            "/api/workspace/preview/sessions/{id}",
            get(crate::server::preview::routes::workspace_preview_sessions_by_id_get)
                .delete(crate::server::preview::routes::workspace_preview_sessions_delete)
                .put(crate::server::preview::routes::workspace_preview_sessions_put),
        )
        .route(
            "/api/workspace/preview/sessions/{id}/rename",
            post(crate::server::preview::routes::workspace_preview_sessions_rename_post),
        )
        .route(
            "/api/workspace/preview/sessions/{id}/start",
            post(crate::server::preview::routes::workspace_preview_sessions_start_post),
        )
        .route(
            "/api/workspace/preview/sessions/{id}/stop",
            post(crate::server::preview::routes::workspace_preview_sessions_stop_post),
        )
        .route(
            "/api/workspace/preview/sessions/discover",
            post(crate::server::preview::routes::workspace_preview_sessions_discover_post),
        )
        .route(
            "/api/workspace/preview/s/{id}",
            axum::routing::any(
                crate::server::preview::routes::workspace_preview_session_proxy_root,
            ),
        )
        .route(
            "/api/workspace/preview/s/{id}/",
            axum::routing::any(
                crate::server::preview::routes::workspace_preview_session_proxy_root,
            ),
        )
        .route(
            "/api/workspace/preview/s/{id}/{*path}",
            axum::routing::any(
                crate::server::preview::routes::workspace_preview_session_proxy_path,
            ),
        )
        .route("/api/agena/diagnostics", get(agena_diagnostics))
        .route("/api/git/status", get(crate::server::git::git_status))
        .route("/api/git/watch", get(crate::server::git::git_watch))
        .route("/api/git/blame", get(crate::server::git::git_blame))
        .route("/api/git/diff", get(crate::server::git::git_diff))
        .route("/api/git/file-diff", get(crate::server::git::git_file_diff))
        .route(
            "/api/git/commit-file-diff",
            get(crate::server::git::git_commit_file_diff),
        )
        .route(
            "/api/git/commit-file-content",
            get(crate::server::git::git_commit_file_content),
        )
        .route(
            "/api/git/conflicts/file",
            get(crate::server::git::git_conflict_file),
        )
        .route(
            "/api/git/conflicts/resolve",
            post(crate::server::git::git_conflict_resolve),
        )
        .route("/api/git/patch", post(crate::server::git::git_apply_patch))
        .route("/api/git/check", get(crate::server::git::git_check))
        .route("/api/git/repos", get(crate::server::git::git_repos))
        .route(
            "/api/git/safe-directory",
            post(crate::server::git::git_safe_directory),
        )
        .route("/api/git/init", post(crate::server::git::git_init))
        .route("/api/git/clone", post(crate::server::git::git_clone))
        .route(
            "/api/git/gpg/enable-preset-passphrase",
            post(crate::server::git::git_gpg_enable_preset_passphrase),
        )
        .route(
            "/api/git/gpg/disable-signing",
            post(crate::server::git::git_gpg_disable_signing),
        )
        .route(
            "/api/git/gpg/set-signing-key",
            post(crate::server::git::git_gpg_set_signing_key),
        )
        .route(
            "/api/git/remote-info",
            get(crate::server::git::git_remote_info),
        )
        .route(
            "/api/git/remotes",
            post(crate::server::git::git_remote_add)
                .put(crate::server::git::git_remote_rename)
                .delete(crate::server::git::git_remote_remove),
        )
        .route(
            "/api/git/remotes/set-url",
            post(crate::server::git::git_remote_set_url),
        )
        .route(
            "/api/git/signing-info",
            get(crate::server::git::git_signing_info),
        )
        .route("/api/git/state", get(crate::server::git::git_state))
        .route(
            "/api/git/merge/abort",
            post(crate::server::git::git_merge_abort),
        )
        .route(
            "/api/git/rebase/abort",
            post(crate::server::git::git_rebase_abort),
        )
        .route("/api/git/stash", get(crate::server::git::git_stash_list))
        .route(
            "/api/git/stash/show",
            get(crate::server::git::git_stash_show),
        )
        .route(
            "/api/git/stash/push",
            post(crate::server::git::git_stash_push),
        )
        .route(
            "/api/git/stash/apply",
            post(crate::server::git::git_stash_apply),
        )
        .route(
            "/api/git/stash/pop",
            post(crate::server::git::git_stash_pop),
        )
        .route(
            "/api/git/stash/drop",
            post(crate::server::git::git_stash_drop),
        )
        .route(
            "/api/git/stash/drop-all",
            post(crate::server::git::git_stash_drop_all),
        )
        .route(
            "/api/git/stash/branch",
            post(crate::server::git::git_stash_branch),
        )
        .route(
            "/api/git/rebase/continue",
            post(crate::server::git::git_rebase_continue),
        )
        .route(
            "/api/git/rebase/skip",
            post(crate::server::git::git_rebase_skip),
        )
        .route(
            "/api/git/cherry-pick/abort",
            post(crate::server::git::git_cherry_pick_abort),
        )
        .route(
            "/api/git/cherry-pick/continue",
            post(crate::server::git::git_cherry_pick_continue),
        )
        .route(
            "/api/git/cherry-pick/skip",
            post(crate::server::git::git_cherry_pick_skip),
        )
        .route(
            "/api/git/cherry-pick",
            post(crate::server::git::git_cherry_pick),
        )
        .route(
            "/api/git/revert/abort",
            post(crate::server::git::git_revert_abort),
        )
        .route(
            "/api/git/revert/continue",
            post(crate::server::git::git_revert_continue),
        )
        .route(
            "/api/git/revert/skip",
            post(crate::server::git::git_revert_skip),
        )
        .route(
            "/api/git/revert-commit",
            post(crate::server::git::git_revert_commit),
        )
        .route("/api/git/merge", post(crate::server::git::git_merge))
        .route("/api/git/rebase", post(crate::server::git::git_rebase))
        .route(
            "/api/git/remote-branches",
            get(crate::server::git::git_remote_branches_list),
        )
        .route("/api/git/compare", get(crate::server::git::git_compare))
        .route("/api/git/lfs", get(crate::server::git::git_lfs_status))
        .route(
            "/api/git/lfs/install",
            post(crate::server::git::git_lfs_install),
        )
        .route(
            "/api/git/lfs/track",
            post(crate::server::git::git_lfs_track),
        )
        .route("/api/git/lfs/locks", get(crate::server::git::git_lfs_locks))
        .route("/api/git/lfs/lock", post(crate::server::git::git_lfs_lock))
        .route(
            "/api/git/lfs/unlock",
            post(crate::server::git::git_lfs_unlock),
        )
        .route(
            "/api/git/submodules",
            get(crate::server::git::git_submodules),
        )
        .route(
            "/api/git/submodules/add",
            post(crate::server::git::git_submodule_add),
        )
        .route(
            "/api/git/submodules/init",
            post(crate::server::git::git_submodule_init),
        )
        .route(
            "/api/git/submodules/update",
            post(crate::server::git::git_submodule_update),
        )
        .route("/api/git/log", get(crate::server::git::git_log))
        .route(
            "/api/git/commit-diff",
            get(crate::server::git::git_commit_diff),
        )
        .route(
            "/api/git/commit-files",
            get(crate::server::git::git_commit_files),
        )
        .route("/api/git/stage", post(crate::server::git::git_stage))
        .route("/api/git/clean", post(crate::server::git::git_clean))
        .route("/api/git/ignore", post(crate::server::git::git_ignore))
        .route("/api/git/rename", post(crate::server::git::git_rename))
        .route("/api/git/delete", post(crate::server::git::git_delete))
        .route("/api/git/unstage", post(crate::server::git::git_unstage))
        .route("/api/git/revert", post(crate::server::git::git_revert))
        .route("/api/git/pull", post(crate::server::git::git_pull))
        .route("/api/git/push", post(crate::server::git::git_push))
        .route(
            "/api/git/create-github-repo-and-push",
            post(crate::server::git::git_create_github_repo_and_push),
        )
        .route("/api/git/fetch", post(crate::server::git::git_fetch))
        .route("/api/git/commit", post(crate::server::git::git_commit))
        .route(
            "/api/git/undo-commit",
            post(crate::server::git::git_undo_commit),
        )
        .route("/api/git/reset", post(crate::server::git::git_reset_commit))
        .route(
            "/api/git/commit-template",
            get(crate::server::git::git_commit_template),
        )
        .route(
            "/api/git/conflicts",
            get(crate::server::git::git_conflicts_list),
        )
        .route(
            "/api/git/branches",
            get(crate::server::git::git_branches)
                .post(crate::server::git::git_create_branch)
                .delete(crate::server::git::git_delete_branch),
        )
        .route(
            "/api/git/branches/rename",
            post(crate::server::git::git_rename_branch),
        )
        .route(
            "/api/git/branches/delete-remote",
            post(crate::server::git::git_delete_remote_branch),
        )
        .route("/api/git/tags", get(crate::server::git::git_tags_list))
        .route("/api/git/tags", post(crate::server::git::git_tags_create))
        .route(
            "/api/git/tags",
            axum::routing::delete(crate::server::git::git_tags_delete),
        )
        .route(
            "/api/git/tags/delete-remote",
            post(crate::server::git::git_tags_delete_remote),
        )
        .route("/api/git/checkout", post(crate::server::git::git_checkout))
        .route(
            "/api/git/checkout-detached",
            post(crate::server::git::git_checkout_detached),
        )
        .route(
            "/api/git/branches/create-from",
            post(crate::server::git::git_create_branch_from),
        )
        .route(
            "/api/git/worktrees",
            get(crate::server::git::git_worktrees)
                .post(crate::server::git::git_worktree_add)
                .delete(crate::server::git::git_worktree_remove),
        )
        .route(
            "/api/git/worktrees/prune",
            post(crate::server::git::git_worktree_prune),
        )
        .route(
            "/api/git/worktrees/migrate",
            post(crate::server::git::git_worktree_migrate),
        )
        .route(
            "/api/ui/terminal/state",
            get(crate::server::terminal::ui_state::terminal_ui_state_get)
                .put(crate::server::terminal::ui_state::terminal_ui_state_put),
        )
        .route(
            "/api/ui/terminal/state/events",
            get(crate::server::terminal::ui_state::terminal_ui_state_events),
        )
        .route(
            "/api/terminal/create",
            post(crate::server::terminal::manager::terminal_create),
        )
        .route(
            "/api/terminal/{session_id}",
            get(crate::server::terminal::manager::terminal_get)
                .delete(crate::server::terminal::manager::terminal_delete),
        )
        .route(
            "/api/terminal/{session_id}/stream",
            get(crate::server::terminal::manager::terminal_stream),
        )
        .route(
            "/api/terminal/{session_id}/input",
            post(crate::server::terminal::manager::terminal_input),
        )
        .route(
            "/api/terminal/{session_id}/resize",
            post(crate::server::terminal::manager::terminal_resize),
        )
        .route(
            "/api/terminal/{session_id}/start",
            post(crate::server::terminal::manager::terminal_start),
        )
        .route(
            "/api/terminal/{session_id}/stop",
            post(crate::server::terminal::manager::terminal_stop),
        )
        .route(
            "/api/terminal/{session_id}/restart",
            post(crate::server::terminal::manager::terminal_restart),
        );

    let server_api_routes =
        server_api_routes
            .with_state(shared_state.clone())
            .layer(middleware::from_fn_with_state(
                shared_state.clone(),
                crate::server::auth::require_ui_auth,
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
        .merge(server_api_routes)
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
                "service": "agena",
                "ui": false,
                "message": "Agena server is running in API-only mode. Pass --ui-dir <dist> to serve the bundled UI.",
            }))
        })
    };

    let addr: SocketAddr = format!("{}:{}", args.host, args.port)
        .parse()
        .map_err(|error| anyhow!("invalid bind address {}:{}: {error}", args.host, args.port))?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind listener on {addr}"))?;

    tracing::info!("Agena listening on http://{addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            runtime.shutdown();
        })
        .await
        .context("server exited unexpectedly")
}
