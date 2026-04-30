mod app;
mod backend;
mod clipboard;
mod commands;
mod composer_queue;
mod external_editor;
mod external_pager;
mod i18n;
mod keybindings;
mod terminal;
mod tui_config;
mod ui_text;

use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{self, Stderr, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
};

use agena::{
    AppError,
    config::{ConfigLoader, ConfigModeName, ConfigOverride, LoadConfigRequest},
    runtime::AgenaRuntime,
    storage::StorageConfig,
};
use anyhow::Context;
use app::{App, LaunchOptions};
use backend::Backend;
use clap::{Args, Parser, Subcommand};
use i18n::I18n;
use sea_orm::Database;
use tracing_subscriber::{
    EnvFilter, fmt::writer::MakeWriter, layer::SubscriberExt, util::SubscriberInitExt,
};

#[derive(Debug, Clone, Parser)]
#[command(name = "agena-tui", version, about = "Agena terminal chat application")]
struct AgenaTuiCli {
    #[arg(long, env = "AGENA_CONFIG", global = true)]
    config: Option<PathBuf>,
    #[arg(long, env = "AGENA_MODE", global = true)]
    mode: Option<ConfigModeName>,
    #[arg(short = 'c', long = "set", global = true)]
    overrides: Vec<ConfigOverride>,
    #[command(subcommand)]
    command: Option<TuiCommand>,
}

#[derive(Debug, Clone, Subcommand)]
enum TuiCommand {
    Run(RunCommand),
}

#[derive(Debug, Clone, Args)]
struct RunCommand {
    #[arg(long, env = "AGENA_DATABASE_URL")]
    database_url: Option<String>,
    #[arg(long, env = "AGENA_DATABASE_PATH")]
    database_path: Option<PathBuf>,
    #[arg(long)]
    workspace_root: Option<PathBuf>,
    #[arg(long)]
    session: Option<i64>,
    #[arg(long)]
    search: Option<String>,
    #[arg(long)]
    locale: Option<String>,
    #[arg(long, env = "AGENA_TUI_LOG_FILE", conflicts_with = "log_stderr")]
    log_file: Option<PathBuf>,
    #[arg(long, env = "AGENA_TUI_LOG_STDERR")]
    log_stderr: bool,
    #[arg(long, env = "AGENA_TUI_CONFIG")]
    tui_config: Option<PathBuf>,
}

impl AgenaTuiCli {
    fn resolved_command(&self) -> TuiCommand {
        self.command.clone().unwrap_or(TuiCommand::Run(RunCommand {
            database_url: None,
            database_path: None,
            workspace_root: None,
            session: None,
            search: None,
            locale: None,
            log_file: None,
            log_stderr: false,
            tui_config: None,
        }))
    }

    fn load_request(&self) -> LoadConfigRequest {
        LoadConfigRequest {
            config_path: self.config.clone(),
            mode: self.mode.clone(),
            overrides: self.overrides.clone(),
        }
    }

    async fn run(self, config_locale: Option<String>) -> Result<(), AppError> {
        match self.resolved_command() {
            TuiCommand::Run(command) => self.run_tui(command, config_locale).await,
        }
    }

    async fn run_tui(
        self,
        command: RunCommand,
        config_locale: Option<String>,
    ) -> Result<(), AppError> {
        let storage = StorageConfig {
            database_url: command.database_url,
            database_path: command.database_path,
        };
        let database_url = storage.resolve_url()?;
        StorageConfig::ensure_parent(database_url.as_str())?;

        let db = Arc::new(Database::connect(database_url.as_str()).await?);
        let workspace_root = command.workspace_root.unwrap_or(env::current_dir()?);
        let runtime = AgenaRuntime::builder()
            .with_load_request(self.load_request())
            .with_workspace_root(workspace_root.clone())
            .with_database_connection(db.as_ref().clone())
            .build()
            .await?;

        let backend = Backend::new(runtime, db, workspace_root.clone());
        let i18n = I18n::resolve(command.locale.as_deref(), config_locale.as_deref());
        let tui_config = tui_config::TuiConfig::load(command.tui_config.clone());
        let mut terminal = terminal::TerminalGuard::enter()
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let mut app = App::new(
            backend,
            LaunchOptions {
                initial_session_id: command.session,
                initial_session_search: command.search,
                workspace_root: Some(workspace_root),
                tui_config,
            },
            i18n,
        );

        let result = app
            .run(terminal.terminal_mut())
            .await
            .with_context(|| "failed while running agena-tui");
        terminal
            .restore()
            .map_err(|error| AppError::Internal(error.to_string()))?;
        result.map_err(|error| AppError::Internal(error.to_string()))
    }
}

