use anyhow::{Context, anyhow};
use serde_json::json;

impl Backend {
    pub fn plugin_statusline_segments(&self) -> Vec<agena::plugin::HostStatuslineSegment> {
        self.runtime
            .current_snapshot()
            .plugin_manager()
            .statusline_segments()
    }

    pub fn plugin_tui_content_blocks(
        &self,
    ) -> Vec<agena::plugin::PluginTuiContentBlockCatalogItem> {
        self.runtime
            .current_snapshot()
            .plugin_manager()
            .tui_content_blocks()
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn workspace_name(&self) -> String {
        self.workspace_root
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| self.workspace_root.display().to_string())
    }

    pub fn plugin_theme_palettes(&self) -> Vec<agena::plugin::HostThemePalette> {
        self.runtime
            .current_snapshot()
            .plugin_manager()
            .theme_palettes()
    }

    pub fn plugin_statuses(&self) -> Vec<agena::plugin::status::PluginStatus> {
        self.runtime
            .current_snapshot()
            .plugin_manager()
            .plugin_statuses()
    }

    pub fn plugin_inspect(&self, plugin_id: &str) -> Option<agena::plugin::PluginInspect> {
        self.runtime
            .current_snapshot()
            .plugin_manager()
            .plugin_inspect(plugin_id)
    }

    pub fn plugin_logs(
        &self,
        plugin_id: &str,
        after_seq: Option<u64>,
        limit: usize,
    ) -> Vec<agena::plugin::PluginLogRecord> {
        self.runtime
            .current_snapshot()
            .plugin_manager()
            .plugin_logs(plugin_id, after_seq, limit)
    }

