use std::{env, net::SocketAddr, path::Path, sync::Arc};

use agena_api_server::AppState as ApiV2State;
use agena_application::Application;
use agena_runtime::bootstrap_application_services;
use anyhow::{Context, Result, anyhow};
use axum::{
    Router, middleware,
    routing::{any, get, post, put},
};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::server::{diagnostics::health, state::AppState};

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

/// Legacy web-UI routes restored for the opencode-studio frontend: filesystem,
/// git, workspace preview, and terminal. All sit behind the same bearer-token
/// `require_ui_auth` as the canonical `/api/v1` router.
fn server_api_router() -> Router<Arc<AppState>> {
    fs_router()
        .route(
            "/api/v1/server/mcp",
            get(crate::server::mcp::get_mcp_server_control)
                .put(crate::server::mcp::update_mcp_server_control),
        )
        .route(
            "/api/v1/server/mcp/oauth/password",
            put(crate::server::mcp::set_mcp_oauth_password)
                .delete(crate::server::mcp::clear_mcp_oauth_password),
        )
        .route("/api/fs/upload", post(crate::server::fs::fs_upload))
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
        )
}

/// Install the server's stderr tracing subscriber before HTTP bootstrap.
///
/// The TUI and one-shot command entry points install their own subscribers,
/// but the long-running `agena server` entry point historically did not. That
/// made tunnel failures opaque: Secure MCP Tunnel can report only an upstream
/// status, while the server had no visible request breadcrumb to correlate it
/// with. Keep this process-level setup here so `2>&1 | tee ...` captures MCP
/// diagnostics as soon as the listener starts.
fn init_server_tracing(args: &crate::server::ServerArgs, workspace_root: &Path) {
    let tracing = agena_runtime::resolve_runtime_bootstrap_preflight(
        &agena_runtime::RuntimeBootstrapRequest {
            workspace_root: Some(workspace_root.to_owned()),
            config_override_expressions: args.overrides.clone(),
            ..Default::default()
        },
    )
    .map(|preflight| preflight.tracing)
    .unwrap_or_default();
    let filter = agena_runtime::runtime_env_filter(&tracing).unwrap_or_else(|_| {
        agena_runtime::runtime_env_filter(&agena_runtime::RuntimeTracingConfiguration::default())
            .expect("default tracing filter should parse")
    });
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_ansi(false)
                .compact()
                .with_writer(std::io::stderr),
        )
        .try_init();
}

pub(crate) async fn run(args: crate::server::ServerArgs) -> Result<()> {
    let workspace_root = args
        .workspace_root
        .clone()
        .unwrap_or(env::current_dir().context("failed to resolve current working directory")?);
    init_server_tracing(&args, workspace_root.as_path());
    let ui_dir = crate::server::web_ui::resolve_ui_dir(args.ui_dir.as_deref(), &workspace_root)?;
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
    let terminal = Arc::new(
        crate::server::terminal::manager::TerminalManager::new(server_state_db.clone()).await,
    );
    terminal.clone().spawn_cleanup_task();
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
    let mcp_workspace = application
        .service()
        .resolve_workspace(agena_application::dto::WorkspaceResolveRequest {
            workspace: agena_application::dto::WorkspacePathRequest {
                path: application.workspace_root().to_string_lossy().into_owned(),
            },
            create_if_missing: true,
        })
        .await
        .map_err(|error| anyhow!("failed to resolve the MCP workspace: {error}"))?;
    let addr: SocketAddr = format!("{}:{}", args.host, args.port)
        .parse()
        .map_err(|error| anyhow!("invalid bind address {}:{}: {error}", args.host, args.port))?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind listener on {addr}"))?;
    let bound_addr = listener
        .local_addr()
        .context("failed to inspect the server listener")?;
    let advertised_ip = if bound_addr.ip().is_unspecified() {
        if bound_addr.is_ipv6() {
            std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)
        } else {
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
        }
    } else {
        bound_addr.ip()
    };
    let endpoint_url = format!(
        "http://{}",
        SocketAddr::new(advertised_ip, bound_addr.port())
    );
    let mcp_state = Arc::new(
        crate::server::mcp::McpServerState::load(
            application.clone(),
            mcp_workspace.id,
            ui_auth.clone(),
            server_state_db.clone(),
            crate::server::mcp::McpServerStartupConfig {
                public_url: args.mcp_public_url.as_deref(),
                oauth_issuer_url: args.mcp_oauth_issuer_url.as_deref(),
                auth_mode: args.mcp_auth_mode.map(Into::into),
                anonymous_access: args.mcp_anonymous_access.map(Into::into),
                tool_exposure: args.mcp_tool_exposure.map(Into::into),
                client_registration: args.mcp_client_registration.map(Into::into),
                fallback_public_url: endpoint_url.as_str(),
            },
        )
        .await
        .map_err(|error| anyhow!("failed to configure the Agena MCP server: {error}"))?,
    );
    mcp_state.log_startup_status();

    let shared_state = Arc::new(AppState {
        ui_auth: ui_auth.clone(),
        mcp_server: mcp_state.clone(),
        terminal,
        workspace_preview_registry,
        workspace_preview_runtime,
        server_state_db,
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

    let api_state = ApiV2State::from_application(application.clone());
    let server_identity = api_state.server().clone();
    let agena_api = agena_api_server::router(api_state).layer(middleware::from_fn_with_state(
        shared_state.clone(),
        crate::server::auth::require_ui_auth,
    ));

    let server_api_routes =
        server_api_router()
            .with_state(shared_state.clone())
            .layer(middleware::from_fn_with_state(
                shared_state.clone(),
                crate::server::auth::require_ui_auth,
            ));

    let app = public_router
        .merge(agena_api)
        .merge(server_api_routes)
        .route("/api", any(crate::server::web_ui::api_not_found))
        .route("/api/{*path}", any(crate::server::web_ui::api_not_found))
        .route("/auth", any(crate::server::web_ui::api_not_found))
        .route("/auth/{*path}", any(crate::server::web_ui::api_not_found))
        .merge(crate::server::mcp::router(mcp_state));
    let app = app.layer(TraceLayer::new_for_http());
    let app = crate::server::web_ui::attach(app, ui_dir);
    let server_record =
        crate::server::server_record::publish_record(endpoint_url.clone(), &server_identity)?;

    tracing::info!(server_id = %server_identity.id, "Agena listening on {endpoint_url}");
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            runtime.shutdown();
        })
        .await
        .context("server exited unexpectedly");
    drop(server_record);
    result
}
