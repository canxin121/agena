use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

use super::{
    ActiveSnapshotOutput, AgenaCli, Agent, AppError, ApplyArgs, ApplyOutput, ApplyPatchToolInput,
    AuthCommand, AuthListArgs, AuthListOutput, AuthSubcommand, Command, CommitArgs, CommitOutput,
    ConfigEnvironment, ConfigLoader, ConfigOutputFormat, ContinueArgs, CostArgs, CostOutput,
    DebugCommand, DebugMessageOutput, DebugSessionArgs, DebugSessionOutput, DebugSubcommand,
    DiagnosticsArgs, DiagnosticsConfigOutput, DiagnosticsEnvironmentOutput, DiagnosticsOutput,
    ExecArgs, ExecOutput, ForkArgs, GitArgs, GitOutput, ManagedSnapshotOutput, MemoryCommand,
    MemoryListArgs, MemoryListOutput, MemorySubcommand, MemorySummaryOutput, PartContent, PathBuf,
    PermissionPolicy, PermissionReply, PermissionsArgs, PermissionsListArgs, PermissionsOutput,
    PermissionsReplaceArgs, PermissionsReplyArgs, PermissionsRevokeArgs, PermissionsSubcommand,
    PermissionsWriteArgs, PrArgs, PrOutput, ResumeArgs, ReviewArgs, SessionCreateRequest,
    SessionExecutionRequest, SessionForkOutput, SessionForkRequest, SessionImportOutput,
    SessionListArgs, SessionListOutput, SessionListView, SessionOutput, SessionRunOptions,
    SessionUserMessageRequest, SessionsCommand, SessionsSubcommand, SnapshotArgs,
    SnapshotBackendSupportOutput, SnapshotCapabilitiesOutput, SnapshotOutput, StorageConfig,
    ToolExecutor, ToolPayloadInput, ToolPermissionPolicy, UsageArgs, WorkflowState, auth_summary,
    collect_git_preflight, ensure_memory_index_path, entities, filter_session_summaries_by_view,
    format_apply_output, format_debug_session_output, fs, git_output, init_schema,
    latest_event_seq, list_all_session_summaries, memory_record_name, memory_type_label,
    paginate_session_summaries, permission_reply_kind_from_arg, permission_rule_crud,
    permission_rule_output, permission_scope_from_arg, render_serialized,
    replace_permission_rule_from_args, resolve_continue_options, resolve_run_options,
    review_prompt, selected_session_id, session_detail, title_from_prompt, tracing_config,
    upsert_permission_rule_from_args, usage_stats_query_from_args,
};

impl AgenaCli {
    pub(super) fn render_apply_command(&self, args: ApplyArgs) -> Result<String, AppError> {
        let patch = fs::read_to_string(&args.patch_file)?;
        let workspace = args
            .workspace
            .clone()
            .map(Ok)
            .unwrap_or_else(std::env::current_dir)?;
        let plugins =
            crate::tool::default_tool_host(workspace.clone()).map_err(AppError::Config)?;
        let executor = ToolExecutor::new(
            workspace,
            Agent::new(
                "cli",
                PermissionPolicy::allow_all(),
                ToolPermissionPolicy::allow_all(),
            ),
            crate::agents::SubagentRegistry::default(),
            plugins,
            None,
            None,
            None,
            crate::plugin::ToolPresentationConfig::default(),
        );
        let input = ToolPayloadInput::ApplyPatch(ApplyPatchToolInput { patch }).into_invocation();
        let execution = executor
            .execute_invocation_detailed(&input, -1, -1)
            .map_err(|err| AppError::Config(err.to_string()))?;
        let patch = execution.apply_patch.ok_or_else(|| {
            AppError::Internal("apply_patch tool did not return patch metadata".to_owned())
        })?;
        if args.json {
            render_serialized(
                ConfigOutputFormat::Json,
                &ApplyOutput {
                    title: execution.view.title,
                    output_text: execution.view.output_text,
                    patch,
                },
            )
        } else {
            Ok(format_apply_output(&patch))
        }
    }

