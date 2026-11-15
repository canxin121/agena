//! MXC isolation for model-owned native processes. The existing permission
//! service approves effects; this module projects that exact scoped authority
//! into kernel-enforced filesystem and network boundaries.
use crate::{
    part::ShellCommandInput,
    tool::{ToolError, ToolExecutor},
};
use agena_domain::{NetworkTarget, PermissionDecision};
use agena_process::sandbox::{ProxyTarget, SandboxNetwork, SandboxPolicy};
use std::{
    collections::HashMap,
    net::ToSocketAddrs,
    path::{Path, PathBuf},
};

static POLICY_WORKERS: agena_async::BlockingPool = agena_async::BlockingPool::new(8);

/// Trusted process default; individual MCP invocations can also disable MXC.
pub fn enabled() -> Result<bool, ToolError> {
    Ok(agena_process::sandbox::enabled())
}

pub(crate) async fn protect_async(
    executor: &ToolExecutor,
    command: Vec<String>,
    cwd: &Path,
    input: &ShellCommandInput,
    env: HashMap<String, String>,
) -> Result<(Vec<String>, HashMap<String, String>), ToolError> {
    executor.ensure_not_cancelled()?;
    if !executor.sandbox_enabled() {
        return Ok((command, env));
    }
    let cancel = executor.cancellation_token().cloned();
    let deadline =
        std::time::Duration::from_millis(input.timeout_ms.unwrap_or(30_000).min(30_000).max(1));
    let executor = executor.clone();
    let input = input.clone();
    let cwd = cwd.to_path_buf();
    let preparation = POLICY_WORKERS.run(move || {
        let mut env = env;
        let command = protect(&executor, command, &cwd, &input, &mut env)?;
        Ok((command, env))
    });
    tokio::select! {
        biased;
        _ = async { match cancel { Some(token) => token.cancelled().await, None => std::future::pending::<()>().await } } => Err(ToolError::Cancelled),
        result = tokio::time::timeout(deadline, preparation) => {
            result.map_err(|_| ToolError::invalid_input("sandbox policy preparation or DNS lookup timed out"))?
                .map_err(|error| ToolError::plugin(format!("sandbox policy worker failed: {error}")))?
        }
    }
}

pub(crate) fn protect(
    executor: &ToolExecutor,
    command: Vec<String>,
    cwd: &Path,
    input: &ShellCommandInput,
    env: &mut HashMap<String, String>,
) -> Result<Vec<String>, ToolError> {
    let began = std::time::Instant::now();
    executor.ensure_not_cancelled()?;
    if !executor.sandbox_enabled() {
        return Ok(command);
    }
    let mut reads = input
        .reads
        .iter()
        .map(|path| executor.resolve_filesystem_effect_path(path, cwd))
        .collect::<Vec<_>>();
    let writes = input
        .writes
        .iter()
        .map(|path| executor.resolve_filesystem_effect_path(path, cwd))
        .collect::<Vec<_>>();
    // The shell's working directory is explicitly included in preflight.
    // Its descendants still retain all finer settings and ceiling rules.
    reads.push(crate::canonicalize_mutation_path(cwd));
    reads.extend(writes.iter().cloned());
    executor.authorize_sandbox_paths(&reads, &writes)?;
    let mut policy = policy_for_paths(executor, &reads, &writes, &input.network)?;
    for path in &writes {
        if !path.exists() {
            return Err(ToolError::invalid_input(format!(
                "MXC requires an existing writable mount: {}. Declare an existing output directory, or create the exact file with fs.write before launching the command.",
                path.display(),
            )));
        }
    }
    executor.ensure_not_cancelled()?;
    policy.timeout_ms = input.timeout_ms.map(|timeout| {
        timeout.saturating_sub(u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX))
    });
    if policy.timeout_ms == Some(0) {
        return Err(ToolError::invalid_input(
            "shell timeout elapsed while preparing sandbox policy",
        ));
    }
    agena_process::sandbox::wrap_argv(&command, cwd, env, &policy)
        .map_err(|error| ToolError::invalid_input(format!("start MXC sandbox: {error}")))
}