#[derive(Clone)]
enum TuiLogWriter {
    Stderr,
    File(Arc<Mutex<File>>),
}

enum TuiLogGuardWriter {
    Stderr(Stderr),
    File(SharedFileWriter),
}

#[derive(Clone)]
struct SharedFileWriter {
    file: Arc<Mutex<File>>,
}

impl TuiLogWriter {
    fn ansi_enabled(&self) -> bool {
        matches!(self, Self::Stderr)
    }
}

impl Write for SharedFileWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut guard = self
            .file
            .lock()
            .map_err(|_| io::Error::other("tui log file lock poisoned"))?;
        guard.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut guard = self
            .file
            .lock()
            .map_err(|_| io::Error::other("tui log file lock poisoned"))?;
        guard.flush()
    }
}

impl Write for TuiLogGuardWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Stderr(stderr) => stderr.write(buf),
            Self::File(file) => file.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Stderr(stderr) => stderr.flush(),
            Self::File(file) => file.flush(),
        }
    }
}

impl<'a> MakeWriter<'a> for TuiLogWriter {
    type Writer = TuiLogGuardWriter;

    fn make_writer(&'a self) -> Self::Writer {
        match self {
            Self::Stderr => TuiLogGuardWriter::Stderr(io::stderr()),
            Self::File(file) => TuiLogGuardWriter::File(SharedFileWriter {
                file: Arc::clone(file),
            }),
        }
    }
}

fn resolve_tui_log_writer(cli: &AgenaTuiCli) -> Result<TuiLogWriter, AppError> {
    let TuiCommand::Run(command) = cli.resolved_command();
    if command.log_stderr {
        return Ok(TuiLogWriter::Stderr);
    }

    let path = command.log_file.unwrap_or_else(default_tui_log_path);
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }

    let file = OpenOptions::new().create(true).append(true).open(path)?;
    Ok(TuiLogWriter::File(Arc::new(Mutex::new(file))))
}

fn default_tui_log_path() -> PathBuf {
    let mut base = default_agena_dir();
    base.push("logs");
    base.push("agena-tui.log");
    base
}

fn default_agena_dir() -> PathBuf {
    let mut base = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.push(".agena");
    base
}

#[tokio::main]
async fn main() -> Result<(), AppError> {
    let cli = AgenaTuiCli::parse();
    let resolution = ConfigLoader::default().load(&cli.load_request()).ok();
    let filter = resolution
        .as_ref()
        .map(|resolution| resolution.config.tracing.filter.clone())
        .unwrap_or_else(|| "info".to_owned());
    let telemetry = resolution
        .as_ref()
        .map(|resolution| resolution.config.telemetry.clone())
        .unwrap_or_default();
    let config_locale = resolution.and_then(|resolution| resolution.config.ui.locale);
    let log_writer = resolve_tui_log_writer(&cli)?;

    let initial_filter = EnvFilter::try_new(filter).unwrap_or_else(|_| EnvFilter::new("info"));
    if let Some(telemetry) = agena_otel::build_layer(&telemetry)
        .map_err(|error| agena::AppError::Config(error.to_string()))?
    {
        let telemetry_layer = telemetry.layer();
        let _telemetry_guard = telemetry.guard;
        tracing_subscriber::registry()
            .with(initial_filter)
            .with(telemetry_layer)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(log_writer.clone())
                    .with_ansi(log_writer.ansi_enabled())
                    .with_target(false)
                    .compact(),
            )
            .init();
        cli.run(config_locale).await
    } else {
        tracing_subscriber::registry()
            .with(initial_filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(log_writer.clone())
                    .with_ansi(log_writer.ansi_enabled())
                    .with_target(false)
                    .compact(),
            )
            .init();
        cli.run(config_locale).await
    }
}
