use super::{ToolError, ToolExecutor};
use agena_domain::{AccessKind, NetworkTarget, PermissionDecision};
use agena_plugin_host::sdk::host_api::HostProcessResources;
use std::path::{Path, PathBuf};

impl ToolExecutor {
    /// Resolve the actual cwd and normalize resource declarations only when
    /// direct execution uses fallback checks. MXC ignores declared scopes.
    pub fn resolve_process_sandbox_resources(
        &self,
        input: HostProcessResources,
    ) -> Result<HostProcessResources, ToolError> {
        if self.resource_guard_enabled()
            && (input.reads.len() + input.writes.len() > 512 || input.network.len() > 128)
        {
            return Err(ToolError::invalid_input(
                "too many process resource declarations",
            ));
        }
        let cwd = match input.cwd.as_deref() {
            Some(raw) => self.process_resource_path(raw, self.workspace_root())?,
            None if self.is_global() => {
                return Err(ToolError::invalid_input(
                    "a global process scope requires an absolute working directory",
                ));
            }
            None => crate::canonicalize_mutation_path(self.workspace_root()),
        };
        if !cwd.is_absolute() || !cwd.is_dir() {
            return Err(ToolError::invalid_input(
                "process working directory must be an existing absolute directory",
            ));
        }
        if !self.resource_guard_enabled() {
            return Ok(HostProcessResources {
                cwd: Some(cwd.to_string_lossy().into_owned()),
                ..Default::default()
            });
        }
        let writes = input
            .writes
            .iter()
            .map(|path| self.process_resource_path(path, &cwd))
            .collect::<Result<Vec<_>, _>>()?;
        let mut reads = input
            .reads
            .iter()
            .map(|path| self.process_resource_path(path, &cwd))
            .collect::<Result<Vec<_>, _>>()?;
        reads.push(cwd.clone());
        reads.extend(writes.iter().cloned());
        let normalize = |paths: Vec<PathBuf>| {
            let mut paths = paths
                .into_iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            paths.sort();
            paths.dedup();
            paths
        };
        let network = crate::shell_sandbox::declared_network_targets(&input.network)?;
        Ok(HostProcessResources {
            cwd: Some(cwd.to_string_lossy().into_owned()),
            reads: normalize(reads),
            writes: normalize(writes),
            network,
        })
    }

    fn process_resource_path(&self, raw: &str, base: &Path) -> Result<PathBuf, ToolError> {
        if raw.is_empty() || raw.contains('\0') {
            return Err(ToolError::invalid_input(
                "process resource paths must be nonempty and contain no NUL",
            ));
        }
        let workspace_alias = raw
            .strip_prefix("<workspace>")
            .filter(|suffix| suffix.is_empty() || suffix.starts_with(['/', '\\']));
        let path = if let Some(suffix) = workspace_alias {
            if self.is_global() {
                return Err(ToolError::invalid_input(
                    "global sessions have no <workspace> path",
                ));
            }
            self.workspace_root()
                .join(suffix.trim_start_matches(['/', '\\']))
        } else if Path::new(raw).is_absolute() {
            PathBuf::from(raw)
        } else {
            base.join(raw)
        };
        if !path.is_absolute() {
            return Err(ToolError::invalid_input(
                "process resources require an absolute owning directory",
            ));
        }
        Ok(crate::canonicalize_mutation_path(&path))
    }

    pub fn process_sandbox_permission_checks(
        &self,
        resources: &HostProcessResources,
    ) -> Result<Vec<agena_tool::ToolPermissionCheck>, ToolError> {
        if !self.resource_guard_enabled() {
            return Ok(Vec::new());
        }
        let mut checks = Vec::new();
        for path in &resources.reads {
            checks.push(self.path_permission_check(AccessKind::Read, path));
        }
        for path in &resources.writes {
            checks.push(self.path_permission_check(AccessKind::Write, path));
        }
        for target in &resources.network {
            checks.push(self.network_permission_check(target)?);
        }
        Ok(checks)
    }

    pub async fn process_sandbox_permission_checks_async(
        &self,
        resources: &HostProcessResources,
    ) -> Result<Vec<agena_tool::ToolPermissionCheck>, ToolError> {
        let resources = resources.clone();
        self.run_permission_preflight(move |executor| {
            executor.process_sandbox_permission_checks(&resources)
        })
        .await
    }

