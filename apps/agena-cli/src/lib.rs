mod app;
mod attachment_source;
mod backend;
mod clipboard;
mod commands;
mod composer_queue;
mod external_editor;
mod external_pager;
mod i18n;
mod iterm2;
mod short_link;
mod terminal;
mod tui_config;
mod tui_keymap;
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
    config::{ConfigLoader, ConfigOverride, LoadConfigRequest, TracingConfig},
    runtime::AgenaRuntime,
    storage::StorageConfig,
    tracing as tracing_config,
};
use anyhow::Context;
use app::{App, LaunchOptions};
use backend::Backend;
use clap::{Args, Parser, Subcommand};
use i18n::I18n;
use tracing_subscriber::{fmt::writer::MakeWriter, layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Debug, Clone, Default)]
pub struct TuiLaunchArgs {
    pub database_url: Option<String>,
    pub database_path: Option<PathBuf>,
    pub workspace_root: Option<PathBuf>,
    pub session: Option<i64>,
    pub search: Option<String>,
    pub locale: Option<String>,
    pub log_file: Option<PathBuf>,
    pub log_stderr: bool,
}

#[derive(Debug, Clone, Parser)]
#[command(
    name = "agena-tui",
    version,
    about = "Agena terminal UI",
    long_about = "Launch the Agena terminal UI."
)]
struct AgenaTuiCli {
    #[arg(short = 'c', long = "set", global = true)]
    overrides: Vec<ConfigOverride>,
    #[command(subcommand)]
    command: Option<TuiCommand>,
}

#[derive(Debug, Clone, Subcommand)]
enum TuiCommand {
    Run(RunCommand),
}

#[derive(Debug, Clone, Args, Default)]
struct RunCommand {
    #[arg(long, env = "AGENA_DATABASE_URL")]
    database_url: Option<String>,
    #[arg(long, env = "AGENA_DATABASE_PATH")]
    database_path: Option<PathBuf>,
    #[arg(long = "workspace")]
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
}

impl AgenaTuiCli {
    fn resolved_command(&self) -> TuiCommand {
        self.command
            .clone()
            .unwrap_or(TuiCommand::Run(RunCommand::default()))
    }

    fn load_request(&self) -> LoadConfigRequest {
        let workspace_root = match self.resolved_command() {
            TuiCommand::Run(command) => command.workspace_root,
        };
        LoadConfigRequest {
            overrides: self.overrides.clone(),
            workspace_root,
        }
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

pub async fn run_cli() -> Result<(), AppError> {
    let cli = AgenaTuiCli::parse();
    run_with_load_request(cli.load_request(), launch_args_from_cli(&cli)).await
}

pub async fn run_with_load_request(
    load_request: LoadConfigRequest,
    args: TuiLaunchArgs,
) -> Result<(), AppError> {
    let resolution = ConfigLoader::default().load(&load_request).ok();
    let tracing = resolution
        .as_ref()
        .map(|resolution| resolution.config.tracing.clone())
        .unwrap_or_else(TracingConfig::default);
    let log_writer = resolve_tui_log_writer(&args)?;

    let initial_filter = tracing_config::env_filter(&tracing).unwrap_or_else(|_| {
        tracing_config::env_filter(&TracingConfig::default())
            .expect("default tracing filter should parse")
    });
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

    run_embedded(load_request, args).await
}

pub async fn run_embedded(
    load_request: LoadConfigRequest,
    args: TuiLaunchArgs,
) -> Result<(), AppError> {
    let bootstrap_config = ConfigLoader::default().load(&load_request).ok();
    let config_locale = bootstrap_config
        .as_ref()
        .and_then(|resolution| resolution.config.ui.locale.clone());

    let storage = StorageConfig {
        database_url: args.database_url.clone(),
        database_path: args.database_path.clone(),
    };
    let database_url = storage.resolve_url()?;
    StorageConfig::ensure_parent(database_url.as_str())?;

    let tracing = bootstrap_config
        .as_ref()
        .map(|resolution| resolution.config.tracing.clone())
        .unwrap_or_default();
    let db = Arc::new(tracing_config::connect_database(database_url.as_str(), &tracing).await?);
    let workspace_root = args.workspace_root.unwrap_or(env::current_dir()?);
    let runtime = AgenaRuntime::new(agena::runtime::AgenaRuntimeConfig {
        load_request,
        workspace_root: Some(workspace_root.clone()),
        database_connection: Some(Arc::clone(&db)),
        database_url: None,
        auto_migrate: true,
        tracing_reload_handle: None,
    })
    .await?;

    let backend = Backend::new(runtime, db, workspace_root.clone());
    let i18n = I18n::resolve(args.locale.as_deref(), config_locale.as_deref());
    let tui_config = tui_config::TuiConfig::load(
        bootstrap_config
            .as_ref()
            .map(|resolution| &resolution.config.ui),
    );
    let mut terminal = terminal::TerminalRuntime::enter()
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let terminal_background = terminal.background();
    let mut app = App::new(
        backend,
        LaunchOptions {
            initial_session_id: args.session,
            initial_session_search: args.search,
            tui_config,
            terminal_background,
        },
        i18n,
    );

    let result = app
        .run(&mut terminal)
        .await
        .with_context(|| "failed while running agena-tui");
    terminal
        .restore()
        .map_err(|error| AppError::Internal(error.to_string()))?;
    result.map_err(|error| AppError::Internal(error.to_string()))
}

fn launch_args_from_cli(cli: &AgenaTuiCli) -> TuiLaunchArgs {
    match cli.resolved_command() {
        TuiCommand::Run(command) => TuiLaunchArgs {
            database_url: command.database_url,
            database_path: command.database_path,
            workspace_root: command.workspace_root,
            session: command.session,
            search: command.search,
            locale: command.locale,
            log_file: command.log_file,
            log_stderr: command.log_stderr,
        },
    }
}

fn resolve_tui_log_writer(args: &TuiLaunchArgs) -> Result<TuiLogWriter, AppError> {
    if args.log_stderr {
        return Ok(TuiLogWriter::Stderr);
    }

    let path = args.log_file.clone().unwrap_or_else(default_tui_log_path);
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
    base.push("agena");
    base
}
