impl ToolExecutor {
    pub(crate) fn resolve_target_path(&self, raw_path: &str) -> PathBuf {
        self.resolve_target_path_with_context(raw_path, None)
    }

    pub(crate) fn shell_effect_base_path(&self, workdir: Option<&str>) -> PathBuf {
        workdir
            .map(|workdir| self.resolve_target_path(workdir))
            .unwrap_or_else(|| self.workspace_root().to_path_buf())
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

    pub(crate) fn resolve_target_path_with_context(
        &self,
        raw_path: &str,
        session_context: Option<&crate::session::SessionExecutionContext>,
    ) -> PathBuf {
        let workspace_root = self.effective_workspace_root(session_context);
        if let Some(path) = resolve_managed_project_path_alias(raw_path, workspace_root) {
            return canonicalize_path_for_execution(&path);
        }
        let candidate = PathBuf::from(raw_path);
        if candidate.is_absolute() {
            return canonicalize_path_for_execution(&candidate);
        }
        canonicalize_path_for_execution(&workspace_root.join(candidate))
    }

    pub(crate) fn execute_shell_command(
        &self,
        request: &ShellRequest,
    ) -> Result<ShellOutput, ToolError> {
        match shell::execute(request, self.cancellation_token()) {
            Err(agena_tool::ShellError::Cancelled) => Err(ToolError::Cancelled),
            result => result.map_err(ToolError::from),
        }
    }

    pub(crate) fn effective_workspace_root<'a>(
        &'a self,
        session_context: Option<&'a crate::session::SessionExecutionContext>,
    ) -> &'a Path {
        session_context
            .and_then(|context| context.effective_workspace_root.as_deref())
            .unwrap_or(self.workspace_root())
    }

    pub(crate) fn display_path(&self, path: &Path) -> String {
        self.display_path_with_context(path, None)
    }

    pub(crate) fn display_path_with_context(
        &self,
        path: &Path,
        session_context: Option<&crate::session::SessionExecutionContext>,
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

    pub(crate) fn ensure_read_permission(&self, target_path: &Path) -> Result<(), ToolError> {
        self.ensure_access_permission(AccessKind::Read, target_path)
    }

    pub(crate) fn ensure_edit_permission(&self, target_path: &Path) -> Result<(), ToolError> {
        self.ensure_access_permission(AccessKind::Write, target_path)
    }

    pub(crate) fn ensure_filesystem_effects_permission(
        &self,
        effects: &[FilesystemEffect],
        base_path: &Path,
    ) -> Result<(), ToolError> {
        for effect in effects {
            let target = self.resolve_filesystem_effect_path(effect.path.as_str(), base_path);
            if effect.access.includes_read() {
                self.ensure_access_permission(AccessKind::Read, &target)?;
            }
            if effect.access.includes_write() {
                self.ensure_access_permission(AccessKind::Write, &target)?;
            }
        }
        Ok(())
    }

    pub(crate) fn ensure_network_effects_permission(
        &self,
        effects: &[NetworkEffect],
    ) -> Result<(), ToolError> {
        for effect in effects {
            let target: NetworkTarget = effect.target.parse().map_err(|err| {
                ToolError::InvalidInput(format!(
                    "invalid network effect target `{}`: {err}",
                    effect.target
                ))
            })?;
            self.ensure_network_permission(&target)?;
        }
        Ok(())
    }

    pub(crate) fn ensure_network_permission(
        &self,
        target: &NetworkTarget,
    ) -> Result<(), ToolError> {
        if self.permission_mode == PermissionEnforcementMode::Bypassed {
            return Ok(());
        }

        match self.agent.authorize_network_connect(target) {
            PermissionDecision::Allow => Ok(()),
            PermissionDecision::Ask { reason } => Err(ToolError::PermissionAsk(reason)),
            PermissionDecision::Deny { reason } => Err(ToolError::PermissionDenied(reason)),
        }
    }

    pub(crate) fn ensure_access_permission(
        &self,
        access: AccessKind,
        target_path: &Path,
    ) -> Result<(), ToolError> {
        if self.permission_mode == PermissionEnforcementMode::Bypassed {
            return Ok(());
        }

        let workspace_root = canonicalize_path_for_execution(self.workspace_root());
        let target_path = canonicalize_path_for_execution(target_path);
        match self
            .agent
            .authorize_path_access(access, &workspace_root, &target_path)
        {
            PermissionDecision::Allow => Ok(()),
            PermissionDecision::Ask { reason } => Err(ToolError::PermissionAsk(reason)),
            PermissionDecision::Deny { reason } => Err(ToolError::PermissionDenied(reason)),
        }
    }

    pub(crate) fn push_path_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        access: AccessKind,
        target_path: &Path,
    ) {
        let canonical_workspace_root = canonicalize_path_for_execution(self.workspace_root());
        let canonical_target_path = canonicalize_path_for_execution(target_path);
        let workspace_root = normalize_path_for_display(&canonical_workspace_root);
        let target = normalize_path_for_display(&canonical_target_path);

        checks.push(ToolPermissionCheck {
            action: PermissionAction::PathAccess {
                access_kind: access_kind_name(access).to_string(),
                workspace_root,
                target_path: target,
            },
            decision: self.agent.authorize_path_access(
                access,
                &canonical_workspace_root,
                &canonical_target_path,
            ),
        });
    }

    pub(crate) fn push_network_check(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        target: &str,
    ) -> Result<(), ToolError> {
        let target: NetworkTarget = target.parse().map_err(|err| {
            ToolError::InvalidInput(format!(
                "invalid network permission target `{target}`: {err}"
            ))
        })?;
        checks.push(ToolPermissionCheck {
            action: PermissionAction::NetworkAccess {
                target: target.original().to_string(),
                host: target.host().to_string(),
                port: target.port(),
            },
            decision: self.agent.authorize_network_connect(&target),
        });
        Ok(())
    }

    pub(crate) fn network_permission_check(
        &self,
        target: &str,
    ) -> Result<ToolPermissionCheck, ToolError> {
        let mut checks = Vec::with_capacity(1);
        self.push_network_check(&mut checks, target)?;
        Ok(checks.remove(0))
    }
}
use super::{
    AccessKind, NetworkTarget, Path, PathBuf, PermissionDecision, PermissionEnforcementMode,
    ShellOutput, ShellRequest, ToolError, ToolExecutor, ToolPermissionCheck, access_kind_name,
    canonicalize_path_for_execution, normalize_path_for_display,
    resolve_managed_project_path_alias, shell,
};
use agena_domain::PermissionAction;
use agena_domain::{FilesystemEffect, NetworkEffect};
