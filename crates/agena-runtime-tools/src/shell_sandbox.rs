//! MXC isolation for model-owned native processes. The existing permission
//! service approves effects; this module projects that exact scoped authority
//! into kernel-enforced filesystem and network boundaries.
mod dns;

use crate::{
    part::ShellCommandInput,
    tool::{ToolError, ToolExecutor},
};
use agena_domain::{
    NetworkAction, NetworkEgressConfig, NetworkIngressConfig, NetworkPeerConfig,
    NetworkPermissionConfig, NetworkPortConfig, NetworkProtocol, NetworkRuleConfig, NetworkTarget,
    PermissionDecision,
};
use agena_process::sandbox::{SandboxNetwork, SandboxPolicy};
use dns::resolver_targets;
use std::{
    collections::HashMap,
    net::{IpAddr, ToSocketAddrs},
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
        network: native_network(&snapshot, network)?,
        ..Default::default()
    })
}

/// A declaration requests an intersection with the configured native policy.
/// "*" requests that policy as a whole; [] always requests offline execution.
pub(crate) fn declared_network_targets(declarations: &[String]) -> Result<Vec<String>, ToolError> {
    if declarations.iter().any(|value| value == "*") {
        if declarations.len() != 1 {
            return Err(ToolError::invalid_input(
                "network \"*\" must be the only declaration",
            ));
        }
        return Ok(vec!["*".into()]);
    }
    let mut result = Vec::new();
    for declaration in declarations {
        let target: NetworkTarget = declaration.parse().map_err(|error| {
            ToolError::invalid_input(format!("invalid sandbox network target: {error}"))
        })?;
        let host = target.host();
        if host.contains(['*', '?', '/'])
            || host.chars().any(char::is_control)
            || target.port() == Some(0)
        {
            return Err(ToolError::invalid_input(
                "network effects require a concrete host or IP, optionally with a port; use a single \"*\" to request the configured native policy",
            ));
        }
        if let Some((scheme, _)) = declaration.split_once("://") {
            if !matches!(
                scheme.to_ascii_lowercase().as_str(),
                "http" | "https" | "tcp" | "udp" | "icmp" | "ws" | "wss" | "ssh" | "ftp"
            ) {
                return Err(ToolError::invalid_input(
                    "unsupported network declaration scheme; use tcp://, udp://, icmp:// or an HTTP(S) URL",
                ));
            }
        }
        if declaration.starts_with("icmp://") && target.port().is_some() {
            return Err(ToolError::invalid_input(
                "ICMP declarations cannot specify a port",
            ));
        }
        result.push(declaration.clone());
    }
    result.sort();
    result.dedup();
    Ok(result)
}

fn protocol(target: &NetworkTarget) -> NetworkProtocol {
    match target
        .original()
        .split_once("://")
        .map(|(scheme, _)| scheme.to_ascii_lowercase())
    {
        Some(scheme) if scheme == "udp" => NetworkProtocol::Udp,
        Some(scheme) if scheme == "icmp" => NetworkProtocol::Icmp,
        Some(_) => NetworkProtocol::Tcp,
        None if target.port().is_some() => NetworkProtocol::Tcp,
        None => NetworkProtocol::Any,
    }
}

/// Do not query the host resolver merely to discover that networking is
/// disabled. A hostname additionally requires its DNS dependency to be allowed.
pub(crate) fn hostname_resolution_permission(
    snapshot: &crate::authorization::SandboxPermissionSnapshot,
) -> PermissionDecision {
    let any: NetworkTarget = "*".parse().expect("wildcard declaration");
    if let denied @ PermissionDecision::Deny { .. } = snapshot.authorize_network_connect(&any) {
        return denied;
    }
    let resolvers = resolver_targets();
    if !resolvers.is_empty() {
        if resolvers.iter().all(|target| {
            matches!(
                snapshot.authorize_network_connect(target),
                PermissionDecision::Allow
            )
        }) {
            return PermissionDecision::Allow;
        }
        return PermissionDecision::Deny {
            reason: "native network settings must allow the configured DNS resolvers on UDP and TCP port 53 before resolving hostname declarations; use literal IP declarations or authorize those DNS endpoints".into(),
        };
    }
    // When the OS does not expose a resolver list here, only a completely
    // unrestricted egress policy proves that a host-side lookup is authorized.
    if let Ok(policy) = snapshot.native_network_policy(None) {
        let egress = policy.egress.expect("resolved egress");
        if egress.default == Some(NetworkAction::Allow)
            && egress.deny.as_ref().is_none_or(Vec::is_empty)
        {
            return PermissionDecision::Allow;
        }
    }
    PermissionDecision::Deny {
        reason: "the native policy cannot authorize host DNS resolution on this platform; declare literal IPs or use network:[\"*\"] to let the sandbox enforce the full configured policy".into(),
    }
}

