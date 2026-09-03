use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{self, Stderr, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
};

use crate::error::AgenaProcessError;

use agena_cli::TuiLaunchRequest;
use agena_tui::i18n::I18n;
use agena_tui_app::{App, LaunchOptions, TuiBackend};
use agena_tui_platform::terminal;
use anyhow::Context;
use tracing_subscriber::{fmt::writer::MakeWriter, layer::SubscriberExt, util::SubscriberInitExt};

pub(crate) async fn run(request: TuiLaunchRequest) -> Result<(), AgenaProcessError> {
    let args = request.args;
    let launch_args = TuiLaunchArgs {
        server_url: super::server_client::resolve_server_url(args.server),
        server_token: args.server_token,
        server_password: args.server_password,
        workspace_root: args.workspace,
        session: args.session,
        search: args.search,
        locale: args.locale,
        log_file: args.log_file,
        log_stderr: args.log_stderr,
        config_override_expressions: request.config_override_expressions,
    };
    init_tui_tracing(&launch_args.config_override_expressions, &launch_args)?;
    run_remote(launch_args).await
}

#[cfg(test)]
mod tests {
    use super::resolve_tui_workspace_root;
    use std::{env, path::PathBuf};

    #[test]
    fn explicit_tui_workspace_wins_over_process_cwd() {
        let explicit = PathBuf::from("/tmp/agena-explicit-workspace");
        assert_eq!(
            resolve_tui_workspace_root(Some(explicit.clone())).expect("resolve explicit workspace"),
            explicit
        );
    }

    #[test]
    fn tui_workspace_defaults_to_process_cwd() {
        let expected = env::current_dir().expect("read test process cwd");
        assert_eq!(
            resolve_tui_workspace_root(None).expect("resolve default TUI workspace"),
            expected
        );
    }
}

fn resolve_tui_workspace_root(explicit: Option<PathBuf>) -> io::Result<PathBuf> {
    explicit.map(Ok).unwrap_or_else(env::current_dir)
}

#[derive(Debug, Clone, Default)]
pub struct TuiLaunchArgs {
    pub server_url: String,
    pub server_token: Option<String>,
    pub server_password: Option<String>,
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
    _config_override_expressions: &[String],
    args: &TuiLaunchArgs,
) -> Result<(), AgenaProcessError> {
    // The TUI is a pure HTTP client; runtime-owned tracing is configured by
    // the server, so the terminal uses the default runtime filter.
    let tracing = agena_runtime::RuntimeTracingConfiguration::default();
    let log_writer = resolve_tui_log_writer(args)?;

    let initial_filter = agena_runtime::runtime_env_filter(&tracing).map_err(|error| {
        AgenaProcessError::Configuration(
            agena_failure::diagnostic::format_error_chain_with_context(
                "invalid TUI tracing configuration",
                &error,
            ),
        )
    })?;
    tracing_subscriber::registry()
        .with(initial_filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(log_writer.clone())
                .with_ansi(log_writer.ansi_enabled())
                .with_target(false)
                .compact(),
        )
        .try_init()
        .map_err(|error| {
            AgenaProcessError::Internal(agena_failure::diagnostic::format_error_chain_with_context(
                "failed to install the TUI tracing subscriber",
                &error,
            ))
        })?;

    Ok(())
}

pub async fn run_remote(args: TuiLaunchArgs) -> Result<(), AgenaProcessError> {
    // The CLI workspace is the directory the user launched Agena from unless
    // they explicitly selected another one. Do not silently inherit the
    // long-lived server process workspace here: installed servers commonly run
    // from $HOME, which would make `cd project && agena` create sessions in ~.
    let workspace_root = resolve_tui_workspace_root(args.workspace_root.clone())?;
    let backend = TuiBackend::connect_remote_authenticated(
        args.server_url.as_str(),
        Some(workspace_root),
        args.server_token.as_deref(),
        args.server_password.as_deref(),
    )
    .await
    .map_err(|error| {
        AgenaProcessError::from_anyhow(
            error.context(format!("cannot connect TUI to server {}", args.server_url)),
        )
    })?;
    // The server's resolved UI preferences are cached on the backend at
    // connect; project them into the terminal configuration so the client
    // launches with the same theme/graphics/locale as the server's runtime.
    let preferences = backend.tui_preferences();
    let tui_config = agena_tui_app::tui_config_from_preferences(&preferences);
    let i18n = I18n::resolve(args.locale.as_deref(), preferences.locale.as_deref());
    run_app(backend, tui_config, i18n, &args).await
}

async fn run_app(
    backend: TuiBackend,
    tui_config: agena_tui::presentation_config::TuiConfig,
    i18n: I18n,
    args: &TuiLaunchArgs,
) -> Result<(), AgenaProcessError> {
    let mut terminal = terminal::TerminalRuntime::enter(tui_config.graphics)
        .map_err(|error| AgenaProcessError::internal_error(error.as_ref()))?;
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
    let mut app = App::new_with_backend(
        backend,
        LaunchOptions {
            initial_session_id: args.session,
            initial_session_search: args.search.clone(),
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
    match (result, restore_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(AgenaProcessError::from_anyhow(error)),
        (Ok(()), Err(error)) => Err(AgenaProcessError::Internal(format!(
            "failed to restore the terminal: {}",
            agena_failure::diagnostic::format_error_chain(error.as_ref())
        ))),
        (Err(run_error), Err(restore_error)) => Err(AgenaProcessError::Internal(format!(
            "{}; terminal restoration also failed: {}",
            agena_failure::diagnostic::format_error_chain(run_error.as_ref()),
            agena_failure::diagnostic::format_error_chain(restore_error.as_ref())
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
        .unwrap_or_else(|error| {
            tracing::error!(
                diagnostic = %error,
                "TUI data home is unavailable; using the current-directory compatibility path"
            );
            PathBuf::from(".")
        });
    base.push("agena");
    base
}
