use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{self, Stderr, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
};

use crate::error::AgenaProcessError;
use agena_runtime::bootstrap_application_services;

use agena_cli::TuiLaunchRequest;
use agena_tui::i18n::I18n;
use agena_tui_app::{App, LaunchOptions};
use agena_tui_backend::Backend;
use agena_tui_platform::terminal;
use anyhow::Context;
use tracing_subscriber::{fmt::writer::MakeWriter, layer::SubscriberExt, util::SubscriberInitExt};

pub(crate) async fn run(request: TuiLaunchRequest) -> Result<(), AgenaProcessError> {
    let args = request.args;
    let launch_args = TuiLaunchArgs {
        database_url: args.database_url,
        database_path: args.database_path,
        workspace_root: args.workspace,
        session: args.session,
        search: args.search,
        locale: args.locale,
        log_file: args.log_file,
        log_stderr: args.log_stderr,
        config_override_expressions: request.config_override_expressions,
    };
    init_tui_tracing(&launch_args.config_override_expressions, &launch_args)?;
    run_embedded(launch_args).await
}

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
    /// Raw `--set` expressions retained until the Runtime bootstrap boundary.
    pub config_override_expressions: Vec<String>,
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

/// Installs the process-wide tracing subscriber used by the terminal UI.
///
/// The binary calls this before launching the UI; the UI library itself never
/// installs global process state.
pub fn init_tui_tracing(
    config_override_expressions: &[String],
    args: &TuiLaunchArgs,
) -> Result<(), AgenaProcessError> {
    let tracing = agena_runtime::resolve_runtime_bootstrap_preflight(
        &agena_runtime::RuntimeBootstrapRequest {
            config_override_expressions: config_override_expressions.to_vec(),
            ..Default::default()
        },
    )
    .ok()
    .map(|preflight| preflight.tracing)
    .unwrap_or_default();
    let log_writer = resolve_tui_log_writer(args)?;

    let initial_filter = agena_runtime::runtime_env_filter(&tracing).unwrap_or_else(|_| {
        agena_runtime::runtime_env_filter(&agena_runtime::RuntimeTracingConfiguration::default())
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

    Ok(())
}

pub async fn run_embedded(args: TuiLaunchArgs) -> Result<(), AgenaProcessError> {
    let workspace_root = args.workspace_root.unwrap_or(env::current_dir()?);
    let runtime = bootstrap_application_services(agena_runtime::RuntimeBootstrapRequest {
        workspace_root: Some(workspace_root.clone()),
        database_url: args.database_url,
        database_path: args.database_path,
        scheduler_database_url: None,
        scheduler_database_path: None,
        initialize_schema: true,
        tracing_reload_handle: None,
        config_override_expressions: args.config_override_expressions.clone(),
    })
    .await
    .map_err(|error| AgenaProcessError::Internal(error.to_string()))?;

    let backend = Backend::new(runtime.application_services(), workspace_root.clone())
        .map_err(|error| AgenaProcessError::Internal(error.to_string()))?;
    let tui_preferences = backend.ui_configuration();
    let i18n = I18n::resolve(args.locale.as_deref(), tui_preferences.locale.as_deref());
    let tui_config = agena_tui_app::tui_config_from_preferences(&tui_preferences);
    let mut terminal = terminal::TerminalRuntime::enter(tui_config.graphics)
        .map_err(|error| AgenaProcessError::Internal(error.to_string()))?;
    let terminal_background = terminal.background();
    let math_graphics = terminal.math_graphics();
    let math_protocol = math_graphics.protocol_name();
    let terminal_context = terminal.context().clone();
    let terminal_summary = terminal_context.diagnostic_summary();
    tracing::debug!(
        terminal = %terminal_summary,
        math_protocol,
        "detected TUI terminal environment"
    );
    for diagnostic in terminal_context.diagnostics() {
        tracing::warn!(
            code = diagnostic.code,
            message = %diagnostic.message,
            "terminal compatibility diagnostic"
        );
    }
    let mut app = App::new(
        backend,
        LaunchOptions {
            initial_session_id: args.session,
            initial_session_search: args.search,
            tui_config,
            terminal_background,
            terminal_context: Some(terminal_context),
            math_graphics: Some(math_graphics),
        },
        i18n,
    );

    let result = app
        .run(&mut terminal)
        .await
        .with_context(|| "failed while running terminal UI");
    let restore_result = terminal.restore();
    runtime.shutdown();
    match (result, restore_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(AgenaProcessError::Internal(format!("{error:#}"))),
        (Ok(()), Err(error)) => Err(AgenaProcessError::Internal(format!(
            "failed to restore the terminal: {error:#}"
        ))),
        (Err(run_error), Err(restore_error)) => Err(AgenaProcessError::Internal(format!(
            "{run_error:#}; terminal restoration also failed: {restore_error:#}"
        ))),
    }
}

fn resolve_tui_log_writer(args: &TuiLaunchArgs) -> Result<TuiLogWriter, AgenaProcessError> {
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
    base.push("agena.log");
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
