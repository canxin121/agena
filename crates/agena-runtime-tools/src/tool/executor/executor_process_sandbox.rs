use super::{ToolError, ToolExecutor};
use agena_domain::{AccessKind, NetworkTarget, PermissionDecision};
use agena_plugin_host::sdk::host_api::HostProcessResources;
use std::path::{Path, PathBuf};

static PROCESS_POLICY_WORKERS: agena_async::BlockingPool = agena_async::BlockingPool::new(4);

impl ToolExecutor {
    /// Resolve declarations before approval. The resulting absolute paths are
    /// retained for compilation; relative resource paths use the child's cwd.
    pub fn resolve_process_sandbox_resources(
        &self,
        input: HostProcessResources,
    ) -> Result<HostProcessResources, ToolError> {
        if input.reads.len() + input.writes.len() > 512 || input.network.len() > 128 {
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
        let writes = input
            .writes
            .iter()
            .map(|path| self.process_resource_path(path, &cwd))
            .collect::<Result<Vec<_>, _>>()?;
        if writes.iter().any(|path| !path.exists()) {
            return Err(ToolError::invalid_input(
                "process writable mounts must already exist; declare an existing output directory",
            ));
        }
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

    /// Compile only after the host authorized `resources` and loaded current
    /// persisted decisions. The boolean marks approvals limited to this call.
    pub async fn prepare_process_sandbox_policy(
        &self,
        resources: HostProcessResources,
    ) -> Result<(serde_json::Value, bool), ToolError> {
        let executor = self.clone();
        PROCESS_POLICY_WORKERS
            .run(move || {
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
            .map_err(|error| {
                ToolError::invalid_input(format!("process sandbox policy worker failed: {error}"))
            })?
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
        let executor = executor(&root);
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
        assert_eq!(
            resources.network,
            ["example.com", "example.com:443", "example.com:80"]
        );
        assert_eq!(
            executor
                .process_sandbox_permission_checks(&resources)
                .unwrap()
                .len(),
            6
        );
        assert!(
            executor
                .resolve_process_sandbox_resources(HostProcessResources {
                    writes: vec!["missing".into()],
                    ..Default::default()
                })
                .is_err()
        );
    }

    #[tokio::test]
    async fn one_call_grants_are_marked_and_secret_descendants_stay_denied() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir(root.join("secret")).unwrap();
        let executor = executor(&root).with_permission_config(
            &serde_json::from_value(serde_json::json!({
                "path": {"workspace": {"read": "ask", "write": "allow"}, "rules": {
                    "<workspace>/secret": {"read": "deny", "write": "deny"},
                    "<workspace>/secret/**": {"read": "deny", "write": "deny"}
                }}
            }))
            .unwrap(),
        );
        let resources = executor
            .resolve_process_sandbox_resources(Default::default())
            .unwrap();
        let (policy, call_bound) = executor
            .prepare_process_sandbox_policy(resources)
            .await
            .unwrap();
        assert!(call_bound);
        let policy: agena_process::sandbox::SandboxPolicy = serde_json::from_value(policy).unwrap();
        assert!(policy.denied_paths.contains(&root.join("secret")));
        assert!(matches!(
            policy.network,
            agena_process::sandbox::SandboxNetwork::Disabled
        ));
        let executor = executor.with_permission_config(
            &serde_json::from_value(serde_json::json!({
                "path": {"workspace": {"read": "allow", "write": "allow"}}
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
}
