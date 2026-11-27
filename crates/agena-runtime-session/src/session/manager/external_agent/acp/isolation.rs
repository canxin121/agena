use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use agena_domain::{ExternalAgent, NetworkAction, NetworkPermissionConfig, ResolvedExternalAgent};
use agena_process::sandbox::{self, SandboxNetwork, SandboxPolicy};
use serde::Deserialize;
use tokio::process::Command;

use super::{AppError, policy};

const CONFIG_ENV: &str = "AGENA_ACP_SANDBOX_FILE";

type Config = BTreeMap<ExternalAgent, AgentConfig>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentConfig {
    network: NetworkPermissionConfig,
    #[serde(default)]
    runtime_paths: Vec<PathBuf>,
}

pub(super) struct Isolation {
    #[cfg(not(windows))]
    _relay: tempfile::TempDir,
    socket: PathBuf,
}

impl Isolation {
    pub(super) fn relay_socket(&self) -> &Path {
        &self.socket
    }
}

fn configured(agent: &ExternalAgent) -> Result<Option<AgentConfig>, AppError> {
    let Some(path) = std::env::var_os(CONFIG_ENV).filter(|path| !path.is_empty()) else {
        return Ok(None);
    };
    let file = fs::File::open(path)
        .map_err(|error| AppError::Config(format!("Could not open {CONFIG_ENV}: {error}")))?;
    let mut config: Config = serde_json::from_reader(file.take(1024 * 1024))
        .map_err(|error| AppError::Config(format!("Invalid {CONFIG_ENV}: {error}")))?;
    Ok(config.remove(agent))
}

pub(super) fn wrap(
    command: &mut Command,
    spec: &ResolvedExternalAgent,
    cwd: &Path,
    additional_directories: &[PathBuf],
) -> Result<Option<Isolation>, AppError> {
    let Some(config) = configured(&spec.id)? else {
        return Ok(None);
    };
    if !sandbox::enabled() {
        return Err(AppError::Config(
            "ACP process isolation needs an enabled MXC sandbox.".into(),
        ));
    }
    let network = config
        .network
        .simplified_for_sandbox()
        .map_err(AppError::Config)?;
    network
        .validate_for_platform(std::env::consts::OS)
        .map_err(AppError::Config)?;
    if !cfg!(target_os = "macos") {
        if network.egress.as_ref().and_then(|egress| egress.default) == Some(NetworkAction::Allow) {
            return Err(AppError::Config(
                "ACP process isolation requires default-deny egress.".into(),
            ));
        }
        if network
            .ingress
            .as_ref()
            .and_then(|ingress| ingress.host_loopback)
            == Some(NetworkAction::Allow)
        {
            return Err(AppError::Config(
                "ACP process isolation forbids host loopback access.".into(),
            ));
        }
        let allows_any = network
            .egress
            .as_ref()
            .and_then(|egress| egress.allow.as_ref())
            .into_iter()
            .flatten()
            .flat_map(|rule| rule.to.iter().flatten())
            .any(|peer| {
                matches!(peer.cidr.trim(), "0.0.0.0/0" | "::/0") && peer.except.is_empty()
            });
        if allows_any {
            return Err(AppError::Config(
                "ACP process isolation requires scoped egress targets rather than unrestricted wildcard CIDRs."
                    .into(),
            ));
        }
    }
    let mut writes = Vec::new();
    match &spec.id {
        ExternalAgent::Antigravity => {
            writes.push(policy::antigravity_store()?);
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
        ExternalAgent::Custom(_) => {}
    }
    for path in &spec.writable_paths {
        if !writes.contains(path) {
            writes.push(path.clone());
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
    let mut private_paths = Vec::new();
    #[cfg(not(windows))]
    let (relay, socket) = {
        let relay = tempfile::Builder::new()
            .prefix("agena-sandbox-acp-relay-")
            .tempdir()?;
        reads.push(relay.path().to_path_buf());
        private_paths.push(relay.path().to_path_buf());
        let socket = relay.path().join("mcp.sock");
        (relay, socket)
    };
    #[cfg(windows)]
    let socket = PathBuf::from(format!(
        "{}{}",
        agena_mcp_server::stdio_relay::PIPE_PREFIX,
        uuid::Uuid::new_v4().simple()
    ));
    let mut denied_paths = Vec::new();
    if let Some(data_dir) = std::env::var_os("AGENA_DATA_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        denied_paths.push(data_dir);
    }
    let policy = SandboxPolicy {
        readonly_paths: reads,
        readwrite_paths: writes,
        denied_paths,
        runtime_paths: config.runtime_paths,
        private_paths,
        workload_home: None,
        network: SandboxNetwork::Native { policy: network },
        ..SandboxPolicy::default()
    };
    sandbox::wrap_tokio_command(command, &policy)?;
    Ok(Some(Isolation {
        #[cfg(not(windows))]
        _relay: relay,
        socket,
    }))
}