    pub(super) async fn render_auth_command<E>(
        &self,
        loader: &ConfigLoader<E>,
        command: AuthCommand,
    ) -> Result<String, AppError>
    where
        E: ConfigEnvironment,
    {
        let manager = self.auth_manager(loader)?;
        match command
            .command
            .unwrap_or(AuthSubcommand::List(AuthListArgs {
                format: ConfigOutputFormat::Json,
            })) {
            AuthSubcommand::List(args) => {
                let mut credentials = manager
                    .all()?
                    .into_iter()
                    .map(|(provider_id, auth)| auth_summary(provider_id, auth))
                    .collect::<Vec<_>>();
                credentials.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
                render_serialized(args.format, &AuthListOutput { credentials })
            }
        }
    }

    pub(super) fn render_memory_command(&self, command: MemoryCommand) -> Result<String, AppError> {
        match command
            .command
            .unwrap_or(MemorySubcommand::List(MemoryListArgs {
                workspace: None,
                format: ConfigOutputFormat::Json,
            })) {
            MemorySubcommand::List(args) => {
                let store = self.memory_store_for_workspace(args.workspace.as_ref())?;
                let entries = store
                    .list()
                    .map_err(|error| AppError::Config(error.to_string()))?;
                let memories = entries
                    .into_iter()
                    .map(|memory| MemorySummaryOutput {
                        file_name: memory.file_name.clone(),
                        name: memory_record_name(&memory),
                        description: memory.frontmatter.description.clone(),
                        memory_type: memory_type_label(memory.frontmatter.r#type),
                        path: memory.path.display().to_string(),
                    })
                    .collect::<Vec<_>>();
                render_serialized(
                    args.format,
                    &MemoryListOutput {
                        dir: store.dir().display().to_string(),
                        count: memories.len(),
                        memories,
                    },
                )
            }
            MemorySubcommand::Forget(args) => {
                self.memory_store_for_workspace(args.workspace.as_ref())?
                    .forget(args.name.as_str())
                    .map_err(|error| AppError::Config(error.to_string()))?;
                Ok(format!("forgot memory: {}", args.name))
            }
            MemorySubcommand::Edit(args) => {
                let store = self.memory_store_for_workspace(args.workspace.as_ref())?;
                let path = match args.name.as_deref() {
                    Some(name) => {
                        store
                            .get(name)
                            .map_err(|error| AppError::Config(error.to_string()))?
                            .path
                    }
                    None => ensure_memory_index_path(&store)?,
                };
                Ok(path.display().to_string())
            }
        }
    }

    pub(super) async fn render_sessions_command(
        &self,
        command: SessionsCommand,
    ) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let manager = runtime
            .session_manager()
            .ok_or_else(super::session_storage_error)?;
        match command
            .command
            .unwrap_or(SessionsSubcommand::List(SessionListArgs {
                limit: 20,
                offset: 0,
                view: SessionListView::All,
                anchor_session_id: None,
                format: ConfigOutputFormat::Json,
            })) {
            SessionsSubcommand::List(args) => {
                let sessions = list_all_session_summaries(manager.as_ref()).await?;
                let sessions =
                    filter_session_summaries_by_view(sessions, args.view, args.anchor_session_id)?;
                let sessions = paginate_session_summaries(sessions, args.offset, args.limit);
                render_serialized(args.format, &SessionListOutput { sessions })
            }
            SessionsSubcommand::Export(args) => {
                let bundle = manager.export_session_jsonl(args.session_id).await?;
                Ok(bundle)
            }
            SessionsSubcommand::Import(args) => {
                let bundle = match args.path {
                    Some(path) => std::fs::read_to_string(&path).map_err(|err| {
                        AppError::Internal(format!("read import bundle {}: {err}", path.display()))
                    })?,
                    None => {
                        use std::io::Read;
                        let mut buf = String::new();
                        std::io::stdin()
                            .read_to_string(&mut buf)
                            .map_err(|err| AppError::Internal(format!("read stdin: {err}")))?;
                        buf
                    }
                };
                let session = manager.import_session_jsonl(&bundle).await?;
                let latest_event_seq = latest_event_seq(&manager, session.id).await?;
                render_serialized(
                    args.format,
                    &SessionImportOutput {
                        session: session_detail(&session, latest_event_seq),
                    },
                )
            }
            SessionsSubcommand::Tree(args) => {
                let mut sessions = manager.list_session_tree(args.root_id).await?;
                if let Some(max_depth) = args.max_depth {
                    let root_depth = sessions.first().map(|first| first.depth).unwrap_or(0);
                    sessions.retain(|s| s.depth - root_depth <= max_depth);
                }
                if let Some(limit) = args.limit {
                    sessions.truncate(limit);
                }
                render_serialized(args.format, &SessionListOutput { sessions })
            }
        }
    }

