use std::{
    fs,
    io::{self, Write},
    path::PathBuf,
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use clap::{Args, Parser, Subcommand};
use serde::Serialize;

use crate::{
    agent::Agent,
    config::{
        ConfigEnvironment, ConfigLoader, ConfigModeName, ConfigOutputFormat, ConfigOverride,
        LoadConfigRequest, ProcessEnvironment,
    },
    error::AppError,
    message::{ApplyPatchToolInput, BuiltinToolInput, PartContent},
    model::ModelRef,
    permission::{PermissionPolicy, ToolPermissionPolicy},
    provider::{
        ModelCapabilities, ModelMetadata, ProviderModel,
        auth::{
            AuthData, AuthManager, ConfiguredAuthStore, CopilotDeployment, wait_for_oauth_callback,
        },
    },
    role::Role,
    runtime::{AgenaRuntime, TracingFilterReloadHandle},
    session::{
        Session, SessionContinueRequest, SessionCreateRequest, SessionForkRequest,
        SessionListRequest, SessionRunOptions, SessionRuntimeStatus, SessionSummary,
        SessionUserTurnRequest,
    },
    storage::StorageConfig,
    tool::{ApplyPatchExecution, ToolExecutor},
};

#[derive(Debug, Clone, Parser)]
#[command(name = "agena", version, about = "Agena backend CLI")]
pub struct AgenaCli {
    #[arg(long, env = "AGENA_CONFIG", global = true)]
    pub config: Option<PathBuf>,
    #[arg(long, env = "AGENA_MODE", global = true)]
    pub mode: Option<ConfigModeName>,
    #[arg(short = 'c', long = "set", global = true)]
    pub overrides: Vec<ConfigOverride>,
    #[arg(long, env = "AGENA_DATABASE_URL", global = true)]
    pub database_url: Option<String>,
    #[arg(long, env = "AGENA_DATABASE_PATH", global = true)]
    pub database_path: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Option<AgenaCommand>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum AgenaCommand {
    Apply(ApplyArgs),
    Auth(AuthCommand),
    Config(ConfigCommand),
    Continue(ContinueArgs),
    Debug(DebugCommand),
    Exec(ExecArgs),
    Fork(ForkArgs),
    Login(LoginArgs),
    Logout(LogoutArgs),
    Provider(ProviderCommand),
    Resume(ResumeArgs),
    Review(ReviewArgs),
    Serve(ServeCommand),
    Sessions(SessionsCommand),
}

#[derive(Debug, Clone, Args)]
pub struct AuthCommand {
    #[command(subcommand)]
    pub command: Option<AuthSubcommand>,
}

#[derive(Debug, Clone, Args)]
pub struct ConfigCommand {
    #[command(subcommand)]
    pub command: Option<ConfigSubcommand>,
}

#[derive(Debug, Clone, Args)]
pub struct DebugCommand {
    #[command(subcommand)]
    pub command: DebugSubcommand,
}

#[derive(Debug, Clone, Args)]
pub struct ProviderCommand {
    #[command(subcommand)]
    pub command: Option<ProviderSubcommand>,
}

#[derive(Debug, Clone, Args)]
pub struct SessionsCommand {
    #[command(subcommand)]
    pub command: Option<SessionsSubcommand>,
}

#[derive(Debug, Clone, Args, Default)]
pub struct ServeCommand {}

#[derive(Debug, Clone, Subcommand)]
pub enum AuthSubcommand {
    List(AuthListArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum ConfigSubcommand {
    Resolve(ConfigResolveArgs),
    Validate,
    Mode(ConfigModeArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum DebugSubcommand {
    Session(DebugSessionArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum ProviderSubcommand {
    List(ProviderListArgs),
    Models(ProviderModelsArgs),
    Capabilities(ProviderCapabilitiesArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum SessionsSubcommand {
    List(SessionListArgs),
}

#[derive(Debug, Clone, Args)]
pub struct AuthListArgs {
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct LoginArgs {
    pub provider_id: String,
    #[arg(long)]
    pub api_key: Option<String>,
    #[arg(long)]
    pub browser: bool,
    #[arg(long)]
    pub device: bool,
    #[arg(long, default_value_t = 1455)]
    pub port: u16,
    #[arg(long, default_value_t = 600)]
    pub timeout_secs: u64,
    #[arg(long, default_value = "https://gitlab.com")]
    pub instance_url: String,
    #[arg(long)]
    pub enterprise_domain: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct LogoutArgs {
    pub provider_id: String,
}

#[derive(Debug, Clone, Args)]
pub struct SessionListArgs {
    #[arg(long, default_value_t = 20)]
    pub limit: u64,
    #[arg(long, default_value_t = 0)]
    pub offset: u64,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct ResumeArgs {
    pub session_id: Option<i64>,
    #[arg(long)]
    pub last: bool,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct ContinueArgs {
    pub session_id: Option<i64>,
    #[arg(long)]
    pub last: bool,
    #[arg(long)]
    pub model: Option<String>,
    #[arg(long)]
    pub temperature: Option<f32>,
    #[arg(long)]
    pub max_output_tokens: Option<u32>,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct ApplyArgs {
    #[arg(long = "workspace", alias = "cwd")]
    pub workspace: Option<PathBuf>,
    #[arg(long)]
    pub json: bool,
    pub patch_file: PathBuf,
}

#[derive(Debug, Clone, Args)]
pub struct DebugSessionArgs {
    pub session_id: i64,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Args)]
pub struct ExecArgs {
    #[arg(long = "workspace", alias = "cwd")]
    pub workspace: Option<PathBuf>,
    #[arg(long)]
    pub model: Option<String>,
    #[arg(long)]
    pub temperature: Option<f32>,
    #[arg(long)]
    pub max_output_tokens: Option<u32>,
    #[arg(long)]
    pub json: bool,
    pub prompt: String,
}

#[derive(Debug, Clone, Args)]
pub struct ReviewArgs {
    #[arg(long = "workspace", alias = "cwd")]
    pub workspace: Option<PathBuf>,
    #[arg(long, default_value = "main")]
    pub base: String,
    #[arg(long)]
    pub model: Option<String>,
    #[arg(long)]
    pub temperature: Option<f32>,
    #[arg(long)]
    pub max_output_tokens: Option<u32>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Args)]
pub struct ForkArgs {
    pub session_id: i64,
    #[arg(long)]
    pub at: i64,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct ConfigResolveArgs {
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct ConfigModeArgs {
    #[arg(long)]
    pub list: bool,
    pub name: Option<ConfigModeName>,
}

#[derive(Debug, Clone, Args)]
pub struct ProviderListArgs {
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct ProviderModelsArgs {
    pub provider_id: String,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct ProviderCapabilitiesArgs {
    pub target: String,
    #[arg(long)]
    pub model: Option<String>,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Serialize)]
struct AuthListOutput {
    credentials: Vec<AuthSummary>,
}

#[derive(Debug, Serialize)]
struct AuthSummary {
    provider_id: String,
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    enterprise_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
struct SessionListOutput {
    sessions: Vec<SessionSummary>,
}

#[derive(Debug, Serialize)]
struct SessionOutput {
    session: SessionDetail,
}

#[derive(Debug, Serialize)]
struct SessionForkOutput {
    source_session_id: i64,
    forked: SessionDetail,
}

#[derive(Debug, Serialize)]
struct ApplyOutput {
    title: String,
    output_text: String,
    patch: ApplyPatchExecution,
}

#[derive(Debug, Serialize)]
struct ExecOutput {
    session: SessionDetail,
    text: String,
}

#[derive(Debug, Serialize)]
struct DebugSessionOutput {
    session: SessionDetail,
    messages: Vec<DebugMessageOutput>,
}

#[derive(Debug, Serialize)]
struct DebugMessageOutput {
    id: i64,
    role: Role,
    state: crate::message::MessageStatus,
    text: String,
}

#[derive(Debug, Serialize)]
struct SessionDetail {
    id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_id: Option<i64>,
    workspace_id: i64,
    title: String,
    version: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    message_count: usize,
    status: SessionRuntimeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_event_seq: Option<i64>,
}

#[derive(Debug, Serialize)]
struct ProviderListOutput {
    providers: Vec<ProviderSummary>,
}

#[derive(Debug, Serialize)]
struct ProviderSummary {
    provider_id: String,
    default_model: String,
    default_model_ref: String,
}

#[derive(Debug, Serialize)]
struct ProviderModelsOutput {
    provider_id: String,
    models: Vec<ProviderModel>,
}

#[derive(Debug, Serialize)]
struct ProviderCapabilitiesOutput {
    provider_id: String,
    model: String,
    model_ref: String,
    capabilities: ModelCapabilities,
    metadata: ModelMetadata,
}

impl AgenaCli {
    pub async fn run(
        self,
        tracing_reload_handle: Option<TracingFilterReloadHandle>,
    ) -> Result<(), AppError> {
        let loader = ConfigLoader::new(ProcessEnvironment);

        match self.command.clone() {
            Some(AgenaCommand::Apply(args)) => self.run_apply(args),
            Some(AgenaCommand::Auth(command)) => self.run_auth(loader, command).await,
            Some(AgenaCommand::Config(command)) => self.run_config(loader, command),
            Some(AgenaCommand::Continue(args)) => self.run_continue(args).await,
            Some(AgenaCommand::Debug(command)) => self.run_debug(command).await,
            Some(AgenaCommand::Exec(args)) => self.run_exec(args).await,
            Some(AgenaCommand::Fork(args)) => self.run_fork(args).await,
            Some(AgenaCommand::Login(args)) => self.run_login(loader, args).await,
            Some(AgenaCommand::Logout(args)) => self.run_logout(loader, args),
            Some(AgenaCommand::Provider(command)) => self.run_provider(loader, command).await,
            Some(AgenaCommand::Resume(args)) => self.run_resume(args).await,
            Some(AgenaCommand::Review(args)) => self.run_review(args).await,
            Some(AgenaCommand::Serve(_command)) => Err(AppError::Config(
                "the HTTP server moved to the `apps/agena-http-api-server` app; run `cargo run -p agena-http-api-server -- --help` from the repository root".to_owned(),
            )),
            Some(AgenaCommand::Sessions(command)) => self.run_sessions(command).await,
            None => self.run_default(loader, tracing_reload_handle).await,
        }
    }

    async fn run_default(
        self,
        loader: ConfigLoader<ProcessEnvironment>,
        tracing_reload_handle: Option<TracingFilterReloadHandle>,
    ) -> Result<(), AppError> {
        let resolution = loader.load(&self.load_request())?;
        let mut builder = AgenaRuntime::builder().with_load_request(self.load_request());
        if let Some(handle) = tracing_reload_handle {
            builder = builder.with_tracing_reload_handle(handle);
        }
        let runtime = builder.build().await?;
        let snapshot = runtime.current_snapshot();
        let mode = resolution
            .meta
            .active_mode
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "default".to_owned());
        tracing::info!(
            mode,
            generation = snapshot.generation(),
            providers = snapshot.provider_registry().provider_ids().len(),
            plugins = snapshot.plugin_manager().plugins().len(),
            sessions = snapshot.session_manager().is_some(),
            "Agena started with resolved configuration"
        );
        Ok(())
    }

    fn run_apply(self, args: ApplyArgs) -> Result<(), AppError> {
        let output = self.render_apply_command(args)?;
        println!("{output}");
        Ok(())
    }

    async fn run_auth(
        self,
        loader: ConfigLoader<ProcessEnvironment>,
        command: AuthCommand,
    ) -> Result<(), AppError> {
        let output = self.render_auth_command(&loader, command).await?;
        println!("{output}");
        Ok(())
    }

    async fn run_login(
        self,
        loader: ConfigLoader<ProcessEnvironment>,
        args: LoginArgs,
    ) -> Result<(), AppError> {
        let manager = self.auth_manager(&loader)?;
        let provider_id = normalize_login_provider(args.provider_id.as_str());
        let method_count = usize::from(args.api_key.is_some())
            + usize::from(args.browser)
            + usize::from(args.device);
        if method_count != 1 {
            return Err(AppError::Config(
                "login requires exactly one of --api-key, --browser, or --device".to_owned(),
            ));
        }

        if let Some(api_key) = args.api_key {
            manager.set_api_key(provider_id.as_str(), api_key)?;
            println!("logged in: {provider_id}");
            return Ok(());
        }

        if args.browser {
            match provider_id.as_str() {
                "openai" => {
                    let redirect_uri = format!("http://localhost:{}/auth/callback", args.port);
                    let start = manager.start_openai_browser_login(redirect_uri.clone())?;
                    println!("open this URL to continue: {}", start.authorize_url);
                    io::stdout().flush()?;
                    let callback = wait_for_oauth_callback(
                        args.port,
                        start.state.as_str(),
                        Duration::from_secs(args.timeout_secs),
                    )?;
                    manager
                        .finish_openai_browser_login(
                            callback.code,
                            start.pkce_verifier,
                            redirect_uri,
                        )
                        .await?;
                }
                "gitlab" => {
                    let redirect_uri = format!("http://localhost:{}/auth/callback", args.port);
                    let start = manager
                        .start_gitlab_login(args.instance_url.clone(), redirect_uri.clone())?;
                    println!("open this URL to continue: {}", start.authorize_url);
                    io::stdout().flush()?;
                    let callback = wait_for_oauth_callback(
                        args.port,
                        start.state.as_str(),
                        Duration::from_secs(args.timeout_secs),
                    )?;
                    manager
                        .finish_gitlab_login(
                            args.instance_url,
                            callback.code,
                            start.pkce_verifier,
                            redirect_uri,
                        )
                        .await?;
                }
                _ => {
                    return Err(AppError::Config(format!(
                        "{provider_id} does not support browser login"
                    )));
                }
            }
            println!("logged in: {provider_id}");
            return Ok(());
        }

        if args.device {
            match provider_id.as_str() {
                "openai" => {
                    let start = manager.start_openai_headless_login().await?;
                    println!("open this URL: {}", start.verification_url);
                    println!("enter code: {}", start.user_code);
                    io::stdout().flush()?;
                    let auth = poll_until(
                        Duration::from_secs(args.timeout_secs),
                        Duration::from_secs(start.interval_seconds.max(1)),
                        || {
                            manager.poll_openai_headless_login(
                                start.device_code.clone(),
                                start.user_code.clone(),
                            )
                        },
                    )
                    .await?;
                    if auth.is_none() {
                        return Err(AppError::Config("openai device login timed out".to_owned()));
                    }
                }
                "github-copilot" | "github-copilot-enterprise" => {
                    let deployment = if provider_id == "github-copilot-enterprise" {
                        let domain = args.enterprise_domain.ok_or_else(|| {
                            AppError::Config(
                                "github-copilot-enterprise login requires --enterprise-domain"
                                    .to_owned(),
                            )
                        })?;
                        CopilotDeployment::Enterprise { domain }
                    } else {
                        CopilotDeployment::GitHubCom
                    };
                    let start = manager.start_copilot_login(deployment.clone()).await?;
                    println!("open this URL: {}", start.verification_url);
                    println!("enter code: {}", start.user_code);
                    io::stdout().flush()?;
                    let auth = poll_until(
                        Duration::from_secs(args.timeout_secs),
                        Duration::from_secs(start.interval_seconds.max(1)),
                        || {
                            manager
                                .poll_copilot_login(start.device_code.clone(), deployment.clone())
                        },
                    )
                    .await?;
                    if auth.is_none() {
                        return Err(AppError::Config(
                            "copilot device login timed out".to_owned(),
                        ));
                    }
                }
                _ => {
                    return Err(AppError::Config(format!(
                        "{provider_id} does not support device login"
                    )));
                }
            }
            println!("logged in: {provider_id}");
        }

        Ok(())
    }

    fn run_logout(
        self,
        loader: ConfigLoader<ProcessEnvironment>,
        args: LogoutArgs,
    ) -> Result<(), AppError> {
        let manager = self.auth_manager(&loader)?;
        let provider_id = normalize_login_provider(args.provider_id.as_str());
        manager.remove(provider_id.as_str())?;
        println!("logged out: {provider_id}");
        Ok(())
    }

    async fn run_provider(
        self,
        loader: ConfigLoader<ProcessEnvironment>,
        command: ProviderCommand,
    ) -> Result<(), AppError> {
        let output = self.render_provider_command(&loader, command).await?;
        println!("{output}");
        Ok(())
    }

    async fn run_sessions(self, command: SessionsCommand) -> Result<(), AppError> {
        let output = self.render_sessions_command(command).await?;
        println!("{output}");
        Ok(())
    }

    async fn run_resume(self, args: ResumeArgs) -> Result<(), AppError> {
        let output = self.render_resume_command(args).await?;
        println!("{output}");
        Ok(())
    }

    async fn run_continue(self, args: ContinueArgs) -> Result<(), AppError> {
        let output = self.render_continue_command(args).await?;
        println!("{output}");
        Ok(())
    }

    async fn run_debug(self, command: DebugCommand) -> Result<(), AppError> {
        let output = self.render_debug_command(command).await?;
        println!("{output}");
        Ok(())
    }

    async fn run_exec(self, args: ExecArgs) -> Result<(), AppError> {
        let output = self.render_exec_command(args).await?;
        println!("{output}");
        Ok(())
    }

    async fn run_fork(self, args: ForkArgs) -> Result<(), AppError> {
        let output = self.render_fork_command(args).await?;
        println!("{output}");
        Ok(())
    }

    async fn run_review(self, args: ReviewArgs) -> Result<(), AppError> {
        let output = self.render_review_command(args).await?;
        println!("{output}");
        Ok(())
    }

    fn run_config(
        self,
        loader: ConfigLoader<ProcessEnvironment>,
        command: ConfigCommand,
    ) -> Result<(), AppError> {
        match command
            .command
            .unwrap_or(ConfigSubcommand::Resolve(ConfigResolveArgs {
                format: ConfigOutputFormat::Toml,
            })) {
            ConfigSubcommand::Resolve(args) => {
                let resolution = loader.load(&self.load_request())?;
                println!("{}", resolution.render(args.format)?);
            }
            ConfigSubcommand::Validate => {
                let resolution = loader.load(&self.load_request())?;
                println!(
                    "config valid: path={}, mode={}",
                    resolution.meta.config_path.display(),
                    resolution
                        .meta
                        .active_mode
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| "<none>".to_owned())
                );
            }
            ConfigSubcommand::Mode(args) => {
                if args.list {
                    let modes = loader.list_modes(self.config.clone())?;
                    if modes.is_empty() {
                        println!("<no modes>");
                    } else {
                        for mode in modes {
                            println!("{mode}");
                        }
                    }
                } else {
                    let mut request = self.load_request();
                    request.mode = args.name.or(request.mode);
                    let resolution = loader.load(&request)?;
                    println!(
                        "{}",
                        resolution
                            .meta
                            .active_mode
                            .map(|mode| mode.to_string())
                            .unwrap_or_else(|| "<none>".to_owned())
                    );
                }
            }
        }

        Ok(())
    }

    fn render_apply_command(&self, args: ApplyArgs) -> Result<String, AppError> {
        let patch = fs::read_to_string(&args.patch_file)?;
        let workspace = args
            .workspace
            .clone()
            .map(Ok)
            .unwrap_or_else(std::env::current_dir)?;
        let executor = ToolExecutor::new(
            workspace,
            Agent::new("cli", PermissionPolicy::allow_all())
                .with_tool_policy(ToolPermissionPolicy::allow_all()),
        );
        let execution = executor
            .execute_builtin_detailed(&BuiltinToolInput::ApplyPatch(ApplyPatchToolInput { patch }))
            .map_err(|err| AppError::Config(err.to_string()))?;
        let patch = execution.apply_patch.ok_or_else(|| {
            AppError::Internal("apply_patch builtin did not return patch metadata".to_owned())
        })?;
        if args.json {
            render_serialized(
                ConfigOutputFormat::Json,
                &ApplyOutput {
                    title: execution.view.title,
                    output_text: execution.view.output_text,
                    patch,
                },
            )
        } else {
            Ok(format_apply_output(&patch))
        }
    }

    async fn render_auth_command<E>(
        &self,
        loader: &ConfigLoader<E>,
        command: AuthCommand,
    ) -> Result<String, AppError>
    where
        E: ConfigEnvironment,
    {
        let manager = self.auth_manager(loader)?;
        match command
            .command
            .unwrap_or(AuthSubcommand::List(AuthListArgs {
                format: ConfigOutputFormat::Toml,
            })) {
            AuthSubcommand::List(args) => {
                let mut credentials = manager
                    .all()?
                    .into_iter()
                    .map(|(provider_id, auth)| auth_summary(provider_id, auth))
                    .collect::<Vec<_>>();
                credentials.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
                render_serialized(args.format, &AuthListOutput { credentials })
            }
        }
    }

    async fn render_sessions_command(&self, command: SessionsCommand) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let manager = runtime
            .session_manager()
            .ok_or_else(session_storage_error)?;
        match command
            .command
            .unwrap_or(SessionsSubcommand::List(SessionListArgs {
                limit: 20,
                offset: 0,
                format: ConfigOutputFormat::Toml,
            })) {
            SessionsSubcommand::List(args) => {
                let sessions = manager
                    .list_session_summaries(SessionListRequest {
                        offset: args.offset,
                        limit: Some(args.limit),
                    })
                    .await?;
                render_serialized(args.format, &SessionListOutput { sessions })
            }
        }
    }

    async fn render_resume_command(&self, args: ResumeArgs) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let manager = runtime
            .session_manager()
            .ok_or_else(session_storage_error)?;
        let session_id = selected_session_id(&manager, args.session_id, args.last).await?;
        let session = manager.get_session(session_id).await?;
        let latest_event_seq = latest_event_seq(&manager, session.id).await?;
        render_serialized(
            args.format,
            &SessionOutput {
                session: session_detail(&session, latest_event_seq),
            },
        )
    }

    async fn render_continue_command(&self, args: ContinueArgs) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let manager = runtime
            .session_manager()
            .ok_or_else(session_storage_error)?;
        let session_id = selected_session_id(&manager, args.session_id, args.last).await?;
        let session = manager.get_session(session_id).await?;
        let options = resolve_continue_options(&runtime, &session, &args)?;
        let session = manager
            .continue_session(SessionContinueRequest {
                session_id,
                options,
            })
            .await?;
        let latest_event_seq = latest_event_seq(&manager, session.id).await?;
        render_serialized(
            args.format,
            &SessionOutput {
                session: session_detail(&session, latest_event_seq),
            },
        )
    }

    async fn render_debug_command(&self, command: DebugCommand) -> Result<String, AppError> {
        match command.command {
            DebugSubcommand::Session(args) => self.render_debug_session_command(args).await,
        }
    }

    async fn render_debug_session_command(
        &self,
        args: DebugSessionArgs,
    ) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let manager = runtime
            .session_manager()
            .ok_or_else(session_storage_error)?;
        let session = manager.get_session(args.session_id).await?;
        let latest_event_seq = latest_event_seq(&manager, session.id).await?;
        let output = DebugSessionOutput {
            session: session_detail(&session, latest_event_seq),
            messages: session
                .messages
                .iter()
                .map(|message| DebugMessageOutput {
                    id: message.id,
                    role: message.role,
                    state: message.state,
                    text: message.as_text_lossy(),
                })
                .collect(),
        };
        if args.json {
            render_serialized(ConfigOutputFormat::Json, &output)
        } else {
            Ok(format_debug_session_output(&output))
        }
    }

    async fn render_exec_command(&self, args: ExecArgs) -> Result<String, AppError> {
        self.render_prompt_command(
            args.workspace.as_ref(),
            args.prompt.as_str(),
            title_from_prompt(args.prompt.as_str()),
            args.model.as_deref(),
            args.temperature,
            args.max_output_tokens,
            args.json,
        )
        .await
    }

    async fn render_review_command(&self, args: ReviewArgs) -> Result<String, AppError> {
        let prompt = review_prompt(args.base.as_str());
        self.render_prompt_command(
            args.workspace.as_ref(),
            prompt.as_str(),
            format!("Review changes against {}", args.base),
            args.model.as_deref(),
            args.temperature,
            args.max_output_tokens,
            args.json,
        )
        .await
    }

    async fn render_prompt_command(
        &self,
        workspace: Option<&PathBuf>,
        prompt: &str,
        title: String,
        model: Option<&str>,
        temperature: Option<f32>,
        max_output_tokens: Option<u32>,
        json: bool,
    ) -> Result<String, AppError> {
        let runtime = self.session_runtime_with_workspace(workspace).await?;
        let manager = runtime
            .session_manager()
            .ok_or_else(session_storage_error)?;
        let options = resolve_run_options(&runtime, model, temperature, max_output_tokens)?;
        let created = manager
            .create_session(SessionCreateRequest {
                title,
                parent_session_id: None,
            })
            .await?;
        let session = manager
            .submit_user_turn(SessionUserTurnRequest {
                session_id: created.id,
                options,
                parts: vec![PartContent::text(prompt)],
            })
            .await?;
        if session.runtime.turn.status == SessionRuntimeStatus::Blocked {
            return Err(AppError::Config(
                "command is blocked awaiting permission or user input".to_owned(),
            ));
        }
        let latest_event_seq = latest_event_seq(&manager, session.id).await?;
        let text = last_assistant_text(&session).unwrap_or_default();
        if json {
            render_serialized(
                ConfigOutputFormat::Json,
                &ExecOutput {
                    session: session_detail(&session, latest_event_seq),
                    text,
                },
            )
        } else {
            Ok(text)
        }
    }

    async fn render_fork_command(&self, args: ForkArgs) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let manager = runtime
            .session_manager()
            .ok_or_else(session_storage_error)?;
        let forked = manager
            .fork_session(SessionForkRequest {
                session_id: args.session_id,
                at_event_seq: args.at,
                title: args.title,
            })
            .await?;
        let latest_event_seq = latest_event_seq(&manager, forked.id).await?;
        render_serialized(
            args.format,
            &SessionForkOutput {
                source_session_id: args.session_id,
                forked: session_detail(&forked, latest_event_seq),
            },
        )
    }

    async fn session_runtime(&self) -> Result<AgenaRuntime, AppError> {
        self.session_runtime_with_workspace(None).await
    }

    async fn session_runtime_with_workspace(
        &self,
        workspace: Option<&PathBuf>,
    ) -> Result<AgenaRuntime, AppError> {
        let storage = StorageConfig {
            database_url: self.database_url.clone(),
            database_path: self.database_path.clone(),
        };
        let database_url = storage.resolve_url()?;
        StorageConfig::ensure_parent(database_url.as_str())?;
        let mut builder = AgenaRuntime::builder()
            .with_load_request(self.load_request())
            .with_database_url(database_url);
        if let Some(workspace) = workspace {
            builder = builder.with_workspace_root(workspace.clone());
        }
        builder.build().await
    }

    fn auth_manager<E>(
        &self,
        loader: &ConfigLoader<E>,
    ) -> Result<AuthManager<ConfiguredAuthStore>, AppError>
    where
        E: ConfigEnvironment,
    {
        let resolution = loader.load(&self.load_request())?;
        Ok(AuthManager::new(resolution.config.auth_store()))
    }

    async fn render_provider_command<E>(
        &self,
        loader: &ConfigLoader<E>,
        command: ProviderCommand,
    ) -> Result<String, AppError>
    where
        E: ConfigEnvironment,
    {
        let resolution = loader.load(&self.load_request())?;
        let registry = resolution
            .config
            .build_provider_registry_with_env(loader.environment())?;

        match command
            .command
            .unwrap_or(ProviderSubcommand::List(ProviderListArgs {
                format: ConfigOutputFormat::Toml,
            })) {
            ProviderSubcommand::List(args) => {
                let mut providers = registry
                    .provider_ids()
                    .into_iter()
                    .filter_map(|provider_id| {
                        registry
                            .get(provider_id.as_str())
                            .map(|provider| ProviderSummary {
                                default_model_ref: format!(
                                    "{provider_id}/{}",
                                    provider.default_model()
                                ),
                                default_model: provider.default_model().to_string(),
                                provider_id,
                            })
                    })
                    .collect::<Vec<_>>();
                providers.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
                render_serialized(args.format, &ProviderListOutput { providers })
            }
            ProviderSubcommand::Models(args) => {
                let models = registry.list_models(args.provider_id.as_str()).await?;
                render_serialized(
                    args.format,
                    &ProviderModelsOutput {
                        provider_id: args.provider_id,
                        models,
                    },
                )
            }
            ProviderSubcommand::Capabilities(args) => {
                let model_ref =
                    registry.resolve_model_target(args.target.as_str(), args.model.as_deref())?;
                let capabilities = registry.model_capabilities(&model_ref)?;
                let metadata = registry.model_metadata(&model_ref)?;
                render_serialized(
                    args.format,
                    &ProviderCapabilitiesOutput {
                        provider_id: model_ref.provider_id.to_string(),
                        model: model_ref.model_id.to_string(),
                        model_ref: model_ref.to_string(),
                        capabilities,
                        metadata,
                    },
                )
            }
        }
    }

    pub fn load_request(&self) -> LoadConfigRequest {
        LoadConfigRequest {
            config_path: self.config.clone(),
            mode: self.mode.clone(),
            overrides: self.overrides.clone(),
        }
    }
}

fn render_serialized<T>(format: ConfigOutputFormat, value: &T) -> Result<String, AppError>
where
    T: Serialize,
{
    match format {
        ConfigOutputFormat::Json => Ok(serde_json::to_string_pretty(value)?),
        ConfigOutputFormat::Toml => toml::to_string_pretty(value)
            .map_err(|err| AppError::Config(format!("failed to render toml output: {err}"))),
    }
}

fn format_apply_output(execution: &ApplyPatchExecution) -> String {
    let mut output = format!("applied patch: {}", execution.operation_id);
    for file in &execution.files {
        output.push_str(&format!("\n- {:?} {}", file.kind, file.path));
    }
    output
}

fn format_debug_session_output(output: &DebugSessionOutput) -> String {
    let mut rendered = format!(
        "session {}: {}\nstatus: {:?}\nmessages: {}",
        output.session.id,
        output.session.title,
        output.session.status,
        output.messages.len()
    );
    for message in &output.messages {
        rendered.push_str(&format!(
            "\n\n[{} #{} {}]\n{}",
            message.role, message.id, message.state, message.text
        ));
    }
    rendered
}

fn review_prompt(base: &str) -> String {
    format!(
        "Review the current workspace changes against `{base}`. Focus on correctness, regressions, security issues, and missing tests. Report findings first, then concise remediation guidance."
    )
}

fn auth_summary(provider_id: String, auth: AuthData) -> AuthSummary {
    match auth {
        AuthData::Api { .. } => AuthSummary {
            provider_id,
            kind: "api_key".to_owned(),
            account_id: None,
            enterprise_url: None,
            expires_at_ms: None,
        },
        AuthData::OAuth {
            expires_at_ms,
            account_id,
            enterprise_url,
            ..
        } => AuthSummary {
            provider_id,
            kind: "oauth".to_owned(),
            account_id,
            enterprise_url,
            expires_at_ms: Some(expires_at_ms),
        },
        AuthData::WellKnown { .. } => AuthSummary {
            provider_id,
            kind: "well_known".to_owned(),
            account_id: None,
            enterprise_url: None,
            expires_at_ms: None,
        },
    }
}

fn normalize_login_provider(provider_id: &str) -> String {
    provider_id.trim_end_matches('/').to_owned()
}

async fn selected_session_id(
    manager: &crate::session::SessionManager,
    session_id: Option<i64>,
    last: bool,
) -> Result<i64, AppError> {
    if session_id.is_some() && last {
        return Err(AppError::Config(
            "pass either a session id or --last, not both".to_owned(),
        ));
    }
    if let Some(session_id) = session_id {
        return Ok(session_id);
    }
    let sessions = manager
        .list_session_summaries(SessionListRequest {
            offset: 0,
            limit: Some(1),
        })
        .await?;
    sessions
        .first()
        .map(|session| session.id)
        .ok_or_else(|| AppError::Config("no sessions found".to_owned()))
}

async fn latest_event_seq(
    manager: &crate::session::SessionManager,
    session_id: i64,
) -> Result<Option<i64>, AppError> {
    Ok(manager
        .list_session_events(session_id)
        .await?
        .last()
        .map(|event| event.meta.seq_global))
}

fn session_detail(session: &Session, latest_event_seq: Option<i64>) -> SessionDetail {
    SessionDetail {
        id: session.id,
        parent_id: session.parent_id,
        workspace_id: session.workspace_id,
        title: session.title.clone(),
        version: session.version,
        created_at: session.created_at,
        updated_at: session.updated_at,
        message_count: session.messages.len(),
        status: session.runtime.turn.status,
        latest_event_seq,
    }
}

fn resolve_continue_options(
    runtime: &AgenaRuntime,
    session: &Session,
    args: &ContinueArgs,
) -> Result<SessionRunOptions, AppError> {
    let snapshot = runtime.current_snapshot();
    let model = if let Some(model) = args.model.as_deref() {
        snapshot.resolve_model_target(model, None)?
    } else if let (Some(provider_id), Some(model_id)) = (
        session.runtime.turn.model_provider_id.as_deref(),
        session.runtime.turn.model_id.as_deref(),
    ) {
        ModelRef::try_new(provider_id, model_id)
            .map_err(|err| AppError::Config(format!("invalid persisted model reference: {err}")))?
    } else {
        default_model(runtime)?
    };

    Ok(SessionRunOptions {
        model,
        system: None,
        temperature: args.temperature,
        max_output_tokens: args.max_output_tokens,
    })
}

fn resolve_run_options(
    runtime: &AgenaRuntime,
    model: Option<&str>,
    temperature: Option<f32>,
    max_output_tokens: Option<u32>,
) -> Result<SessionRunOptions, AppError> {
    let model = if let Some(model) = model {
        runtime
            .current_snapshot()
            .resolve_model_target(model, None)?
    } else {
        default_model(runtime)?
    };

    Ok(SessionRunOptions {
        model,
        system: None,
        temperature,
        max_output_tokens,
    })
}

fn default_model(runtime: &AgenaRuntime) -> Result<ModelRef, AppError> {
    let snapshot = runtime.current_snapshot();
    let registry = snapshot.provider_registry();
    let mut providers = registry.provider_ids();
    providers.sort();
    let provider_id = providers
        .first()
        .ok_or_else(|| AppError::Config("no providers configured".to_owned()))?;
    registry.resolve_model_target(provider_id, None)
}

fn last_assistant_text(session: &Session) -> Option<String> {
    session
        .messages
        .iter()
        .rev()
        .find(|message| message.role == Role::Assistant)
        .map(|message| message.as_text_lossy())
}

fn title_from_prompt(prompt: &str) -> String {
    let title = prompt.trim().replace('\n', " ");
    let mut chars = title.chars();
    let truncated = chars.by_ref().take(80).collect::<String>();
    if truncated.is_empty() {
        "exec".to_owned()
    } else if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

fn session_storage_error() -> AppError {
    AppError::Config("session storage is unavailable; configure a database URL or path".to_owned())
}

async fn poll_until<T, F, Fut>(
    timeout: Duration,
    interval: Duration,
    mut poll: F,
) -> Result<Option<T>, AppError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Option<T>, AppError>>,
{
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = poll().await? {
            return Ok(Some(value));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        tokio::time::sleep(interval).await;
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use serde_json::Value;

    use super::*;
    use crate::{config::ConfigEnvironment, provider::CapabilitySupport};

    #[derive(Debug, Clone, Default)]
    struct TestEnvironment {
        vars: BTreeMap<String, String>,
    }

    impl ConfigEnvironment for TestEnvironment {
        fn var(&self, key: &str) -> Option<String> {
            self.vars.get(key).cloned()
        }

        fn vars(&self) -> Vec<(String, String)> {
            self.vars
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        }
    }

    #[tokio::test]
    async fn login_api_key_then_auth_list_redacts_secret() {
        let auth_path = std::env::temp_dir().join(format!(
            "agena-cli-auth-{}.json",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time should move forward")
                .as_nanos()
        ));
        let path = write_temp_config(
            format!(
                r#"
[auth]
store_backend = "file"
store_path = "{}"
"#,
                auth_path.display()
            )
            .as_str(),
        );
        let cli = AgenaCli {
            config: Some(path.clone()),
            mode: None,
            overrides: Vec::new(),
            database_url: None,
            database_path: None,
            command: None,
        };

        cli.clone()
            .run_login(
                ConfigLoader::new(ProcessEnvironment),
                LoginArgs {
                    provider_id: "openai".to_owned(),
                    api_key: Some("sk-test".to_owned()),
                    browser: false,
                    device: false,
                    port: 1455,
                    timeout_secs: 1,
                    instance_url: "https://gitlab.com".to_owned(),
                    enterprise_domain: None,
                },
            )
            .await
            .expect("login should write credential");

        let output = cli
            .render_auth_command(
                &ConfigLoader::new(TestEnvironment::default()),
                AuthCommand {
                    command: Some(AuthSubcommand::List(AuthListArgs {
                        format: ConfigOutputFormat::Json,
                    })),
                },
            )
            .await
            .expect("auth list should render");
        let value: Value = serde_json::from_str(output.as_str()).expect("output should be json");

        assert_eq!(value["credentials"][0]["provider_id"], "openai");
        assert_eq!(value["credentials"][0]["kind"], "api_key");
        assert!(!output.contains("sk-test"));
    }

    #[tokio::test]
    async fn logout_removes_cli_credential() {
        let auth_path = std::env::temp_dir().join(format!(
            "agena-cli-auth-{}.json",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time should move forward")
                .as_nanos()
        ));
        let path = write_temp_config(
            format!(
                r#"
[auth]
store_backend = "file"
store_path = "{}"
"#,
                auth_path.display()
            )
            .as_str(),
        );
        let cli = AgenaCli {
            config: Some(path),
            mode: None,
            overrides: Vec::new(),
            database_url: None,
            database_path: None,
            command: None,
        };

        cli.clone()
            .run_login(
                ConfigLoader::new(ProcessEnvironment),
                LoginArgs {
                    provider_id: "openai".to_owned(),
                    api_key: Some("sk-test".to_owned()),
                    browser: false,
                    device: false,
                    port: 1455,
                    timeout_secs: 1,
                    instance_url: "https://gitlab.com".to_owned(),
                    enterprise_domain: None,
                },
            )
            .await
            .expect("login should write credential");
        cli.clone()
            .run_logout(
                ConfigLoader::new(ProcessEnvironment),
                LogoutArgs {
                    provider_id: "openai".to_owned(),
                },
            )
            .expect("logout should remove credential");

        let output = cli
            .render_auth_command(
                &ConfigLoader::new(TestEnvironment::default()),
                AuthCommand {
                    command: Some(AuthSubcommand::List(AuthListArgs {
                        format: ConfigOutputFormat::Json,
                    })),
                },
            )
            .await
            .expect("auth list should render");
        let value: Value = serde_json::from_str(output.as_str()).expect("output should be json");
        assert!(value["credentials"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn sessions_list_and_resume_last_render_session() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!("agena-cli-session-{suffix}.db"));
        let cli = AgenaCli {
            config: None,
            mode: None,
            overrides: Vec::new(),
            database_url: Some(format!("sqlite://{}?mode=rwc", db_path.display())),
            database_path: None,
            command: None,
        };
        let runtime = cli.session_runtime().await.expect("runtime should build");
        let manager = runtime
            .session_manager()
            .expect("session manager should be available");
        let created = manager
            .create_session(crate::session::SessionCreateRequest {
                title: "cli session".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("session should be created");

        let list_output = cli
            .render_sessions_command(SessionsCommand {
                command: Some(SessionsSubcommand::List(SessionListArgs {
                    limit: 10,
                    offset: 0,
                    format: ConfigOutputFormat::Json,
                })),
            })
            .await
            .expect("sessions list should render");
        let list: Value = serde_json::from_str(list_output.as_str()).expect("list should be json");
        assert_eq!(list["sessions"][0]["id"], created.id);

        let resume_output = cli
            .render_resume_command(ResumeArgs {
                session_id: None,
                last: true,
                format: ConfigOutputFormat::Json,
            })
            .await
            .expect("resume should render");
        let resumed: Value =
            serde_json::from_str(resume_output.as_str()).expect("resume should be json");
        assert_eq!(resumed["session"]["id"], created.id);
        assert_eq!(resumed["session"]["title"], "cli session");
    }

    #[test]
    fn exec_command_parses_json_and_workspace_alias() {
        let cli = AgenaCli::parse_from([
            "agena",
            "exec",
            "--cwd",
            ".",
            "--model",
            "openai/gpt-5",
            "--json",
            "summarize",
        ]);

        let Some(AgenaCommand::Exec(args)) = cli.command else {
            panic!("expected exec command");
        };
        assert_eq!(args.workspace, Some(PathBuf::from(".")));
        assert_eq!(args.model.as_deref(), Some("openai/gpt-5"));
        assert!(args.json);
        assert_eq!(args.prompt, "summarize");
    }

    #[test]
    fn exec_helpers_render_last_assistant_text_and_title() {
        let mut session = Session::new(1, 1, "test", Utc::now());
        session
            .messages
            .push(crate::message::Message::prompt_text(Role::User, "hello"));
        session.messages.push(crate::message::Message::prompt_text(
            Role::Assistant,
            "first",
        ));
        session.messages.push(crate::message::Message::prompt_text(
            Role::Assistant,
            "second",
        ));

        assert_eq!(last_assistant_text(&session).as_deref(), Some("second"));
        assert_eq!(title_from_prompt("  hello\nworld  "), "hello world");
    }

    #[test]
    fn developer_workflow_commands_parse() {
        let apply = AgenaCli::parse_from([
            "agena",
            "apply",
            "--workspace",
            ".",
            "--json",
            "change.patch",
        ]);
        let Some(AgenaCommand::Apply(args)) = apply.command else {
            panic!("expected apply command");
        };
        assert_eq!(args.workspace, Some(PathBuf::from(".")));
        assert_eq!(args.patch_file, PathBuf::from("change.patch"));
        assert!(args.json);

        let review = AgenaCli::parse_from(["agena", "review", "--base", "develop"]);
        let Some(AgenaCommand::Review(args)) = review.command else {
            panic!("expected review command");
        };
        assert_eq!(args.base, "develop");

        let debug = AgenaCli::parse_from(["agena", "debug", "session", "42", "--json"]);
        let Some(AgenaCommand::Debug(command)) = debug.command else {
            panic!("expected debug command");
        };
        let DebugSubcommand::Session(args) = command.command;
        assert_eq!(args.session_id, 42);
        assert!(args.json);
    }

    #[test]
    fn apply_command_invokes_apply_patch_builtin() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        let workspace = std::env::temp_dir().join(format!("agena-cli-apply-{suffix}"));
        fs::create_dir_all(&workspace).expect("workspace should be created");
        let patch_file = workspace.join("change.patch");
        fs::write(
            &patch_file,
            "*** Begin Patch\n*** Add File: added.txt\n+created\n*** End Patch",
        )
        .expect("patch file should be written");
        let cli = AgenaCli {
            config: None,
            mode: None,
            overrides: Vec::new(),
            database_url: None,
            database_path: None,
            command: None,
        };

        let output = cli
            .render_apply_command(ApplyArgs {
                workspace: Some(workspace.clone()),
                json: true,
                patch_file,
            })
            .expect("apply should succeed");
        let value: Value = serde_json::from_str(output.as_str()).expect("output should be json");

        assert_eq!(value["patch"]["files"][0]["path"], "added.txt");
        assert_eq!(
            fs::read_to_string(workspace.join("added.txt")).expect("file should be created"),
            "created\n"
        );
    }

    #[test]
    fn debug_session_plain_output_includes_messages() {
        let output = DebugSessionOutput {
            session: SessionDetail {
                id: 7,
                parent_id: None,
                workspace_id: 1,
                title: "debug me".to_owned(),
                version: 1,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                message_count: 1,
                status: SessionRuntimeStatus::Idle,
                latest_event_seq: Some(9),
            },
            messages: vec![DebugMessageOutput {
                id: 3,
                role: Role::Assistant,
                state: crate::message::MessageStatus::Completed,
                text: "done".to_owned(),
            }],
        };

        let rendered = format_debug_session_output(&output);
        assert!(rendered.contains("session 7: debug me"));
        assert!(rendered.contains("[assistant #3 completed]"));
        assert!(rendered.contains("done"));
    }

    #[tokio::test]
    async fn provider_capabilities_command_renders_resolved_alias_capabilities() {
        let path = write_temp_config(
            r#"
[providers.openai]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1-mini"
api_key_env = "OPENAI_API_KEY"

[providers.prod]
kind = "alias"
target_provider_id = "openai"
default_model = "gpt-5"

[[providers.prod.capability_overrides]]
model = "gpt-5"
image_input = "unsupported"
"#,
        );
        let env = TestEnvironment {
            vars: BTreeMap::from([("OPENAI_API_KEY".to_owned(), "sk-test".to_owned())]),
        };
        let loader = ConfigLoader::new(env);
        let cli = AgenaCli {
            config: Some(path),
            mode: None,
            overrides: Vec::new(),
            database_url: None,
            database_path: None,
            command: Some(AgenaCommand::Provider(ProviderCommand {
                command: Some(ProviderSubcommand::Capabilities(ProviderCapabilitiesArgs {
                    target: "prod".to_owned(),
                    model: None,
                    format: ConfigOutputFormat::Json,
                })),
            })),
        };

        let output = cli
            .render_provider_command(
                &loader,
                ProviderCommand {
                    command: Some(ProviderSubcommand::Capabilities(ProviderCapabilitiesArgs {
                        target: "prod/gpt-5".to_owned(),
                        model: None,
                        format: ConfigOutputFormat::Json,
                    })),
                },
            )
            .await
            .expect("provider capabilities command should succeed");
        let value: Value = serde_json::from_str(output.as_str()).expect("output should be json");

        assert_eq!(value["provider_id"], "prod");
        assert_eq!(value["model"], "gpt-5");
        assert_eq!(value["model_ref"], "prod/gpt-5");
        assert_eq!(value["capabilities"]["image_input"], "unsupported");
        assert_eq!(value["capabilities"]["document_input"], "supported");
        assert_eq!(value["metadata"]["family"], "gpt");
    }

    #[tokio::test]
    async fn provider_models_command_renders_static_gitlab_models() {
        let path = write_temp_config(
            r#"
[providers.gitlab]
kind = "gitlab"
api_key = "glpat-test"
default_model = "claude-sonnet-4-5"
"#,
        );
        let loader = ConfigLoader::new(TestEnvironment {
            vars: BTreeMap::from([("OPENAI_API_KEY".to_owned(), "sk-test".to_owned())]),
        });
        let cli = AgenaCli {
            config: Some(path),
            mode: None,
            overrides: Vec::new(),
            database_url: None,
            database_path: None,
            command: None,
        };

        let output = cli
            .render_provider_command(
                &loader,
                ProviderCommand {
                    command: Some(ProviderSubcommand::Models(ProviderModelsArgs {
                        provider_id: "gitlab".to_owned(),
                        format: ConfigOutputFormat::Json,
                    })),
                },
            )
            .await
            .expect("provider models command should succeed");
        let value: Value = serde_json::from_str(output.as_str()).expect("output should be json");

        assert_eq!(value["provider_id"], "gitlab");
        assert_eq!(value["models"][0]["id"], "claude-sonnet-4-5");
        assert_eq!(value["models"][0]["metadata"]["family"], "claude");
        assert_eq!(
            value["models"][0]["capabilities"]["tool_calling"],
            "supported"
        );
    }

    #[tokio::test]
    async fn provider_list_command_includes_alias_default_models() {
        let path = write_temp_config(
            r#"
[providers.openai]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1-mini"
api_key_env = "OPENAI_API_KEY"

[providers.prod]
kind = "alias"
target_provider_id = "openai"
default_model = "gpt-5"
"#,
        );
        let env = TestEnvironment {
            vars: BTreeMap::from([("OPENAI_API_KEY".to_owned(), "sk-test".to_owned())]),
        };
        let loader = ConfigLoader::new(env);
        let cli = AgenaCli {
            config: Some(path),
            mode: None,
            overrides: Vec::new(),
            database_url: None,
            database_path: None,
            command: None,
        };

        let output = cli
            .render_provider_command(
                &loader,
                ProviderCommand {
                    command: Some(ProviderSubcommand::List(ProviderListArgs {
                        format: ConfigOutputFormat::Json,
                    })),
                },
            )
            .await
            .expect("provider list command should succeed");
        let value: Value = serde_json::from_str(output.as_str()).expect("output should be json");
        let providers = value["providers"]
            .as_array()
            .expect("providers should be an array");

        assert!(providers.iter().any(|item| {
            item["provider_id"] == "openai"
                && item["default_model"] == "gpt-4.1-mini"
                && item["default_model_ref"] == "openai/gpt-4.1-mini"
        }));
        assert!(providers.iter().any(|item| {
            item["provider_id"] == "prod"
                && item["default_model"] == "gpt-5"
                && item["default_model_ref"] == "prod/gpt-5"
        }));
    }

    #[test]
    fn capability_support_json_serialization_uses_snake_case_strings() {
        let encoded =
            serde_json::to_string(&CapabilitySupport::Unsupported).expect("encoding should work");
        assert_eq!(encoded, "\"unsupported\"");
    }

    fn write_temp_config(content: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("agena-cli-{suffix}.toml"));
        fs::write(&path, content).expect("temp config should be written");
        path
    }
}
