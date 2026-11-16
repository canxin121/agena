static PERMISSION_CHECK_WORKERS: agena_async::BlockingPool = agena_async::BlockingPool::new(8);

impl ToolExecutor {
    /// Host resolution and filesystem inspection must not block an async
    /// runtime thread. A running OS resolver retains its bounded worker slot
    /// after cancellation until the syscall returns.
    pub(super) async fn run_permission_preflight<T, F>(&self, operation: F) -> Result<T, ToolError>
    where
        T: Send + 'static,
        F: FnOnce(ToolExecutor) -> Result<T, ToolError> + Send + 'static,
    {
        self.ensure_not_cancelled()?;
        let cancel = self.cancellation_token().cloned();
        // Tokio task-local sandbox overrides do not cross into blocking workers.
        let executor = self.clone().with_sandbox_enabled(self.sandbox_enabled());
        let work = PERMISSION_CHECK_WORKERS.run(move || {
            executor.ensure_not_cancelled()?;
            operation(executor)
        });
        tokio::select! {
            biased;
            _ = async {
                match cancel {
                    Some(token) => token.cancelled().await,
                    None => std::future::pending::<()>().await,
                }
            } => Err(ToolError::Cancelled),
            result = tokio::time::timeout(std::time::Duration::from_secs(30), work) => {
                result.map_err(|_| ToolError::invalid_input("permission preflight or DNS lookup timed out"))?
                    .map_err(|error| ToolError::plugin(format!("permission preflight worker failed: {error}")))?
            }
        }
    }

    pub(crate) fn read_path_filter(&self) -> agena_tool::path_filter::PathFilter<ToolError> {
        let executor = self.clone();
        agena_tool::path_filter::PathFilter::new(move |path| {
            executor.ensure_not_cancelled()?;
            if let Some(check) = executor.read_path_authorizer.as_ref() {
                return check(path);
            }
            if let Some(check) = executor.path_effect_authorizer.as_ref() {
                return check(AccessKind::Read, path);
            }
            let check = executor.path_permission_check(AccessKind::Read, &path.to_string_lossy());
            match check.decision {
                agena_domain::PermissionDecision::Allow => Ok(()),
                agena_domain::PermissionDecision::Deny { reason }
                | agena_domain::PermissionDecision::Ask { reason }
                | agena_domain::PermissionDecision::Auto { reason } => Err(
                    ToolError::PolicyDenied(Box::new(agena_domain::PolicyDeniedResult {
                        action: check.action.clone(),
                        related_actions: vec![],
                        denied_actions: vec![check.action],
                        explanation: reason.clone(),
                        reason,
                        source: Some("static_policy".into()),
                        scope: None,
                        operator: None,
                        authority: agena_domain::PermissionAuthorityKind::StaticPolicy,
                        rule_id: None,
                        rule_revision_ms: None,
                        trace: vec![],
                    })),
                ),
            }
        })
    }
    /// Called only on blocking workers; callbacks may await the host's
    /// existing approval pipeline. Repeated approved effects reuse its cache.
    pub(crate) fn authorize_sandbox_paths(
        &self,
        reads: &[PathBuf],
        writes: &[PathBuf],
    ) -> Result<(), ToolError> {
        let mut all_reads = reads.iter().chain(writes).collect::<Vec<_>>();
        all_reads.sort();
        all_reads.dedup();
        for path in all_reads {
            self.ensure_not_cancelled()?;
            if let Some(check) = &self.path_effect_authorizer {
                check(AccessKind::Read, path)?;
            } else if let Some(check) = &self.read_path_authorizer {
                check(path)?;
            }
        }
        if let Some(check) = &self.path_effect_authorizer {
            for path in writes {
                self.ensure_not_cancelled()?;
                check(AccessKind::Write, path)?;
            }
        }
        Ok(())
    }

