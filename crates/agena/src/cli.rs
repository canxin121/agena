use std::{
    io::{self, Write},
    path::PathBuf,
    time::{Duration, Instant},
};

use clap::{Args, Parser, Subcommand};
use serde::Serialize;

use crate::{
    config::{
        ConfigEnvironment, ConfigLoader, ConfigModeName, ConfigOutputFormat, ConfigOverride,
        LoadConfigRequest, ProcessEnvironment,
    },
    error::AppError,
    provider::{
        ModelCapabilities, ModelMetadata, ProviderModel,
        auth::{
            AuthData, AuthManager, ConfiguredAuthStore, CopilotDeployment, wait_for_oauth_callback,
        },
    },
    runtime::{AgenaRuntime, TracingFilterReloadHandle},
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
    #[command(subcommand)]
    pub command: Option<AgenaCommand>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum AgenaCommand {
    Auth(AuthCommand),
    Config(ConfigCommand),
    Login(LoginArgs),
    Logout(LogoutArgs),
    Provider(ProviderCommand),
    Serve(ServeCommand),
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
pub struct ProviderCommand {
    #[command(subcommand)]
    pub command: Option<ProviderSubcommand>,
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
pub enum ProviderSubcommand {
    List(ProviderListArgs),
    Models(ProviderModelsArgs),
    Capabilities(ProviderCapabilitiesArgs),
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
            Some(AgenaCommand::Auth(command)) => self.run_auth(loader, command).await,
            Some(AgenaCommand::Config(command)) => self.run_config(loader, command),
            Some(AgenaCommand::Login(args)) => self.run_login(loader, args).await,
            Some(AgenaCommand::Logout(args)) => self.run_logout(loader, args),
            Some(AgenaCommand::Provider(command)) => self.run_provider(loader, command).await,
            Some(AgenaCommand::Serve(_command)) => Err(AppError::Config(
                "the HTTP server moved to the `apps/agena-http-api-server` app; run `cargo run -p agena-http-api-server -- --help` from the repository root".to_owned(),
            )),
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