/// Paths are canonical, absolute effects already authorized by the caller.
/// A worker's private scratch directory may also be supplied as a write grant.
/// Runtime installations are resolved separately by agena-process; this never
/// adds the host home directory, credentials or package caches implicitly.
pub(crate) fn policy_for_paths(
    executor: &ToolExecutor,
    reads: &[PathBuf],
    writes: &[PathBuf],
    network: &[String],
) -> Result<SandboxPolicy, ToolError> {
    if !executor.sandbox_enabled() {
        return Ok(SandboxPolicy {
            disabled: true,
            readonly_paths: reads.to_vec(),
            readwrite_paths: writes.to_vec(),
            ..Default::default()
        });
    }
    let snapshot = executor.sandbox_permission_snapshot();
    for path in reads.iter().chain(writes) {
        if !path.is_absolute() || crate::canonicalize_mutation_path(path) != *path {
            return Err(ToolError::invalid_input(format!(
                "sandbox resource identity changed after authorization: {}",
                path.display(),
            )));
        }
    }
    let mut reads = reads.to_vec();
    let mut writes = writes.to_vec();
    reads.sort();
    reads.dedup();
    writes.sort();
    writes.dedup();
    // Validate original host resources before any staging copy replaces them
    // with private scratch grants. No invocation may read another worker's
    // payload or use a shared temporary root to reach other sessions.
    agena_process::sandbox::validate_resource_paths(&reads)
        .and_then(|()| agena_process::sandbox::validate_resource_paths(&writes))
        .map_err(|error| ToolError::invalid_input(format!("sandbox resource path: {error}")))?;
    let restrictions = snapshot
        .filesystem_restrictions(&reads, &writes)
        .map_err(|error| ToolError::invalid_input(format!("sandbox permission policy: {error}")))?;
    // Read/write mounts already include reading. A duplicate read-only mount
    // overrides writes on Seatbelt, so ordinary read grants covered by a write
    // grant must be omitted. Keep explicit policy read-only exceptions below.
    reads.retain(|read| !writes.iter().any(|write| read.starts_with(write)));
    reads.extend(restrictions.readonly_paths);
    reads.sort();
    reads.dedup();
    Ok(SandboxPolicy {
        readonly_paths: reads,
        readwrite_paths: writes,
        denied_paths: restrictions.denied_paths,
        network: proxy_network(&snapshot, network)?,
        ..Default::default()
    })
}

/// Expand the same ports that the HTTP broker will permit, before approval.
/// A broad hostname decision must not skip a more specific host:port rule.
pub(crate) fn declared_network_targets(declarations: &[String]) -> Result<Vec<String>, ToolError> {
    let mut result = Vec::new();
    for declaration in declarations {
        let target: NetworkTarget = declaration.parse().map_err(|error| {
            ToolError::invalid_input(format!("invalid sandbox network target: {error}"))
        })?;
        let host = target.host().trim_matches(['[', ']']);
        if host.contains(['*', '?', '/']) || host.chars().any(char::is_control) {
            return Err(ToolError::invalid_input(
                "sandbox network effects must name a concrete host or IP",
            ));
        }
        result.push(declaration.clone());
        if target.port().is_none() {
            result.push(authority(host, 80));
            result.push(authority(host, 443));
        }
    }
    result.sort();
    result.dedup();
    Ok(result)
}

fn proxy_network(
    snapshot: &crate::authorization::SandboxPermissionSnapshot,
    declarations: &[String],
) -> Result<SandboxNetwork, ToolError> {
    if declarations.is_empty() {
        return Ok(SandboxNetwork::Disabled);
    }
    let mut targets = Vec::new();
    for declaration in declarations {
        let target: NetworkTarget = declaration.parse().map_err(|error| {
            ToolError::invalid_input(format!("invalid sandbox network target: {error}"))
        })?;
        if target.host().contains(['*', '?', '/']) || target.host().chars().any(char::is_control) {
            return Err(ToolError::invalid_input(
                "sandbox network effects must name a concrete host or IP",
            ));
        }
        deny_network(snapshot.authorize_network_connect(&target), declaration)?;
        // A bare hostname grants the normal HTTP(S) destinations. Other
        // services name their exact port, using the existing host:port field.
        let ports = target
            .port()
            .map_or_else(|| vec![80, 443], |port| vec![port]);
        for port in ports {
            let host = target.host().trim_matches(['[', ']']);
            let concrete: NetworkTarget = authority(host, port)
                .parse()
                .map_err(|error| ToolError::invalid_input_error(&error))?;
            deny_network(
                snapshot.authorize_network_connect(&concrete),
                concrete.original(),
            )?;
            let addresses = (host, port)
                .to_socket_addrs()
                .map_err(|error| {
                    ToolError::invalid_input(format!("resolve sandbox destination {host}: {error}"))
                })?
                .map(|address| address.ip())
                .collect::<Vec<_>>();
            let mut approved = Vec::new();
            for address in addresses {
                let resolved: NetworkTarget = authority(&address.to_string(), port)
                    .parse()
                    .map_err(|error| ToolError::invalid_input_error(&error))?;
                // Reuse the approved hostname inside its network class, while
                // enforcing every IP/CIDR exception and authority ceiling.
                // Crossing into private/loopback space requires an explicit
                // Allow for the resolved address.
                if !matches!(
                    snapshot.authorize_resolved_network_address(&concrete, &resolved),
                    PermissionDecision::Allow,
                ) {
                    continue;
                }
                approved.push(address);
            }
            approved.sort();
            approved.dedup();
            if approved.is_empty() {
                return Err(ToolError::invalid_input(format!(
                    "sandbox destination {host}:{port} has no IP address allowed by the network settings"
                )));
            }
            targets.push(ProxyTarget {
                host: host.to_owned(),
                port: Some(port),
                addresses: approved,
            });
        }
    }
    Ok(SandboxNetwork::HttpProxy { targets })
}