    /// Resolve a plugin's concrete access using the same canonical paths and
    /// session principal as executor-owned tools. This does not grant access.
    pub fn path_permission_check(&self, access: AccessKind, path: &str) -> ToolPermissionCheck {
        if self.is_global() && !Path::new(path).is_absolute() {
            return ToolPermissionCheck {
                action: PermissionAction::PathAccess {
                    access_kind: access_kind_name(access).to_string(),
                    workspace_root: String::new(),
                    target_path: path.to_owned(),
                },
                decision: agena_domain::PermissionDecision::Deny {
                    reason: "global sessions require an explicit absolute path".into(),
                },
            };
        }
        let mut checks = Vec::with_capacity(1);
        self.push_resolved_path_check(&mut checks, access, path);
        checks.remove(0)
    }

    pub fn network_permission_check(&self, target: &str) -> Result<ToolPermissionCheck, ToolError> {
        let mut checks = Vec::with_capacity(1);
        self.push_network_check(&mut checks, target)?;
        Ok(checks.remove(0))
    }

    pub async fn network_permission_check_async(
        &self,
        target: &str,
    ) -> Result<ToolPermissionCheck, ToolError> {
        let target = target.to_owned();
        self.run_permission_preflight(move |executor| executor.network_permission_check(&target))
            .await
    }

    pub fn resolve_target_path(&self, raw_path: &str) -> PathBuf {
        self.resolve_target_path_with_context(raw_path, None)
    }

    /// The base folder a tool's relative argument resolves against when the
    /// caller may name one. A project session always keeps its project root;
    /// only a global (Bot) session, which has none, may substitute an absolute
    /// folder.
    pub fn resolve_folder_base(&self, folder: Option<&str>) -> PathBuf {
        if self.is_global()
            && let Some(folder) = folder.map(str::trim).filter(|value| !value.is_empty())
            && Path::new(folder).is_absolute()
        {
            return canonicalize_path_for_execution(Path::new(folder));
        }
        self.workspace_root().to_path_buf()
    }

    /// Resolve a tool's path argument when the caller may name a base folder.
    ///
    /// A project session owns exactly one project root, so its tools always
    /// resolve there; the folder argument is deliberately ignored. A global
    /// (Bot) session has no project of its own, so it may pass an absolute
    /// `folder` plus a relative path and work inside that folder instead of
    /// duplicating the prefix on every argument.
    pub fn resolve_target_path_in_folder(&self, raw_path: &str, folder: Option<&str>) -> PathBuf {
        let candidate = PathBuf::from(raw_path);
        if !self.is_global() || candidate.is_absolute() {
            return self.resolve_target_path_with_context(raw_path, None);
        }
        if folder.is_none_or(|value| value.trim().is_empty()) {
            return self.resolve_target_path_with_context(raw_path, None);
        }
        canonicalize_path_for_execution(&self.resolve_folder_base(folder).join(candidate))
    }

    /// An effect declaration is resolved against the same folder-aware base
    /// that the tool itself used, so a Bot's relative `reads`/`writes` never
    /// fall back to a project root it does not have.
    pub(crate) fn shell_effect_base_path_in_folder(
        &self,
        workdir: Option<&str>,
        folder: Option<&str>,
    ) -> PathBuf {
        workdir
            .map(|workdir| self.resolve_target_path_in_folder(workdir, folder))
            .unwrap_or_else(|| self.resolve_folder_base(folder))
    }

    pub(crate) fn resolve_target_path_with_context(
        &self,
        raw_path: &str,
        session_context: Option<&dyn crate::ToolSessionContext>,
    ) -> PathBuf {
        let workspace_root = self.effective_workspace_root(session_context);
        if let Some(path) = resolve_managed_project_path_alias(raw_path, workspace_root) {
            return canonicalize_path_for_execution(&path);
        }
        let candidate = PathBuf::from(raw_path);
        if candidate.is_absolute() {
            return canonicalize_path_for_execution(&candidate);
        }
        // A global session has no root of its own; joining a relative path
        // there would only produce a filesystem-root path by accident.
        if self.is_global() {
            return canonicalize_path_for_execution(&candidate);
        }
        canonicalize_path_for_execution(&workspace_root.join(candidate))
    }