/// Resolve a declaration without granting its DNS name any authority. Callers
/// check every usable address, and launch performs the same check again.
pub(crate) fn resolved_network_targets(
    declaration: &str,
    snapshot: &crate::authorization::SandboxPermissionSnapshot,
) -> Result<Vec<NetworkTarget>, ToolError> {
    declared_network_targets(&[declaration.to_owned()])?;
    let target: NetworkTarget = declaration
        .parse()
        .map_err(|error| ToolError::invalid_input(format!("invalid network target: {error}")))?;
    if target.host() == "*" || target.host().parse::<IpAddr>().is_ok() {
        return Ok(vec![target]);
    }
    if let PermissionDecision::Deny { reason } = hostname_resolution_permission(snapshot) {
        return Err(ToolError::invalid_input(reason));
    }
    let mut addresses = (target.host(), target.port().unwrap_or(0))
        .to_socket_addrs()
        .map_err(|error| {
            ToolError::invalid_input(format!(
                "resolve network destination {}: {error}",
                target.host()
            ))
        })?
        .map(|address| address.ip())
        .collect::<Vec<_>>();
    addresses.sort();
    addresses.dedup();
    if addresses.is_empty() {
        return Err(ToolError::invalid_input(
            "network destination resolved to no addresses",
        ));
    }
    addresses
        .into_iter()
        .map(|address| {
            let host = if address.is_ipv6() {
                format!("[{address}]")
            } else {
                address.to_string()
            };
            let endpoint = match target.port() {
                Some(port) => format!("{host}:{port}"),
                None => host,
            };
            let value = match protocol(&target) {
                NetworkProtocol::Udp => format!("udp://{endpoint}"),
                NetworkProtocol::Icmp => format!("icmp://{endpoint}"),
                NetworkProtocol::Tcp => format!("tcp://{endpoint}"),
                NetworkProtocol::Any => endpoint,
            };
            value.parse().map_err(|error| {
                ToolError::invalid_input(format!("resolved network destination: {error}"))
            })
        })
        .collect()
}

fn endpoint_rule(target: &NetworkTarget) -> Result<NetworkRuleConfig, ToolError> {
    let address: IpAddr = target
        .host()
        .parse()
        .map_err(|_| ToolError::invalid_input("native MXC needs a numeric network destination"))?;
    let selector = protocol(target);
    Ok(NetworkRuleConfig {
        to: Some(vec![NetworkPeerConfig {
            cidr: format!("{address}/{}", if address.is_ipv4() { 32 } else { 128 }),
            except: vec![],
        }]),
        ports: (selector != NetworkProtocol::Any || target.port().is_some()).then(|| {
            vec![NetworkPortConfig {
                protocol: Some(selector),
                port: target.port(),
                end_port: None,
            }]
        }),
    })
}

