use base64::Engine as _;
use std::path::PathBuf;

use agena::config::{ConfigOverride, LoadConfigRequest};
use clap::{Parser, ValueEnum};
use tracing::Level;

mod app;
mod ui_auth;
pub(crate) use app::AppState;

#[derive(Debug, Parser)]
#[command(
    name = "agena-studio",
    version,
    about = "Agena Studio (Rust + Vue) server"
)]
pub(crate) struct Args {
    /// Agena runtime config file path.
    #[arg(long, env = "AGENA_CONFIG", value_name = "PATH")]
    pub(crate) config: Option<String>,

    /// Agena runtime overrides.
    #[arg(short = 'c', long = "set")]
    pub(crate) overrides: Vec<ConfigOverride>,

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

impl Args {
    pub(crate) fn load_request(&self) -> LoadConfigRequest {
        LoadConfigRequest {
            config_path: self.config.as_ref().map(PathBuf::from),
            overrides: self.overrides.clone(),
        }
    }
}

pub(crate) fn issue_token() -> String {
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).expect("issue_token: getrandom failed");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=info".into()),
        )
        .with_target(false)
        .with_max_level(Level::INFO)
        .init();

    let args = Args::parse();
    if let Err(err) = app::run(args).await {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