    /// Compile with current persisted decisions. Native processes receive the
    /// configured scope; direct fallback approvals retain their call lifetime.
    pub async fn prepare_process_sandbox_policy(
        &self,
        resources: HostProcessResources,
    ) -> Result<(serde_json::Value, bool), ToolError> {
        self.run_permission_preflight(move |executor| {
            if executor.sandbox_enabled() || !executor.resource_guard_enabled() {
                let cwd = resources.cwd.as_deref().ok_or_else(|| {
                    ToolError::invalid_input("process working directory is missing")
                })?;
                executor.validate_sandbox_workdir(Path::new(cwd))?;
                return Ok((
                    serde_json::to_value(crate::shell_sandbox::policy_from_settings(&executor)?)
                        .map_err(|error| ToolError::invalid_input_error(&error))?,
                    false,
                ));
            }
            let snapshot = executor.sandbox_permission_snapshot();
            let reads = resources
                .reads
                .iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>();
            let writes = resources
                .writes
                .iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>();
            let mut call_bound = reads.iter().any(|path| {
                !matches!(
                    snapshot.authorize_path_access(AccessKind::Read, path),
                    PermissionDecision::Allow
                )
            }) || writes.iter().any(|path| {
                !matches!(
                    snapshot.authorize_path_access(AccessKind::Write, path),
                    PermissionDecision::Allow
                )
            });
            for target in &resources.network {
                let target: NetworkTarget = target.parse().map_err(|error| {
                    ToolError::invalid_input(format!("invalid process network target: {error}"))
                })?;
                call_bound |= !matches!(
                    snapshot.authorize_network_connect(&target),
                    PermissionDecision::Allow
                );
            }
            let policy = crate::shell_sandbox::policy_for_paths(
                &executor,
                &reads,
                &writes,
                &resources.network,
            )?;
            Ok((
                serde_json::to_value(policy)
                    .map_err(|error| ToolError::invalid_input_error(&error))?,
                call_bound,
            ))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_runtime_contracts::{
        authorization::ExecutionPrincipal,
        permission::{PermissionPolicy, ToolPermissionPolicy},
    };

    fn executor(root: &Path) -> ToolExecutor {
        ToolExecutor::new(
            root.to_owned(),
            ExecutionPrincipal::new(
                PermissionPolicy::allow_all(),
                ToolPermissionPolicy::allow_all(),
            ),
            agena_plugin_host::PluginHost::new_empty(),
            None,
            None,
        )
    }

    #[test]
    fn resource_normalization_checks_cwd_and_write_reads_and_concrete_ports() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir(root.join("output")).unwrap();
        let executor = executor(&root).with_sandbox_enabled(false);
        let resources = executor
            .resolve_process_sandbox_resources(HostProcessResources {
                writes: vec!["output".into()],
                network: vec!["example.com".into()],
                ..Default::default()
            })
            .unwrap();
        assert_eq!(resources.cwd.as_deref(), Some(root.to_str().unwrap()));
        assert!(
            resources
                .reads
                .contains(&root.to_string_lossy().into_owned())
        );
        assert!(
            resources
                .reads
                .contains(&root.join("output").to_string_lossy().into_owned())
        );
        assert_eq!(resources.network, ["example.com"]);
        assert_eq!(
            executor
                .process_sandbox_permission_checks(&resources)
                .unwrap()
                .len(),
            4
        );
        assert!(
            executor
                .resolve_process_sandbox_resources(HostProcessResources {
                    writes: vec!["missing".into()],
                    ..Default::default()
                })
                .is_ok()
        );
    }

    #[tokio::test]
    async fn direct_process_approvals_keep_one_call_lifetimes() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir(root.join("secret")).unwrap();
        // Public path settings contain concrete modes only. Build the
        // transient approval policy directly to retain the one-call grant
        // regression without accepting ask/auto as persisted path modes.
        let mut policy = PermissionPolicy::new(
            agena_domain::PermissionMode::Ask,
            agena_domain::PermissionMode::Allow,
        );
        for pattern in ["<workspace>/secret", "<workspace>/secret/**"] {
            policy
                .add_path_pattern_rule(
                    agena_domain::AccessSelector::Any,
                    agena_domain::PermissionMode::Deny,
                    pattern,
                )
                .unwrap();
        }
        let executor = ToolExecutor::new(
            root.clone(),
            ExecutionPrincipal::new(policy, ToolPermissionPolicy::allow_all()),
            agena_plugin_host::PluginHost::new_empty(),
            None,
            None,
        )
        .with_sandbox_enabled(false);
        let resources = executor
            .resolve_process_sandbox_resources(Default::default())
            .unwrap();
        let (policy, call_bound) = executor
            .prepare_process_sandbox_policy(resources)
            .await
            .unwrap();
        assert!(call_bound);
        let policy: agena_process::sandbox::SandboxPolicy = serde_json::from_value(policy).unwrap();
        assert!(policy.disabled);
        assert!(matches!(
            policy.network,
            agena_process::sandbox::SandboxNetwork::Disabled
        ));
        let executor = executor.with_permission_config(
            &serde_json::from_value(serde_json::json!({
                "path": {"workspace": "readwrite"}
            }))
            .unwrap(),
        );
        let resources = executor
            .resolve_process_sandbox_resources(Default::default())
            .unwrap();
        assert!(
            !executor
                .prepare_process_sandbox_policy(resources)
                .await
                .unwrap()
                .1
        );
    }

    #[tokio::test]
    async fn sandbox_process_uses_configured_roots_and_network_without_declarations() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir(root.join("input")).unwrap();
        std::fs::create_dir(root.join("output")).unwrap();
        let executor = executor(&root).with_permission_config(
            &serde_json::from_value(serde_json::json!({
                "path": {"workspace":"deny", "external":"deny", "rules": {
                    "<workspace>/input":"readonly", "<workspace>/output":"readwrite"
                }},
                "network": {"default":"deny"}
            }))
            .unwrap(),
        );
        let resources = executor
            .resolve_process_sandbox_resources(HostProcessResources {
                cwd: Some(root.join("input").to_string_lossy().into_owned()),
                reads: vec!["/not-authorized".into()],
                writes: vec!["missing-and-not-configured".into()],
                network: vec!["not a valid target".into()],
                ..Default::default()
            })
            .unwrap();
        assert!(
            resources.reads.is_empty()
                && resources.writes.is_empty()
                && resources.network.is_empty()
        );
        assert!(
            executor
                .process_sandbox_permission_checks(&resources)
                .unwrap()
                .is_empty()
        );
        let (value, call_bound) = executor
            .prepare_process_sandbox_policy(resources)
            .await
            .unwrap();
        let policy: agena_process::sandbox::SandboxPolicy = serde_json::from_value(value).unwrap();
        assert!(!call_bound);
        assert!(!policy.disabled);
        assert_eq!(policy.readonly_paths, [root.join("input")]);
        assert_eq!(policy.readwrite_paths, [root.join("output")]);
        assert!(matches!(
            policy.network,
            agena_process::sandbox::SandboxNetwork::Native { .. }
        ));
    }
}