fn native_network(
    snapshot: &crate::authorization::SandboxPermissionSnapshot,
    declarations: &[String],
) -> Result<SandboxNetwork, ToolError> {
    let declarations = declared_network_targets(declarations)?;
    if declarations.is_empty() {
        return Ok(SandboxNetwork::Disabled);
    }
    let policy = if declarations.as_slice() == ["*"] {
        snapshot.native_network_policy(None)
    } else {
        let mut allow = Vec::new();
        let mut needs_dns = false;
        for declaration in &declarations {
            let original: NetworkTarget = declaration.parse().map_err(|error| {
                ToolError::invalid_input(format!("invalid network target: {error}"))
            })?;
            needs_dns |= original.host().parse::<IpAddr>().is_err();
            let mut approved = false;
            for target in resolved_network_targets(declaration, snapshot)? {
                if matches!(
                    snapshot.authorize_resolved_network_address(&original, &target),
                    PermissionDecision::Allow
                ) {
                    allow.push(endpoint_rule(&target)?);
                    approved = true;
                }
            }
            if !approved {
                return Err(ToolError::invalid_input(format!(
                    "native network policy denies every resolved address for {declaration}"
                )));
            }
        }
        // MXC keeps host resolvers and maps loopback resolvers to its Linux
        // namespace DNS forwarder. DNS dependencies never exceed settings.
        if needs_dns {
            for target in resolver_targets() {
                if matches!(
                    snapshot.authorize_network_connect(&target),
                    PermissionDecision::Allow
                ) {
                    allow.push(endpoint_rule(&target)?);
                }
            }
        }
        let scope = NetworkPermissionConfig {
            egress: Some(NetworkEgressConfig {
                default: Some(NetworkAction::Deny),
                allow: Some(allow),
                deny: Some(vec![]),
            }),
            // Declarations constrain outbound effects. Inbound and host-loopback
            // authority remain the intersection of the configured policy layers.
            ingress: Some(NetworkIngressConfig {
                default: Some(NetworkAction::Allow),
                host_loopback: Some(NetworkAction::Allow),
            }),
        };
        snapshot.native_network_policy(Some(&scope))
    }
    .map_err(|error| ToolError::invalid_input(format!("native network policy: {error}")))?;
    policy.validate_for_platform(std::env::consts::OS).map_err(|error| {
        ToolError::invalid_input(format!("{error}; this backend cannot narrow network access to these declarations; request network:[\"*\"] only when the full configured native policy is intended"))
    })?;
    Ok(SandboxNetwork::Native { policy })
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
            native_network(&snapshot, &[]).unwrap(),
            SandboxNetwork::Disabled
        );
    }
    fn network_snapshot(
        value: serde_json::Value,
    ) -> crate::authorization::SandboxPermissionSnapshot {
        crate::authorization::ExecutionPrincipal::new(
            crate::permission::PermissionPolicy::allow_all(),
            crate::permission::ToolPermissionPolicy::allow_all(),
        )
        .try_apply_permission_config(
            &serde_json::from_value(serde_json::json!({"network":value})).unwrap(),
        )
        .unwrap()
        .sandbox_permission_snapshot(Path::new("/workspace"))
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn native_network_scopes_keep_configured_denials_and_other_destinations_closed() {
        let snapshot = network_snapshot(serde_json::json!({
            "default":"deny","allow":[{
                "to":"192.0.2.0/24","protocol":"tcp","ports":["8000-8099"]
            }],"deny":[{
                "to":"*","protocol":"tcp","ports":["8080-8089"]
            }]
        }));
        let SandboxNetwork::Native { policy } =
            native_network(&snapshot, &["192.0.2.10".into()]).unwrap()
        else {
            panic!("expected native MXC policy");
        };
        let compiled = crate::permission::NetworkPermissionPolicy::from_config(&policy).unwrap();
        assert!(matches!(
            compiled.check_connect(&"192.0.2.10:8001".parse().unwrap()),
            PermissionDecision::Allow
        ));
        for target in [
            "192.0.2.10:8085",
            "192.0.2.11:8001",
            "udp://192.0.2.10:8001",
        ] {
            assert!(
                matches!(
                    compiled.check_connect(&target.parse().unwrap()),
                    PermissionDecision::Deny { .. }
                ),
                "{target}"
            );
        }
        assert_eq!(policy.egress.unwrap().deny.unwrap().len(), 1);
    }

    #[test]
    fn explicit_full_network_scope_uses_configured_native_defaults() {
        let snapshot = network_snapshot(serde_json::json!({"default":"allow"}));
        let SandboxNetwork::Native { policy } = native_network(&snapshot, &["*".into()]).unwrap()
        else {
            panic!("expected native MXC policy");
        };
        assert_eq!(policy.egress.unwrap().default, Some(NetworkAction::Allow));
        assert_eq!(
            policy.ingress.unwrap().host_loopback,
            Some(NetworkAction::Deny)
        );
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    #[test]
    fn explicit_full_network_scope_preserves_native_ingress_without_egress() {
        let cases = [
            ("allow", "deny"),
            ("allow", "allow"),
            #[cfg(target_os = "windows")]
            ("deny", "allow"),
        ];
        for (inbound, host_loopback) in cases {
            let snapshot = network_snapshot(serde_json::json!({
                "default":"deny","allow":[],"deny":[],
                "inbound":inbound,"hostLoopback":host_loopback
            }));
            assert!(matches!(
                snapshot.authorize_network_connect(&"*".parse().unwrap()),
                PermissionDecision::Allow
            ));
            let SandboxNetwork::Native { policy } =
                native_network(&snapshot, &["*".into()]).unwrap()
            else {
                panic!("expected the configured ingress policy");
            };
            policy.validate_for_platform(std::env::consts::OS).unwrap();
            let egress = policy.egress.unwrap();
            assert_eq!(egress.default, Some(NetworkAction::Deny));
            assert!(egress.allow.unwrap().is_empty());
            let ingress = policy.ingress.unwrap();
            assert_eq!(
                ingress.default,
                Some(if inbound == "allow" {
                    NetworkAction::Allow
                } else {
                    NetworkAction::Deny
                })
            );
            assert_eq!(
                ingress.host_loopback,
                Some(if host_loopback == "allow" {
                    NetworkAction::Allow
                } else {
                    NetworkAction::Deny
                })
            );
            assert_eq!(
                native_network(&snapshot, &[]).unwrap(),
                SandboxNetwork::Disabled
            );
        }
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn native_udp_scope_preserves_transport_protocol() {
        let snapshot = network_snapshot(serde_json::json!({
            "default":"deny","allow":[{
                "to":"192.0.2.53","protocol":"udp","ports":[53]
            }]
        }));
        let SandboxNetwork::Native { policy } =
            native_network(&snapshot, &["udp://192.0.2.53:53".into()]).unwrap()
        else {
            panic!("expected native MXC policy");
        };
        let compiled = crate::permission::NetworkPermissionPolicy::from_config(&policy).unwrap();
        assert!(matches!(
            compiled.check_connect(&"udp://192.0.2.53:53".parse().unwrap()),
            PermissionDecision::Allow
        ));
        assert!(matches!(
            compiled.check_connect(&"192.0.2.53:53".parse().unwrap()),
            PermissionDecision::Deny { .. }
        ));
    }
}
