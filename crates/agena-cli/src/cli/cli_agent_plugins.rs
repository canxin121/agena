//! CLI arguments and output for the shared agent plugin installer.

use std::collections::BTreeMap;

use agena_application::agent_packages::{AgentInstallOutcome, AgentInstallRequest};
use agena_runtime::{ProcessEnvironment, default_config_path};

use super::{AppError, PluginInstallArgs};

pub(super) use agena_application::agent_packages::{is_agent_source, list, uninstall};

pub(super) fn reject_agent_options(args: &PluginInstallArgs) -> Result<(), AppError> {
    if args.id.is_some()
        || args.native_home.is_some()
        || args.profile.is_some()
        || args.installed
        || args.enable_hooks
        || !args.environment.is_empty()
        || !args.option_env.is_empty()
        || args.accept_command.is_some()
        || args.package_source.is_some()
    {
        return Err(config_error(
            "native install options require a codex:, claude:, deepseek:, or local directory source",
        ));
    }
    Ok(())
}

pub(super) async fn install(args: PluginInstallArgs) -> Result<(), AppError> {
    if args.require_signature || args.allow_unverified || args.registry_id != "default" {
        return Err(config_error(
            "signature and registry ID options apply to the Agena marketplace",
        ));
    }
    tokio::task::spawn_blocking(move || {
        let config_path = default_config_path(&ProcessEnvironment);
        let cwd = std::env::current_dir().map_err(|error| AppError::config_error(&error))?;
        let request = AgentInstallRequest {
            spec: args.spec,
            registry: args.registry,
            package_source: args.package_source,
            force: args.force,
            dry_run: args.dry_run,
            refresh: args.refresh,
            id: args.id,
            native_home: args.native_home,
            profile: args.profile,
            installed: args.installed,
            enable_hooks: args.enable_hooks,
            environment: args.environment,
            option_env: option_environment(&args.option_env)?,
            accept_command: args.accept_command,
        };
        let outcome = agena_application::agent_packages::install(&config_path, &cwd, request)
            .map_err(|error| AppError::config_error(&error))?;
        match outcome {
            AgentInstallOutcome::Preview(preview) => {
                println!("{}", preview.summary());
                for message in preview.diagnostics {
                    eprintln!("{}: {message}", preview.id);
                }
            }
            AgentInstallOutcome::Package {
                id,
                format,
                path,
                diagnostics,
                ..
            } => {
                println!(
                    "Installed {id} ({}) from {}",
                    format.as_str(),
                    path.display()
                );
                println!("Agena configuration: {}", config_path.display());
                for message in diagnostics {
                    eprintln!("{id}: {message}");
                }
            }
        }
        Ok(())
    })
    .await
    .map_err(|error| AppError::internal_error(&error))?
}

fn option_environment(entries: &[String]) -> Result<BTreeMap<String, String>, AppError> {
    entries
        .iter()
        .map(|entry| {
            entry
                .split_once('=')
                .filter(|(key, value)| !key.is_empty() && !value.is_empty())
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .ok_or_else(|| config_error("--option-env uses KEY=NAME"))
        })
        .collect()
}

fn config_error(error: impl Into<String>) -> AppError {
    AppError::Config(error.into())
}
