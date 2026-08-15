use std::{env, net::SocketAddr, sync::Arc};

use agena_api_server::AppState as ApiV2State;
use agena_application::Application;
use agena_runtime::bootstrap_application_services;
use anyhow::{Context, Result, anyhow};
use axum::{
    Json, Router,
    middleware,
    routing::{get, post},
};
use serde_json::json;
use tower_http::trace::TraceLayer;

use crate::server::{diagnostics::health, state::AppState};

pub(crate) async fn run(args: crate::server::ServerArgs) -> Result<()> {
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

    let application =
        Application::from_composed_runtime_services(runtime.application_services())
            .map_err(|error| anyhow!("failed to compose application services: {error}"))?;
    let shared_state = Arc::new(AppState {
        ui_auth: ui_auth.clone(),
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

    let api_state = ApiV2State::from_application(application);
    let center_identity = api_state.center().clone();
    let agena_api = agena_api_server::router(api_state).layer(middleware::from_fn_with_state(
        shared_state.clone(),
        crate::server::auth::require_ui_auth,
    ));

    let app = public_router
        .merge(agena_api)
        .layer(TraceLayer::new_for_http())
        .fallback(|| async {
            Json(json!({
                "service": "agena",
                "message": "Agena server is running. The TUI and CLI clients connect over the HTTP API.",
            }))
        });

    let addr: SocketAddr = format!("{}:{}", args.host, args.port)
        .parse()
        .map_err(|error| anyhow!("invalid bind address {}:{}: {error}", args.host, args.port))?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind listener on {addr}"))?;
    let bound_addr = listener
        .local_addr()
        .context("failed to inspect the processing-center listener")?;
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
    let center_record =
        crate::server::center_record::publish_record(endpoint_url.clone(), &center_identity)?;

    tracing::info!(center_id = %center_identity.id, "Agena listening on {endpoint_url}");
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            runtime.shutdown();
        })
        .await
        .context("server exited unexpectedly");
    drop(center_record);
    result
}
