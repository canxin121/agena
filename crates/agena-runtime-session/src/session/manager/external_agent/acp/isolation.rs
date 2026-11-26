use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use agena_domain::{ExternalAgent, NetworkAction, NetworkPermissionConfig};
use agena_process::sandbox::{self, SandboxNetwork, SandboxPolicy};
use serde::Deserialize;
use tokio::process::Command;

use super::{AppError, policy};

const CONFIG_ENV: &str = "AGENA_ACP_SANDBOX_FILE";

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct Config {
    antigravity: Option<AgentConfig>,
    claude_code: Option<AgentConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentConfig {
    network: NetworkPermissionConfig,
    #[serde(default)]
    runtime_paths: Vec<PathBuf>,
}

pub(super) struct Isolation {
    _relay: tempfile::TempDir,
    socket: PathBuf,
}

impl Isolation {
    pub(super) fn relay_socket(&self) -> &Path {
        &self.socket
    }
}

fn configured(agent: ExternalAgent) -> Result<Option<AgentConfig>, AppError> {
    let Some(path) = std::env::var_os(CONFIG_ENV) else {
        return Ok(None);
    };
    let file = fs::File::open(path)
        .map_err(|error| AppError::Config(format!("Could not open {CONFIG_ENV}: {error}")))?;
    let config: Config = serde_json::from_reader(file.take(1024 * 1024))
        .map_err(|error| AppError::Config(format!("Invalid {CONFIG_ENV}: {error}")))?;
    Ok(match agent {
        ExternalAgent::Antigravity => config.antigravity,
        ExternalAgent::ClaudeCode => config.claude_code,
    })
}

pub(super) fn wrap(
    command: &mut Command,
    agent: ExternalAgent,
    cwd: &Path,
    additional_directories: &[PathBuf],
    previous_history: bool,
    private_home: Option<&Path>,
) -> Result<Option<Isolation>, AppError> {
    let Some(config) = configured(agent)? else {
        return Ok(None);
    };
    if !cfg!(target_os = "linux") || !sandbox::enabled() {
        return Err(AppError::Config(
            "ACP process isolation needs an enabled Linux MXC sandbox.".into(),
        ));
    }
    let network = config
        .network
        .simplified_for_sandbox()
        .map_err(AppError::Config)?;
    network
        .validate_for_platform(std::env::consts::OS)
        .map_err(AppError::Config)?;
    if network.egress.as_ref().and_then(|egress| egress.default) == Some(NetworkAction::Allow) {
        return Err(AppError::Config(
            "ACP process isolation requires default-deny egress.".into(),
        ));
    }
    let mut writes = Vec::new();
    match agent {
        ExternalAgent::Antigravity => {
            let root = policy::antigravity_profile()?;
            writes.push(root);
            if previous_history {
                writes.push(policy::previous_antigravity_store()?);
            }
        }
        ExternalAgent::ClaudeCode => {
            let config = std::env::var_os("CLAUDE_CONFIG_DIR")
                .map(PathBuf::from)
                .or_else(|| policy::native_home().map(|home| home.join(".claude")))
                .filter(|path| path.is_absolute() && path.parent().is_some())
                .ok_or_else(|| {
                    AppError::Config(
                        "Claude Code needs an absolute native configuration directory.".into(),
                    )
                })?;
            command.env("CLAUDE_CONFIG_DIR", &config);
            writes.push(config);
        }
    }
    let mut reads = vec![cwd.to_path_buf()];
    reads.extend_from_slice(additional_directories);
    let roots = reads
        .iter()
        .map(fs::canonicalize)
        .collect::<Result<Vec<_>, _>>()?;
    for path in &writes {
        let expected = agena_runtime_tools::canonicalize_mutation_path(path);
        if roots
            .iter()
            .any(|root| expected.starts_with(root) || root.starts_with(&expected))
        {
            return Err(AppError::Config(
                "The external agent's writable native store overlaps a read-only workspace.".into(),
            ));
        }
        fs::create_dir_all(path)?;
    }
    let relay = tempfile::Builder::new()
        .prefix("agena-sandbox-acp-relay-")
        .tempdir()?;
    reads.push(relay.path().to_path_buf());
    let mut private_paths = vec![relay.path().to_path_buf()];
    if let Some(home) = private_home {
        reads.push(home.to_owned());
        private_paths.push(home.to_owned());
    }
    let socket = relay.path().join("mcp.sock");
    let policy = SandboxPolicy {
        readonly_paths: reads,
        readwrite_paths: writes,
        runtime_paths: config.runtime_paths,
        private_paths,
        workload_home: private_home.map(Path::to_path_buf),
        network: SandboxNetwork::Native { policy: network },
        ..SandboxPolicy::default()
    };
    sandbox::wrap_tokio_command(command, &policy)?;
    Ok(Some(Isolation {
        _relay: relay,
        socket,
    }))
}
