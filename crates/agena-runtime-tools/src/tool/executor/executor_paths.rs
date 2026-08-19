impl ToolExecutor {
    pub fn resolve_target_path(&self, raw_path: &str) -> PathBuf {
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
        canonicalize_path_for_execution(&workspace_root.join(candidate))
    }

    pub(crate) async fn execute_shell_command(
        &self,
        request: &ShellRequest,
        command_text: &str,
        session_id: Option<i64>,
        call_id: Option<i64>,
    ) -> Result<ShellOutput, ToolError> {
        let Some(sink) = self.command_event_sink.clone() else {
            return match shell::execute(request, self.cancellation_token()).await {
                Err(agena_tool::ShellError::Cancelled) => Err(ToolError::Cancelled),
                result => result.map_err(ToolError::from),
            };
        };

        let context = agena_domain::CommandContext {
            session_id: session_id.unwrap_or(-1),
            call_id: call_id.unwrap_or(-1),
            message_id: None,
            part_id: None,
            activity_id: None,
        };
        let now = || chrono::Utc::now().timestamp_millis();
        (sink)(agena_tool::ToolRuntimeEvent::CommandBegin(
            agena_domain::CommandBeginEvent {
                context: context.clone(),
                command: command_text.to_owned(),
                cwd: request.cwd.display().to_string(),
                argv: request.command.clone(),
                is_user_initiated: false,
                ts_ms: now(),
            },
        ));

        let output_sequence = portable_atomic::AtomicU64::new(0);
        let output_callback = |stream: agena_domain::CommandOutputStream, bytes: &[u8]| {
            let sequence = output_sequence.fetch_add(1, std::sync::atomic::Ordering::AcqRel) + 1;
            (sink)(agena_tool::ToolRuntimeEvent::CommandOutputDelta(
                agena_domain::CommandOutputDeltaEvent {
                    context: context.clone(),
                    stream,
                    seq: sequence,
                    ts_ms: now(),
                    chunk: bytes.to_vec(),
                    preview_text: String::from_utf8_lossy(bytes).into_owned(),
                    preview_lossy: std::str::from_utf8(bytes).is_err(),
                },
            ));
        };

        let result = shell::execute_with_callback(
            request,
            self.cancellation_token(),
            Some(&output_callback),
        )
        .await;
        match &result {
            Ok(output) => {
                (sink)(agena_tool::ToolRuntimeEvent::CommandEnd(
                    agena_domain::CommandEndEvent {
                        context: context.clone(),
                        status: if output.timed_out {
                            agena_domain::ExecutionStatus::Failed
                        } else {
                            agena_domain::ExecutionStatus::Completed
                        },
                        exit_code: output.exit_code,
                        duration_ms: output.duration.as_millis() as u64,
                        stdout: output.stdout.clone(),
                        stderr: output.stderr.clone(),
                        aggregated_output: output.aggregated_output.clone(),
                        ts_ms: now(),
                    },
                ));
            }
            Err(error) => {
                (sink)(agena_tool::ToolRuntimeEvent::CommandEnd(
                    agena_domain::CommandEndEvent {
                        context: context.clone(),
                        status: if matches!(error, &agena_tool::ShellError::Cancelled) {
                            agena_domain::ExecutionStatus::Cancelled
                        } else {
                            agena_domain::ExecutionStatus::Failed
                        },
                        exit_code: -1,
                        duration_ms: 0,
                        stdout: String::new(),
                        stderr: String::new(),
                        aggregated_output: String::new(),
                        ts_ms: now(),
                    },
                ));
            }
        }
        match result {
            Err(agena_tool::ShellError::Cancelled) => Err(ToolError::Cancelled),
            result => result.map_err(ToolError::from),
        }
    }

    pub(crate) fn effective_workspace_root<'a>(
        &'a self,
        session_context: Option<&'a dyn crate::ToolSessionContext>,
    ) -> &'a Path {
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
            decision: self.principal.authorize_path_access(
                access,
                &canonical_workspace_root,
                &canonical_target_path,
            ),
            contract: agena_domain::ToolPermissionContract::default(),
        });
    }

    pub(crate) fn push_network_check(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        target: &str,
    ) -> Result<(), ToolError> {
        let target: NetworkTarget = target.parse().map_err(|err| {
            ToolError::invalid_input(format!(
                "invalid network permission target `{target}`: {err}"
            ))
        })?;
        checks.push(ToolPermissionCheck {
            action: PermissionAction::NetworkAccess {
                target: target.original().to_string(),
                host: target.host().to_string(),
                port: target.port(),
            },
            decision: self.principal.authorize_network_connect(&target),
            contract: agena_domain::ToolPermissionContract::default(),
        });
        Ok(())
    }
}
use super::{
    AccessKind, NetworkTarget, Path, PathBuf, ShellOutput, ShellRequest, ToolError, ToolExecutor,
    ToolPermissionCheck, access_kind_name, canonicalize_path_for_execution,
    normalize_path_for_display, resolve_managed_project_path_alias, shell,
};
use agena_domain::PermissionAction;
