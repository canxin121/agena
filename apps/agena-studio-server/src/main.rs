use base64::Engine as _;
use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod app;
mod attachment_cache;
mod config;
mod error;
mod fs;
mod git {
    pub(crate) use agena_studio_git::*;
}
mod path_utils;
mod persistence_paths;
mod providers;
mod settings;
mod settings_events;
mod studio_db;
mod terminal;
mod terminal_ui_state;
mod ui_auth;
mod workspace_preview;
mod workspace_preview_registry;
mod workspace_preview_runtime;
pub(crate) use app::AppState;
pub(crate) use error::{ApiResult, AppError};

#[derive(Debug, Parser)]
#[command(
    name = "agena-studio",
    version,
    about = "Agena Studio (Rust + Vue) server"
)]
pub(crate) struct Args {
    /// Agena runtime overrides.
    #[arg(short = 'c', long = "set")]
    pub(crate) overrides: Vec<String>,

    /// Bind address (e.g. 127.0.0.1 or 0.0.0.0)
    #[arg(long, env = "AGENA_STUDIO_HOST", default_value = "127.0.0.1")]
    pub(crate) host: String,

    /// HTTP port
    #[arg(short, long, env = "AGENA_STUDIO_PORT", default_value_t = 3210)]
    pub(crate) port: u16,

    /// Enable UI session password
    #[arg(long, env = "AGENA_STUDIO_UI_PASSWORD")]
    pub(crate) ui_password: Option<String>,

    /// Agena workspace root for session/runtime features.
    #[arg(long, env = "AGENA_WORKSPACE_ROOT", value_name = "PATH")]
    pub(crate) workspace_root: Option<PathBuf>,

    /// Agena database URL.
    #[arg(long, env = "AGENA_DATABASE_URL", value_name = "URL")]
    pub(crate) database_url: Option<String>,

    /// Agena database path.
    #[arg(long, env = "AGENA_DATABASE_PATH", value_name = "PATH")]
    pub(crate) database_path: Option<PathBuf>,

    /// Directory with built UI assets (Vite dist).
    ///
    /// When unset, Agena Studio runs API-only (no static UI).
    #[arg(long, env = "AGENA_STUDIO_UI_DIR", value_name = "PATH")]
    pub(crate) ui_dir: Option<String>,

    /// Allowed CORS origins for cross-origin frontends.
    ///
    /// Use a comma-separated list via env (AGENA_STUDIO_CORS_ORIGINS) or repeat
    /// this flag.
    ///
    /// Example: --cors-origin http://localhost:5173
    #[arg(
        long,
        env = "AGENA_STUDIO_CORS_ORIGINS",
        value_delimiter = ',',
        value_name = "ORIGIN"
    )]
    pub(crate) cors_origin: Vec<String>,

    /// Allow all CORS origins (`*`).
    ///
    /// This is intended for explicit cross-origin API usage where credentials are
    /// not required by the browser CORS layer.
    #[arg(long, env = "AGENA_STUDIO_CORS_ALLOW_ALL", default_value_t = false)]
    pub(crate) cors_allow_all: bool,

    /// SameSite policy for the UI session cookie.
    ///
    /// - auto: Strict by default; switches to None when CORS origins are configured
    /// - none: required for cross-site cookie auth (e.g. localhost -> studio.cxits.cn)
    ///
    /// NOTE: SameSite=None requires Secure cookies, so ensure TLS (or a proxy that
    /// sets X-Forwarded-Proto=https).
    #[arg(
        long,
        env = "AGENA_STUDIO_UI_COOKIE_SAMESITE",
        value_enum,
        default_value = "auto",
        value_name = "MODE"
    )]
    pub(crate) ui_cookie_samesite: UiCookieSameSite,
}

#[derive(Clone, Debug, ValueEnum)]
#[value(rename_all = "kebab_case")]
pub(crate) enum UiCookieSameSite {
    Auto,
    Strict,
    Lax,
    None,
}

pub(crate) fn issue_token() -> String {
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).expect("issue_token: getrandom failed");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

fn main() {
    let runtime = match agena_runtime::build_app_runtime() {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };
    runtime.block_on(async_main());
}

async fn async_main() {
    let args = Args::parse();
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
        .add_directive(
            "tower_http=info"
                .parse()
                .expect("tower_http filter should parse"),
        );

    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .compact(),
        )
        .init();

    if let Err(err) = app::run(args).await {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