    pub fn runtime_tool_rows(&self) -> Vec<InspectorRow> {
        let mut rows = self
            .runtime
            .current_snapshot()
            .plugin_manager()
            .registered_tools()
            .into_iter()
            .map(|entry| {
                let detail = format!(
                    "{} | {}",
                    entry.plugin_full_name(),
                    entry
                        .definition
                        .summary_text()
                        .or_else(|| entry.definition.help_text())
                        .unwrap_or("")
                );
                InspectorRow {
                    label: entry.model_name(),
                    detail,
                }
            })
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.label.cmp(&right.label));
        rows
    }

    pub async fn list_permission_rules(&self) -> Result<Vec<PermissionRuleResource>> {
        match dispatch::dispatch_query(
            &self.app_state,
            Query::ListPermissionRules(ListPermissionRulesParams {
                cursor: None,
                limit: Some(200),
                search: None,
            }),
        )
        .await
        .map_err(api_error)?
        {
            QueryResult::PermissionRules(page) => {
                let mut rules = page.items;
                rules.sort_by(|left, right| left.action_key.cmp(&right.action_key));
                Ok(rules)
            }
            other => Err(anyhow!("unexpected query result: {:?}", other)),
        }
        .context("failed to list permission rules")
    }

    pub async fn create_permission_rule(
        &self,
        params: UpsertPermissionRuleParams,
    ) -> Result<PermissionRuleResource> {
        match dispatch::dispatch_command(&self.app_state, ApiCommand::UpsertPermissionRule(params))
            .await
            .map_err(api_error)?
        {
            CommandResult::PermissionRule(rule) => Ok(rule),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to create permission rule")
    }

    pub async fn replace_permission_rule(
        &self,
        rule_id: i64,
        params: UpsertPermissionRuleParams,
    ) -> Result<PermissionRuleResource> {
        match dispatch::dispatch_command(
            &self.app_state,
            ApiCommand::ReplacePermissionRule(ReplacePermissionRuleParams {
                rule_id,
                rule: params,
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::PermissionRule(rule) => Ok(rule),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to replace permission rule")
    }

    pub async fn revoke_permission_rule(&self, rule_id: i64) -> Result<PermissionRuleResource> {
        match dispatch::dispatch_command(
            &self.app_state,
            ApiCommand::RevokePermissionRule(agena_api::commands::RevokePermissionRuleParams {
                rule_id,
                reason: None,
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::PermissionRule(rule) => Ok(rule),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to revoke permission rule")
    }

    pub fn snapshot_inspector_rows(&self) -> Vec<InspectorRow> {
        let Some(manager) = self.runtime.session_manager() else {
            return vec![InspectorRow {
                label: "session_runtime".to_string(),
                detail: "unavailable".to_string(),
            }];
        };
        let executor = manager.tool_executor();
        let Some(registry) = executor.snapshot_registry() else {
            return vec![InspectorRow {
                label: "snapshot_registry".to_string(),
                detail: "unavailable".to_string(),
            }];
        };
        let active = tool::snapshot_list_active(registry);
        let managed = tool::snapshot_list_managed(&self.workspace_root, registry);
        let capabilities = tool::snapshot_backend_capabilities(&self.workspace_root);
        let mut rows = vec![
            InspectorRow {
                label: "preferred_backend".to_string(),
                detail: capabilities
                    .preferred_backend
                    .map(|backend| backend.to_string())
                    .unwrap_or_else(|| "none".to_string()),
            },
            InspectorRow {
                label: "rift_backend".to_string(),
                detail: format!(
                    "available={} | {}",
                    capabilities.rift.available, capabilities.rift.detail
                ),
            },
            InspectorRow {
                label: "git_backend".to_string(),
                detail: format!(
                    "available={} | {}",
                    capabilities.git.available, capabilities.git.detail
                ),
            },
            InspectorRow {
                label: "active_sessions".to_string(),
                detail: active.len().to_string(),
            },
            InspectorRow {
                label: "managed_dirs".to_string(),
                detail: managed.len().to_string(),
            },
        ];
        rows.extend(active.into_iter().map(|entry| InspectorRow {
            label: format!("session #{}", entry.session_id),
            detail: format!(
                "{} | backend={} | branch={} | created_here={}",
                entry.path.display(),
                entry.backend,
                entry.branch,
                entry.created_here
            ),
        }));
        rows.extend(managed.into_iter().map(|entry| {
            let session_id = entry
                .session_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "none".to_string());
            let branch = entry
                .branch
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            let stale = entry.is_stale();
            InspectorRow {
                label: entry.path.display().to_string(),
                detail: format!(
                    "session={} | backend={} | branch={} | git_registered={} | rift_registered={} | stale={}",
                    session_id,
                    entry.backend
                        .map(|backend| backend.to_string())
                        .unwrap_or_else(|| "unknown".to_string()),
                    branch,
                    entry.registered_with_git,
                    entry.registered_with_rift,
                    stale
                ),
            }
        }));
        rows
    }

    pub fn enter_snapshot(
        &self,
        session_id: i64,
        name: Option<String>,
        path: Option<String>,
    ) -> Result<SnapshotCommandOutput> {
        let manager = self.session_manager()?;
        let output = manager
            .tool_executor()
            .execute_tool_payload_for_host(
                "enter_snapshot",
                serde_json::to_value(EnterSnapshotToolInput { name, path })?,
                Some(session_id),
                None,
                None,
            )
            .map_err(|error| anyhow!(error.to_string()))?;
        parse_snapshot_payload(output.payload)
    }

    pub fn exit_snapshot(
        &self,
        session_id: i64,
        action: String,
        discard_changes: bool,
    ) -> Result<SnapshotCommandOutput> {
        let manager = self.session_manager()?;
        let output = manager
            .tool_executor()
            .execute_tool_payload_for_host(
                "exit_snapshot",
                serde_json::to_value(ExitSnapshotToolInput {
                    action,
                    discard_changes,
                })?,
                Some(session_id),
                None,
                None,
            )
            .map_err(|error| anyhow!(error.to_string()))?;
        parse_snapshot_payload(output.payload)
    }

    pub fn runtime_tool_exists(&self, name: &str) -> bool {
        self.runtime
            .current_snapshot()
            .plugin_manager()
            .lookup_tool(name)
            .is_some()
    }

    pub fn runtime_tool_prompt(&self, session_id: i64, name: &str, args: &str) -> Result<String> {
        let manager = self.session_manager()?;
        let invocation = ToolInvocation::new(
            name.to_string(),
            serde_json::from_value::<agena::message::StructuredObject>(json!({
                "args": if args.trim().is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::Value::String(args.trim().to_string())
                }
            }))
            .map_err(|error| anyhow!(error))?,
        );
        let execution = manager
            .tool_executor()
            .execute_invocation_detailed(&invocation, session_id, -1)
            .map_err(|error| anyhow!(error.to_string()))?;
        Ok(execution.view.output_text)
    }

    pub async fn create_commit(&self, message: String) -> Result<(String, String)> {
        let status = self
            .git_status()
            .await
            .context("failed to load git status")?;
        if !status.git_available {
            return Err(anyhow!("git is not available in PATH"));
        }
        if !status.repo {
            return Err(anyhow!(
                "not a git repository: {}",
                self.workspace_root.display()
            ));
        }
        if status.staged_files == 0 {
            return Err(anyhow!("no staged changes to commit"));
        }

        let output = Command::new("git")
            .args(["commit", "-m", message.as_str()])
            .current_dir(&self.workspace_root)
            .output()
            .context("failed to execute git commit")?;
        if !output.status.success() {
            return Err(anyhow!(
                "git commit failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }

        let commit = git_command_output(&self.workspace_root, ["rev-parse", "HEAD"])?;
        let summary = git_command_output(&self.workspace_root, ["log", "-1", "--pretty=%s"])?;
        Ok((commit, summary))
    }

    pub async fn create_pr(
        &self,
        title: String,
        body: Option<String>,
        base: Option<String>,
        head: Option<String>,
    ) -> Result<String> {
        let status = self
            .git_status()
            .await
            .context("failed to load git status")?;
        if !status.git_available {
            return Err(anyhow!("git is not available in PATH"));
        }
        if !status.gh_available {
            return Err(anyhow!("gh is not available in PATH"));
        }
        if !status.repo {
            return Err(anyhow!(
                "not a git repository: {}",
                self.workspace_root.display()
            ));
        }

        let branch = head
            .clone()
            .or(status.branch.clone())
            .ok_or_else(|| anyhow!("could not determine current branch"))?;

        let mut command = Command::new("gh");
        command.arg("pr").arg("create").arg("--title").arg(title);
        command.arg("--body").arg(body.unwrap_or_default());
        if let Some(base) = base {
            command.arg("--base").arg(base);
        }
        command.arg("--head").arg(branch);
        command.current_dir(&self.workspace_root);

        let output = command.output().context("failed to execute gh pr create")?;
        if !output.status.success() {
            return Err(anyhow!(
                "gh pr create failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    pub(super) async fn resolve_workspace_resource(
        &self,
        create_if_missing: bool,
    ) -> Result<WorkspaceResource> {
        match dispatch::dispatch_command(
            &self.app_state,
            ApiCommand::ResolveWorkspace(agena_api::commands::ResolveWorkspaceParams {
                path: self.workspace_root.to_string_lossy().to_string(),
                create_if_missing,
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::Workspace(workspace) => Ok(workspace),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to resolve workspace")
    }

    pub(super) async fn git_status(&self) -> Result<GitStatusResource> {
        let workspace_root = self.runtime.workspace_root().to_path_buf();
        let git_available = command_available("git");
        let gh_available = command_available("gh");

        if self.runtime.session_manager().is_none() {
            return Ok(GitStatusResource {
                git_available,
                repo: false,
                gh_available,
                branch: None,
                staged_files: 0,
            });
        }

        if !git_available {
            return Ok(GitStatusResource {
                git_available,
                repo: false,
                gh_available,
                branch: None,
                staged_files: 0,
            });
        }

        let repo = git_success(&workspace_root, ["rev-parse", "--is-inside-work-tree"]);
        if !repo {
            return Ok(GitStatusResource {
                git_available,
                repo,
                gh_available,
                branch: None,
                staged_files: 0,
            });
        }

        let branch = git_command_output(&workspace_root, ["branch", "--show-current"])?;
        let status = git_command_output(&workspace_root, ["status", "--porcelain"])?;
        let (staged_files, _, _, _) = summarize_git_status(status.as_str());

        Ok(GitStatusResource {
            git_available,
            repo,
            gh_available,
            branch: non_empty(Some(branch.as_str())).map(ToOwned::to_owned),
            staged_files,
        })
    }

    pub(super) async fn current_workspace_id(&self) -> Result<i64> {
        Ok(self
            .resolve_workspace_resource(true)
            .await
            .context("failed to resolve current workspace")?
            .id)
    }

    pub(super) async fn list_sessions_query(
        &self,
        query: ListSessionsParams,
    ) -> Result<Vec<SessionResource>> {
        let mut cursor = query.cursor.clone();
        let limit = query.limit.unwrap_or(200);
        let mut items = Vec::new();

        loop {
            let page = match dispatch::dispatch_query(
                &self.app_state,
                Query::ListSessions(ListSessionsParams {
                    cursor: cursor.clone(),
                    limit: Some(limit),
                    workspace_id: query.workspace_id,
                    parent_id: query.parent_id,
                    roots: query.roots,
                    search: query.search.clone(),
                }),
            )
            .await
            .map_err(api_error)?
            {
                QueryResult::Sessions(page) => page,
                other => return Err(anyhow!("unexpected query result: {:?}", other)),
            };
            cursor = page.page.next_cursor.clone();
            items.extend(page.items);
            if !page.page.has_more || cursor.is_none() {
                break;
            }
        }

        Ok(items)
    }

    pub(super) async fn resolve_session_root(&self, session_id: i64) -> Result<SessionResource> {
        let mut current = self
            .get_session(session_id)
            .await?
            .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
        while let Some(parent_id) = current.parent_id {
            current = self
                .get_session(parent_id)
                .await?
                .ok_or_else(|| anyhow!("session not found: {parent_id}"))?;
        }
        Ok(current)
    }

    pub(super) fn session_manager(&self) -> Result<Arc<agena::session::SessionManager>> {
        self.runtime
            .session_manager()
            .ok_or_else(|| anyhow!("session runtime is not available"))
    }

    pub(super) fn memory_store(&self) -> MemoryStore {
        MemoryStore::for_workspace(&self.workspace_root)
    }
}
use crate::backend::Result;
use crate::backend::{
    ApiCommand, Arc, Backend, Command, CommandResult, EnterSnapshotToolInput,
    ExitSnapshotToolInput, GitStatusResource, InspectorRow, ListPermissionRulesParams,
    ListSessionsParams, MemoryStore, Path, PermissionRuleResource, Query, QueryResult,
    ReplacePermissionRuleParams, SessionResource, SnapshotCommandOutput, ToolInvocation,
    UpsertPermissionRuleParams, WorkspaceResource, api_error, command_available, dispatch,
    git_command_output, git_success, non_empty, parse_snapshot_payload, summarize_git_status, tool,
};