    pub(crate) fn resolve_filesystem_effect_path(
        &self,
        raw_path: &str,
        base_path: &Path,
    ) -> PathBuf {
        let candidate = PathBuf::from(raw_path);
        let resolved = if candidate.is_absolute() {
            candidate
        } else {
            base_path.join(candidate)
        };
        canonicalize_path_for_execution(&resolved)
    }

    /// Execute a foreground command and forward its output while it runs. The
    /// returned [`ShellOutput`] stays the single source of truth for the
    /// terminal result, so a live consumer never changes the outcome.
    pub(crate) async fn execute_shell_command_with_live(
        &self,
        request: &ShellRequest,
        live: Option<crate::tool::ToolOutputStreams>,
    ) -> Result<ShellOutput, ToolError> {
        match shell::execute_with_sink(request, self.cancellation_token(), live).await {
            Err(agena_tool::ShellError::Cancelled) => Err(ToolError::Cancelled),
            result => result.map_err(ToolError::from),
        }
    }

    pub(crate) fn effective_workspace_root<'a>(
        &'a self,
        session_context: Option<&'a dyn crate::ToolSessionContext>,
    ) -> &'a Path {
        if self.is_global() || session_context.is_some_and(crate::ToolSessionContext::is_global) {
            return self.workspace_root();
        }
        session_context
            .and_then(crate::ToolSessionContext::effective_workspace_root)
            .unwrap_or(self.workspace_root())
    }

    pub(crate) fn display_path(&self, path: &Path) -> String {
        self.display_path_with_context(path, None)
    }

    pub(crate) fn display_path_with_context(
        &self,
        path: &Path,
        session_context: Option<&dyn crate::ToolSessionContext>,
    ) -> String {
        let workspace_root = self.effective_workspace_root(session_context);
        if let Ok(relative) = path.strip_prefix(workspace_root) {
            let normalized = normalize_path_for_display(relative);
            if normalized.is_empty() {
                return ".".to_string();
            }
            return normalized;
        }
        normalize_path_for_display(path)
    }

    /// Record one path decision for the host's own tool.
    ///
    /// The declaration surface is gone: the host no longer asks a plugin which
    /// paths its tool will touch. What survives is the host resolving the path
    /// arguments of the tools *it* owns (the executor-backed builtins), so an
    /// `ask` rule under `permission.path` still reaches the approval flow for
    /// `read`, `glob`, `apply_patch`, `shell.exec` and friends. A plugin that
    /// performs its own I/O is not described here and is not preflighted; it
    /// asks the host itself through `HostClient::require_path_permission`.
    pub(super) fn push_path_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        access: AccessKind,
        target_path: &Path,
    ) {
        let canonical_workspace_root = if self.is_global() {
            PathBuf::new()
        } else {
            canonicalize_path_for_execution(self.workspace_root())
        };
        let canonical_target_path = canonicalize_path_for_execution(target_path);
        let workspace_root = normalize_path_for_display(&canonical_workspace_root);
        let target = normalize_path_for_display(&canonical_target_path);

        checks.push(ToolPermissionCheck {
            action: PermissionAction::PathAccess {
                access_kind: access_kind_name(access).to_string(),
                workspace_root,
                target_path: target,
            },
            decision: self.principal.authorize_path_access(
                access,
                &canonical_workspace_root,
                &canonical_target_path,
            ),
        });
    }

    /// Record one resolved path argument of a host-owned tool.
    pub(super) fn push_resolved_path_check(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        access: AccessKind,
        raw_path: &str,
    ) {
        let target = self.resolve_target_path(raw_path);
        self.push_path_checks(checks, access, target.as_path());
    }

    /// Record one outbound target of a host-owned tool.
    pub(super) fn push_network_check(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        target: &str,
    ) -> Result<(), ToolError> {
        let target: NetworkTarget = target.parse().map_err(|err| {
            ToolError::invalid_input(format!(
                "invalid network permission target `{target}`: {err}"
            ))
        })?;
        let snapshot = self.sandbox_permission_snapshot();
        let requires_dns =
            target.host() != "*" && target.host().parse::<std::net::IpAddr>().is_err();
        let mut decision = if requires_dns {
            crate::shell_sandbox::hostname_resolution_permission(&snapshot)
        } else {
            agena_domain::PermissionDecision::Allow
        };
        if matches!(decision, agena_domain::PermissionDecision::Allow) {
            decision = agena_domain::PermissionDecision::Deny {
                reason: "native network policy denies all resolved destinations".into(),
            };
            for resolved in
                crate::shell_sandbox::resolved_network_targets(target.original(), &snapshot)?
            {
                let candidate = if target.host() == "*" {
                    snapshot.authorize_network_connect(&resolved)
                } else {
                    snapshot.authorize_resolved_network_address(&target, &resolved)
                };
                if matches!(candidate, agena_domain::PermissionDecision::Allow) {
                    decision = candidate;
                    break;
                }
                decision = candidate;
            }
        }
        checks.push(ToolPermissionCheck {
            action: PermissionAction::NetworkAccess {
                target: target.original().to_string(),
                host: target.host().to_string(),
                port: target.port(),
            },
            decision,
        });
        Ok(())
    }

    /// Record every declared `reads`/`writes` path of a shell command.
    pub(super) fn push_filesystem_effect_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        effects: &FilesystemEffects,
        base_path: &Path,
    ) {
        for effect in effects.to_effects() {
            let target = self.resolve_filesystem_effect_path(effect.path.as_str(), base_path);
            // MXC writable mounts also permit reads; approve both effects.
            if effect.access.includes_read() || effect.access.includes_write() {
                self.push_path_checks(checks, AccessKind::Read, target.as_path());
            }
            if effect.access.includes_write() {
                self.push_path_checks(checks, AccessKind::Write, target.as_path());
            }
        }
    }
}
use super::{
    AccessKind, FilesystemEffects, NetworkTarget, Path, PathBuf, ShellOutput, ShellRequest,
    ToolError, ToolExecutor, ToolPermissionCheck, access_kind_name,
    canonicalize_path_for_execution, normalize_path_for_display,
    resolve_managed_project_path_alias, shell,
};
use agena_domain::PermissionAction;