fn authority(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn deny_network(decision: PermissionDecision, target: &str) -> Result<(), ToolError> {
    if let PermissionDecision::Deny { reason } = decision {
        Err(ToolError::invalid_input(format!(
            "sandbox network policy denies {target}: {reason}"
        )))
    } else {
        // Ask/Auto has been resolved by the invocation's effect approval.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn direct_shell_skips_mount_and_dns_requirements_without_changing_other_executors() {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let ordinary = ToolExecutor::new(
            &root,
            crate::authorization::ExecutionPrincipal::new(
                crate::permission::PermissionPolicy::allow_all(),
                crate::permission::ToolPermissionPolicy::allow_all(),
            ),
            agena_plugin_host::PluginHost::new_empty(),
            None,
            None,
        );
        let direct = ordinary.clone().with_sandbox_enabled(false);
        assert!(!direct.sandbox_enabled());
        assert_eq!(
            ordinary.sandbox_enabled(),
            agena_process::sandbox::enabled()
        );
        let input: ShellCommandInput = serde_json::from_value(serde_json::json!({
            "command": "echo direct", "writes": ["does-not-exist"],
            "network": ["no-dns-needed.invalid"]
        }))
        .unwrap();
        let argv = vec!["/bin/sh".into(), "-c".into(), "echo direct".into()];
        let env = HashMap::from([("HOME".into(), "original-home".into())]);
        let (protected, environment) =
            protect_async(&direct, argv.clone(), &root, &input, env.clone())
                .await
                .unwrap();
        assert_eq!(protected, argv);
        assert_eq!(environment, env);
        let policy = policy_for_paths(&direct, &[root.clone()], &[], &input.network).unwrap();
        assert!(policy.disabled);
        assert_eq!(policy.readonly_paths, vec![root]);
    }
    #[cfg(unix)]
    #[test]
    fn sandbox_mount_retargeting_is_rejected_after_authorization() {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let public = root.join("public");
        let secret = root.join("secret");
        std::fs::create_dir(&public).unwrap();
        std::fs::create_dir(&secret).unwrap();
        let executor = ToolExecutor::new(
            &root,
            crate::authorization::ExecutionPrincipal::new(
                crate::permission::PermissionPolicy::allow_all(),
                crate::permission::ToolPermissionPolicy::allow_all(),
            ),
            agena_plugin_host::PluginHost::new_empty(),
            None,
            None,
        );
        let approved = std::fs::canonicalize(&public).unwrap();
        std::fs::rename(&public, root.join("original-public")).unwrap();
        std::os::unix::fs::symlink(&secret, &public).unwrap();
        let error = policy_for_paths(&executor, &[approved], &[], &[]).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("identity changed after authorization")
        );
    }

    #[test]
    fn sandbox_write_grants_preserve_only_explicit_readonly_exceptions() {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let input = root.join("input");
        let output = root.join("output");
        let nested = output.join("nested");
        let locked = output.join("locked");
        for path in [&input, &output, &nested, &locked] {
            std::fs::create_dir(path).unwrap();
        }
        let executor = ToolExecutor::new(
            &root,
            crate::authorization::ExecutionPrincipal::new(
                crate::permission::PermissionPolicy::allow_all(),
                crate::permission::ToolPermissionPolicy::allow_all(),
            ),
            agena_plugin_host::PluginHost::new_empty(),
            None,
            None,
        )
        .with_permission_config(
            &serde_json::from_value(serde_json::json!({
                "path": {"workspace": {"read": "allow", "write": "allow"}, "rules": {
                    "<workspace>/output/locked": {"read": "allow", "write": "deny"},
                    "<workspace>/output/locked/**": {"read": "allow", "write": "deny"}
                }}
            }))
            .unwrap(),
        );
        let policy = policy_for_paths(
            &executor,
            &[input.clone(), output.clone(), nested.clone()],
            std::slice::from_ref(&output),
            &[],
        )
        .unwrap();
        assert_eq!(policy.readwrite_paths, [output.clone()]);
        assert!(policy.readonly_paths.contains(&input));
        assert!(policy.readonly_paths.contains(&locked));
        assert!(!policy.readonly_paths.contains(&output));
        assert!(!policy.readonly_paths.contains(&nested));
    }

    #[test]
    fn empty_network_effects_require_offline_mxc_policy() {
        let principal = crate::authorization::ExecutionPrincipal::new(
            crate::permission::PermissionPolicy::allow_all(),
            crate::permission::ToolPermissionPolicy::allow_all(),
        );
        let snapshot = principal.sandbox_permission_snapshot(Path::new("/workspace"));
        assert_eq!(
            proxy_network(&snapshot, &[]).unwrap(),
            SandboxNetwork::Disabled
        );
    }
}
