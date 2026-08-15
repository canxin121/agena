//! One-shot command dispatch.
//!
//! Center-backed commands and explicitly unavailable compatibility commands
//! skip Runtime bootstrap preflight. Pure local presentation/marketplace
//! commands may still read local configuration, but the CLI crate has no
//! Runtime bootstrap or execution-lease path.

use crate::error::AgenaProcessError;
use agena_cli::{AgenaCli, AgenaCommand, PluginSubcommand};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub(crate) async fn run_command(mut cli: AgenaCli) -> Result<(), AgenaProcessError> {
    let runtime_free_command = matches!(
        &cli.command,
        Some(
            AgenaCommand::Exec(_)
                | AgenaCommand::Continue(_)
                | AgenaCommand::Resume(_)
                | AgenaCommand::Fork(_)
                | AgenaCommand::Sessions(_)
                | AgenaCommand::Cost(_)
                | AgenaCommand::Usage(_)
                | AgenaCommand::Permissions(_)
                | AgenaCommand::Provider(_)
                | AgenaCommand::Review(_)
                | AgenaCommand::Debug(_)
                | AgenaCommand::Auth(_)
                | AgenaCommand::Login(_)
                | AgenaCommand::Logout(_)
                | AgenaCommand::Git(_)
                | AgenaCommand::Snapshot(_)
                | AgenaCommand::Commit(_)
                | AgenaCommand::Pr(_)
                | AgenaCommand::Memory(_)
                | AgenaCommand::Mcp(_)
                | AgenaCommand::Config(_)
                | AgenaCommand::Diagnostics(_)
                | AgenaCommand::Apply(_)
                | AgenaCommand::McpServer(_)
        )
    ) || matches!(
        &cli.command,
        Some(AgenaCommand::Plugin(command))
            if matches!(
                &command.command,
                PluginSubcommand::Status(_)
                    | PluginSubcommand::Inspect(_)
                    | PluginSubcommand::Logs(_)
            )
    );
    let tracing = if runtime_free_command {
        // Thin clients and explicitly unavailable compatibility commands do
        // not bootstrap or preflight Runtime config.
        agena_runtime::RuntimeTracingConfiguration::default()
    } else {
        agena_runtime::resolve_runtime_bootstrap_preflight(
            &agena_runtime::RuntimeBootstrapRequest {
                config_override_expressions: cli.overrides.clone(),
                ..Default::default()
            },
        )
        .ok()
        .map(|preflight| preflight.tracing)
        .unwrap_or_default()
    };
    cli.center = Some(super::super::center_client::resolve_center_url(
        cli.center.take(),
    ));

    let initial_filter = agena_runtime::runtime_env_filter(&tracing).unwrap_or_else(|_| {
        agena_runtime::runtime_env_filter(&agena_runtime::RuntimeTracingConfiguration::default())
            .expect("default tracing filter should parse")
    });
    tracing_subscriber::registry()
        .with(initial_filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .compact()
                .with_writer(std::io::stderr),
        )
        .init();

    cli.run_command()
        .await
        .map_err(|error| AgenaProcessError::Internal(error.to_string()))
}