#[cfg(test)]
mod permission_worker_tests {
    use super::*;

    fn executor() -> ToolExecutor {
        ToolExecutor::new(
            std::env::temp_dir(),
            crate::authorization::ExecutionPrincipal::new(
                crate::permission::PermissionPolicy::allow_all(),
                crate::permission::ToolPermissionPolicy::allow_all(),
            ),
            agena_plugin_host::PluginHost::new_empty(),
            None,
            None,
        )
    }

    #[tokio::test(flavor = "current_thread")]
    async fn blocking_permission_work_leaves_the_async_runtime_responsive() {
        let executor = executor();
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, waiting) = std::sync::mpsc::channel();
        let worker = executor.run_permission_preflight(move |_| {
            started.send(()).unwrap();
            waiting
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
            Ok(42)
        });
        let caller = async {
            ready.await.unwrap();
            tokio::task::yield_now().await;
            release.send(()).unwrap();
        };
        let (result, ()) = tokio::join!(worker, caller);
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancelling_permission_preflight_returns_while_an_os_lookup_is_running() {
        let cancel = tokio_util::sync::CancellationToken::new();
        let executor = executor().with_cancellation_token(Some(cancel.clone()));
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, waiting) = std::sync::mpsc::channel();
        let worker = executor.run_permission_preflight(move |_| {
            started.send(()).unwrap();
            waiting
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
            Ok(())
        });
        let canceller = async {
            ready.await.unwrap();
            cancel.cancel();
        };
        let (result, ()) = tokio::join!(worker, canceller);
        assert!(matches!(result, Err(ToolError::Cancelled)));
        release.send(()).unwrap();
    }
}
