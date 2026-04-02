use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::{
    config::{
        ConfigLoader, ConfigModeName, ConfigOutputFormat, ConfigOverride, LoadConfigRequest,
        ProcessEnvironment,
    },
    error::AppError,
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
    Config(ConfigCommand),
}

#[derive(Debug, Clone, Args)]
pub struct ConfigCommand {
    #[command(subcommand)]
    pub command: Option<ConfigSubcommand>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ConfigSubcommand {
    Resolve(ConfigResolveArgs),
    Validate,
    Mode(ConfigModeArgs),
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

impl AgenaCli {
    pub async fn run(
        self,
        tracing_reload_handle: Option<TracingFilterReloadHandle>,
    ) -> Result<(), AppError> {
        let loader = ConfigLoader::new(ProcessEnvironment);

        match self.command.clone() {
            Some(AgenaCommand::Config(command)) => self.run_config(loader, command),
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
            sessions = snapshot.session_service().is_some(),
            "Agena started with resolved configuration"
        );
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

    pub fn load_request(&self) -> LoadConfigRequest {
        LoadConfigRequest {
            config_path: self.config.clone(),
            mode: self.mode.clone(),
            overrides: self.overrides.clone(),
        }
    }
}