    pub(super) async fn render_resume_command(&self, args: ResumeArgs) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let manager = runtime
            .session_manager()
            .ok_or_else(super::session_storage_error)?;
        let session_id = selected_session_id(&manager, args.session_id, args.last).await?;
        let session = if args.agent.is_some() {
            let agent_profile = args
                .agent
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
            let options = SessionRunOptions {
                model: super::resolve_default_model(&runtime)?,
                thinking_mode: None,
                speed_mode: None,
                verbosity: None,
                thinking: None,
                request_override: Default::default(),
                system: None,
                temperature: None,
                max_output_tokens: None,
                agent_profile,
            };
            manager
                .continue_session(SessionExecutionRequest::new(session_id, options))
                .await?
        } else {
            manager.get_session(session_id).await?
        };
        let latest_event_seq = latest_event_seq(&manager, session.id).await?;
        render_serialized(
            args.format,
            &SessionOutput {
                session: session_detail(&session, latest_event_seq),
            },
        )
    }

    pub(super) async fn render_cost_command(&self, args: CostArgs) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let manager = runtime
            .session_manager()
            .ok_or_else(super::session_storage_error)?;
        let session_id = selected_session_id(&manager, args.session_id, args.last).await?;
        let session = manager.get_session(session_id).await?;
        let latest_event_seq = latest_event_seq(&manager, session.id).await?;
        let output = CostOutput {
            session: session_detail(&session, latest_event_seq),
            summary: crate::session::cost::summarize(&session.messages),
        };
        render_serialized(args.format, &output)
    }

    pub(super) async fn render_usage_command(&self, args: UsageArgs) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let manager = runtime
            .session_manager()
            .ok_or_else(super::session_storage_error)?;
        let query = usage_stats_query_from_args(&args)?;
        let output = manager.usage_stats(query).await?;
        render_serialized(args.format, &output)
    }

    pub(super) async fn render_permissions_command(
        &self,
        args: PermissionsArgs,
    ) -> Result<String, AppError> {
        match args
            .command
            .unwrap_or(PermissionsSubcommand::List(PermissionsListArgs {
                search: None,
                format: ConfigOutputFormat::Json,
            })) {
            PermissionsSubcommand::List(args) => self.render_permissions_list_command(args).await,
            PermissionsSubcommand::Create(args) => {
                self.render_permissions_create_command(args).await
            }
            PermissionsSubcommand::Replace(args) => {
                self.render_permissions_replace_command(args).await
            }
            PermissionsSubcommand::Revoke(args) => {
                self.render_permissions_revoke_command(args).await
            }
            PermissionsSubcommand::Reply(args) => self.render_permissions_reply_command(args).await,
        }
    }

    pub(super) async fn render_permissions_list_command(
        &self,
        args: PermissionsListArgs,
    ) -> Result<String, AppError> {
        let storage = StorageConfig {
            database_url: self.database_url.clone(),
            database_path: self.database_path.clone(),
        };
        let database_url = storage.resolve_url()?;
        StorageConfig::ensure_parent(database_url.as_str())?;
        let db = tracing_config::connect_database(
            database_url.as_str(),
            &self.resolved_tracing_config(),
        )
        .await?;
        init_schema(&db).await?;

        let mut query = entities::permission_rule::Entity::find()
            .order_by_desc(entities::permission_rule::Column::UpdatedAtMs)
            .order_by_desc(entities::permission_rule::Column::Id);
        if let Some(search) = args
            .search
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            query = query
                .filter(entities::permission_rule::Column::ActionKey.like(format!("%{search}%")));
        }
        let rows = query.all(&db).await?;
        let rules = rows
            .into_iter()
            .map(permission_rule_output)
            .collect::<Result<Vec<_>, AppError>>()?;
        render_serialized(
            args.format,
            &PermissionsOutput {
                count: rules.len(),
                rules,
            },
        )
    }

    pub(super) async fn render_permissions_create_command(
        &self,
        args: PermissionsWriteArgs,
    ) -> Result<String, AppError> {
        let storage = StorageConfig {
            database_url: self.database_url.clone(),
            database_path: self.database_path.clone(),
        };
        let database_url = storage.resolve_url()?;
        StorageConfig::ensure_parent(database_url.as_str())?;
        let db = tracing_config::connect_database(
            database_url.as_str(),
            &self.resolved_tracing_config(),
        )
        .await?;
        init_schema(&db).await?;
        let workspace_root = self.resolve_workspace_root(None)?;
        let created =
            upsert_permission_rule_from_args(&db, workspace_root.as_path(), &args).await?;
        render_serialized(args.format, &created)
    }

    pub(super) async fn render_permissions_replace_command(
        &self,
        args: PermissionsReplaceArgs,
    ) -> Result<String, AppError> {
        let storage = StorageConfig {
            database_url: self.database_url.clone(),
            database_path: self.database_path.clone(),
        };
        let database_url = storage.resolve_url()?;
        StorageConfig::ensure_parent(database_url.as_str())?;
        let db = tracing_config::connect_database(
            database_url.as_str(),
            &self.resolved_tracing_config(),
        )
        .await?;
        init_schema(&db).await?;
        let workspace_root = self.resolve_workspace_root(None)?;
        let updated = replace_permission_rule_from_args(
            &db,
            workspace_root.as_path(),
            args.rule_id,
            &args.rule,
        )
        .await?;
        render_serialized(args.rule.format, &updated)
    }

    pub(super) async fn render_permissions_revoke_command(
        &self,
        args: PermissionsRevokeArgs,
    ) -> Result<String, AppError> {
        let storage = StorageConfig {
            database_url: self.database_url.clone(),
            database_path: self.database_path.clone(),
        };
        let database_url = storage.resolve_url()?;
        StorageConfig::ensure_parent(database_url.as_str())?;
        let db = tracing_config::connect_database(
            database_url.as_str(),
            &self.resolved_tracing_config(),
        )
        .await?;
        init_schema(&db).await?;
        let updated = permission_rule_crud::revoke_rule(
            &db,
            args.rule_id,
            args.reason,
            Some("cli".to_string()),
        )
        .await?;
        let Some(updated) = updated else {
            return Err(AppError::Config(format!(
                "permission rule not found: {}",
                args.rule_id
            )));
        };
        render_serialized(args.format, &permission_rule_output(updated)?)
    }

    pub(super) async fn render_permissions_reply_command(
        &self,
        args: PermissionsReplyArgs,
    ) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let manager = runtime
            .session_manager()
            .ok_or_else(super::session_storage_error)?;
        let session_id = selected_session_id(&manager, args.session_id, args.last).await?;
        let session = manager
            .reply_permission(crate::session::SessionPermissionReplyRequest::new(
                session_id,
                resolve_run_options(&runtime, None, None, None, None)?,
                PermissionReply {
                    request_id: args.request_id,
                    kind: permission_reply_kind_from_arg(args.kind),
                    reason: args.reason,
                    scope: args.scope.map(permission_scope_from_arg),
                },
                Some("cli".to_string()),
            ))
            .await?;
        let latest_event_seq = latest_event_seq(&manager, session.id).await?;
        render_serialized(
            args.format,
            &SessionOutput {
                session: session_detail(&session, latest_event_seq),
            },
        )
    }

    pub(super) async fn render_snapshot_command(
        &self,
        args: SnapshotArgs,
    ) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let manager = runtime
            .session_manager()
            .ok_or_else(super::session_storage_error)?;
        let executor = manager.tool_executor();
        let registry = executor.snapshot_registry().ok_or_else(|| {
            AppError::Config("snapshot registry is not enabled in this runtime".to_owned())
        })?;
        let capabilities = crate::tool::snapshot_backend_capabilities(runtime.workspace_root());
        let active = crate::tool::snapshot_list_active(registry)
            .into_iter()
            .map(|entry| ActiveSnapshotOutput {
                session_id: entry.session_id,
                path: entry.path.display().to_string(),
                branch: entry.branch,
                backend: entry.backend.to_string(),
                created_here: entry.created_here,
            })
            .collect::<Vec<_>>();
        let managed = crate::tool::snapshot_list_managed(runtime.workspace_root(), registry)
            .into_iter()
            .map(|entry: crate::tool::ManagedSnapshot| {
                let stale = entry.is_stale();
                ManagedSnapshotOutput {
                    path: entry.path.display().to_string(),
                    session_id: entry.session_id,
                    branch: entry.branch,
                    backend: entry
                        .backend
                        .map(|backend: crate::tool::SnapshotBackend| backend.to_string()),
                    registered_with_git: entry.registered_with_git,
                    registered_with_rift: entry.registered_with_rift,
                    stale,
                }
            })
            .collect::<Vec<_>>();
        render_serialized(
            args.format,
            &SnapshotOutput {
                workspace_root: runtime.workspace_root().display().to_string(),
                capabilities: SnapshotCapabilitiesOutput {
                    preferred_backend: capabilities
                        .preferred_backend
                        .map(|backend: crate::tool::SnapshotBackend| backend.to_string()),
                    git: SnapshotBackendSupportOutput {
                        available: capabilities.git.available,
                        detail: capabilities.git.detail,
                    },
                    rift: SnapshotBackendSupportOutput {
                        available: capabilities.rift.available,
                        detail: capabilities.rift.detail,
                    },
                },
                active,
                managed,
            },
        )
    }

    pub(super) async fn render_git_command(&self, args: GitArgs) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let workspace_root = runtime.workspace_root().to_path_buf();
        let preflight = collect_git_preflight(&workspace_root)?;

        let (snapshot_active_sessions, snapshot_managed_dirs) = match runtime.session_manager() {
            Some(manager) => {
                let executor = manager.tool_executor();
                match executor.snapshot_registry() {
                    Some(registry) => (
                        crate::tool::snapshot_list_active(registry).len() as u64,
                        crate::tool::snapshot_list_managed(runtime.workspace_root(), registry).len()
                            as u64,
                    ),
                    None => (0, 0),
                }
            }
            None => (0, 0),
        };

        render_serialized(
            args.format,
            &GitOutput {
                workspace_root: workspace_root.display().to_string(),
                git_available: preflight.git_available,
                repo: preflight.repo,
                gh_available: preflight.gh_available,
                branch: preflight.branch,
                upstream: preflight.upstream,
                ahead: preflight.ahead,
                behind: preflight.behind,
                staged_files: preflight.staged_files,
                unstaged_files: preflight.unstaged_files,
                untracked_files: preflight.untracked_files,
                changed_files: preflight.changed_files,
                clean: preflight.clean,
                snapshot_active_sessions,
                snapshot_managed_dirs,
            },
        )
    }

    pub(super) fn render_commit_command(&self, args: CommitArgs) -> Result<String, AppError> {
        let workspace_root = self.resolve_workspace_root(None)?;
        let preflight = collect_git_preflight(&workspace_root)?;
        if !preflight.git_available {
            return Err(AppError::Config("git is not available in PATH".to_owned()));
        }
        if !preflight.repo {
            return Err(AppError::Config(format!(
                "not a git repository: {}",
                workspace_root.display()
            )));
        }
        if preflight.staged_files == 0 {
            return Err(AppError::Config("no staged changes to commit".to_owned()));
        }
        let output = Command::new("git")
            .args(["commit", "-m", args.message.as_str()])
            .current_dir(&workspace_root)
            .output()?;
        if !output.status.success() {
            return Err(AppError::Config(format!(
                "git commit failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        let commit = git_output(&workspace_root, ["rev-parse", "HEAD"])?;
        let summary = git_output(&workspace_root, ["log", "-1", "--pretty=%s"])?;
        render_serialized(
            args.format,
            &CommitOutput {
                workspace_root: workspace_root.display().to_string(),
                commit,
                summary,
            },
        )
    }

    pub(super) fn render_pr_command(&self, args: PrArgs) -> Result<String, AppError> {
        let workspace_root = self.resolve_workspace_root(None)?;
        let preflight = collect_git_preflight(&workspace_root)?;
        if !preflight.git_available {
            return Err(AppError::Config("git is not available in PATH".to_owned()));
        }
        if !preflight.gh_available {
            return Err(AppError::Config("gh is not available in PATH".to_owned()));
        }
        if !preflight.repo {
            return Err(AppError::Config(format!(
                "not a git repository: {}",
                workspace_root.display()
            )));
        }
        let branch = args
            .head
            .clone()
            .or(preflight.branch.clone())
            .ok_or_else(|| AppError::Config("could not determine current branch".to_owned()))?;

        let mut command = Command::new("gh");
        command
            .arg("pr")
            .arg("create")
            .arg("--title")
            .arg(args.title);
        command.arg("--body").arg(args.body.unwrap_or_default());
        if let Some(base) = args.base {
            command.arg("--base").arg(base);
        }
        if let Some(head) = args.head {
            command.arg("--head").arg(head);
        }
        command.current_dir(&workspace_root);

        let output = command.output()?;
        if !output.status.success() {
            return Err(AppError::Config(format!(
                "gh pr create failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        render_serialized(
            args.format,
            &PrOutput {
                workspace_root: workspace_root.display().to_string(),
                branch,
                url,
            },
        )
    }

    pub(super) async fn render_continue_command(
        &self,
        args: ContinueArgs,
    ) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let manager = runtime
            .session_manager()
            .ok_or_else(super::session_storage_error)?;
        let session_id = selected_session_id(&manager, args.session_id, args.last).await?;
        let session = manager.get_session(session_id).await?;
        let options = resolve_continue_options(&runtime, &session, &args)?;
        let session = manager
            .continue_session(SessionExecutionRequest::new(session_id, options))
            .await?;
        let latest_event_seq = latest_event_seq(&manager, session.id).await?;
        render_serialized(
            args.format,
            &SessionOutput {
                session: session_detail(&session, latest_event_seq),
            },
        )
    }

    pub(super) async fn render_debug_command(
        &self,
        command: DebugCommand,
    ) -> Result<String, AppError> {
        match command.command {
            DebugSubcommand::Session(args) => self.render_debug_session_command(args).await,
        }
    }

    pub(super) async fn render_debug_session_command(
        &self,
        args: DebugSessionArgs,
    ) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let manager = runtime
            .session_manager()
            .ok_or_else(super::session_storage_error)?;
        let session = manager.get_session(args.session_id).await?;
        let latest_event_seq = latest_event_seq(&manager, session.id).await?;
        let output = DebugSessionOutput {
            session: session_detail(&session, latest_event_seq),
            messages: session
                .messages
                .iter()
                .map(|message| DebugMessageOutput {
                    id: message.id,
                    role: message.role,
                    state: message.state,
                    text: message.visible_text_lossy(),
                })
                .collect(),
        };
        if args.json {
            render_serialized(ConfigOutputFormat::Json, &output)
        } else {
            Ok(format_debug_session_output(&output))
        }
    }

    pub(super) fn render_diagnostics_command<E>(
        &self,
        loader: &ConfigLoader<E>,
        args: DiagnosticsArgs,
    ) -> Result<String, AppError>
    where
        E: ConfigEnvironment,
    {
        let resolution = loader.load(&self.load_request())?;
        let config = &resolution.config;
        render_serialized(
            args.format,
            &DiagnosticsOutput {
                version: env!("CARGO_PKG_VERSION"),
                os: std::env::consts::OS.to_owned(),
                arch: std::env::consts::ARCH.to_owned(),
                current_dir: std::env::current_dir()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|_| "<unavailable>".to_owned()),
                config: DiagnosticsConfigOutput {
                    path: resolution.meta.config_path.display().to_string(),
                    found: resolution.meta.config_found,
                    project_path: resolution.meta.project_config_path.display().to_string(),
                    project_found: resolution.meta.project_config_found,
                    applied_layers: resolution
                        .meta
                        .applied_layers
                        .iter()
                        .map(|layer| layer.description.clone())
                        .collect(),
                    provider_count: config.providers.len(),
                    plugin_count: config.plugins.list.len(),
                },
                environment: DiagnosticsEnvironmentOutput {
                    agena_database_url_set: std::env::var_os("AGENA_DATABASE_URL").is_some(),
                    agena_database_path_set: std::env::var_os("AGENA_DATABASE_PATH").is_some(),
                    agena_adapter_log_set: std::env::var_os("AGENA_ADAPTER_LOG").is_some(),
                },
            },
        )
    }

    pub(super) async fn render_exec_command(&self, args: ExecArgs) -> Result<String, AppError> {
        self.render_prompt_command(
            args.workspace.as_ref(),
            args.prompt.as_str(),
            title_from_prompt(args.prompt.as_str()),
            args.model.as_deref(),
            args.agent.as_deref(),
            args.temperature,
            args.max_output_tokens,
            args.json,
        )
        .await
    }

    pub(super) async fn render_review_command(&self, args: ReviewArgs) -> Result<String, AppError> {
        let prompt = review_prompt(args.base.as_str());
        self.render_prompt_command(
            args.workspace.as_ref(),
            prompt.as_str(),
            format!("Review changes against {}", args.base),
            args.model.as_deref(),
            args.agent.as_deref(),
            args.temperature,
            args.max_output_tokens,
            args.json,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn render_prompt_command(
        &self,
        workspace: Option<&PathBuf>,
        prompt: &str,
        title: String,
        model: Option<&str>,
        agent_profile: Option<&str>,
        temperature: Option<f32>,
        max_output_tokens: Option<u32>,
        json: bool,
    ) -> Result<String, AppError> {
        let runtime = self.session_runtime_with_workspace(workspace).await?;
        let manager = runtime
            .session_manager()
            .ok_or_else(super::session_storage_error)?;
        let options = resolve_run_options(
            &runtime,
            model,
            agent_profile,
            temperature,
            max_output_tokens,
        )?;
        let created = manager
            .create_session(SessionCreateRequest {
                title,
                parent_session_id: None,
            })
            .await?;
        let session = manager
            .submit_user_message(SessionUserMessageRequest::new(
                created.id,
                options,
                vec![PartContent::text(prompt)],
            ))
            .await?;
        if session.runtime.workflow.state == WorkflowState::Blocked {
            return Err(AppError::Config(
                "command is blocked awaiting permission or user input".to_owned(),
            ));
        }
        let latest_event_seq = latest_event_seq(&manager, session.id).await?;
        let text = session.last_assistant_text().unwrap_or_default();
        if json {
            render_serialized(
                ConfigOutputFormat::Json,
                &ExecOutput {
                    session: session_detail(&session, latest_event_seq),
                    text,
                },
            )
        } else {
            Ok(text)
        }
    }

    pub(super) async fn render_fork_command(&self, args: ForkArgs) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let manager = runtime
            .session_manager()
            .ok_or_else(super::session_storage_error)?;
        let forked = manager
            .fork_session(SessionForkRequest {
                session_id: args.session_id,
                at_message_id: args.at_message,
                title: args.title,
                expected_version: None,
            })
            .await?;
        let latest_event_seq = latest_event_seq(&manager, forked.id).await?;
        render_serialized(
            args.format,
            &SessionForkOutput {
                source_session_id: args.session_id,
                forked: session_detail(&forked, latest_event_seq),
            },
        )
    }
}
