use agena_application::Application;

use super::{
    ActiveSnapshotOutput, AgenaCli, AppError, ApplyArgs, ApplyOutput, AuthCommand, AuthListArgs,
    AuthListOutput, AuthSubcommand, Command, CommitArgs, CommitOutput, ContinueArgs, CostArgs,
    CostOutput, DebugCommand, DebugMessageOutput, DebugSessionArgs, DebugSessionOutput,
    DebugSubcommand, DiagnosticsArgs, DiagnosticsConfigOutput, DiagnosticsEnvironmentOutput,
    DiagnosticsOutput, ExecArgs, ExecOutput, ForkArgs, GitArgs, GitOutput, ManagedSnapshotOutput,
    MemoryCommand, MemoryListArgs, MemoryListOutput, MemorySubcommand, MemorySummaryOutput,
    OutputFormat, PathBuf, PermissionReply, PermissionsArgs, PermissionsListArgs,
    PermissionsOutput, PermissionsReplaceArgs, PermissionsReplyArgs, PermissionsRevokeArgs,
    PermissionsSubcommand, PermissionsWriteArgs, PrArgs, PrOutput, ResumeArgs, ReviewArgs,
    SessionCreateRequest, SessionExecutionRequest, SessionForkOutput, SessionForkRequest,
    SessionImportOutput, SessionListArgs, SessionListOutput, SessionListView, SessionOutput,
    SessionPermissionReplyRequest, SessionRunOptions, SessionUserMessagePart,
    SessionUserMessageRequest, SessionsCommand, SessionsSubcommand, SnapshotArgs,
    SnapshotBackendSupportOutput, SnapshotCapabilitiesOutput, SnapshotOutput, TextPart, UsageArgs,
    WorkflowState, application_from_runtime, auth_summary, collect_git_preflight, default_model,
    filter_session_summaries_by_view, format_apply_output, format_debug_session_output, fs,
    git_output, last_assistant_text_from_projection, latest_event_seq, list_all_session_summaries,
    list_permission_rules, memory_type_label, paginate_session_summaries,
    permission_reply_kind_from_arg, permission_rule_output,
    permission_rule_write_command_from_args, permission_scope_from_arg,
    projected_message_visible_text, render_serialized, resolve_continue_options,
    resolve_run_options, review_prompt, selected_session_id, session_detail_from_presentation,
    session_storage_error, title_from_prompt, usage_stats_query_from_args,
};

impl AgenaCli {
    pub(super) async fn render_apply_command(&self, args: ApplyArgs) -> Result<String, AppError> {
        let patch = fs::read_to_string(&args.patch_file)?;
        let input = agena_domain::ToolInvocation::new(
            "apply_patch",
            agena_domain::StructuredObject::try_from(serde_json::json!({ "patch": patch }))
                .map_err(|error| AppError::Config(error.to_string()))?,
        );
        self.with_session_runtime_services(|services| async move {
            let summary = services
                .tools
                .execute_runtime_tool(&input, -1, -1)
                .map_err(|err| AppError::Config(err.to_string()))?;
            let patch_payload = summary.payload.ok_or_else(|| {
                AppError::Internal("apply_patch tool did not return patch metadata".to_owned())
            })?;
            let patch = serde_json::from_value(patch_payload).map_err(|error| {
                AppError::Internal(format!(
                    "apply_patch tool returned invalid patch metadata: {error}"
                ))
            })?;
            if args.json {
                render_serialized(
                    OutputFormat::Json,
                    &ApplyOutput {
                        title: summary.title,
                        output_text: summary.output_text,
                        patch,
                    },
                )
            } else {
                Ok(format_apply_output(&patch))
            }
        })
        .await
    }

    pub(super) async fn render_auth_command(
        &self,
        command: AuthCommand,
    ) -> Result<String, AppError> {
        self.with_application(|application| async move {
            match command
                .command
                .unwrap_or(AuthSubcommand::List(AuthListArgs {
                    format: OutputFormat::Json,
                })) {
                AuthSubcommand::List(args) => {
                    let mut credentials = application
                        .auth_providers()
                        .map_err(|error| AppError::Config(error.to_string()))?
                        .into_iter()
                        .map(auth_summary)
                        .collect::<Vec<_>>();
                    credentials.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
                    render_serialized(args.format, &AuthListOutput { credentials })
                }
            }
        })
        .await
    }

    pub(super) async fn render_memory_command(
        &self,
        command: MemoryCommand,
    ) -> Result<String, AppError> {
        let command = command
            .command
            .unwrap_or(MemorySubcommand::List(MemoryListArgs {
                workspace: None,
                format: OutputFormat::Json,
            }));
        let workspace = match &command {
            MemorySubcommand::List(args) => args.workspace.as_ref(),
            MemorySubcommand::Forget(args) => args.workspace.as_ref(),
            MemorySubcommand::Edit(args) => args.workspace.as_ref(),
        };
        let runtime = self.session_runtime_with_workspace(workspace).await?;
        let result = (|| {
            let application =
                Application::from_composed_runtime_services(runtime.application_services())
                    .map_err(|error| AppError::Internal(error.to_string()))?;
            match command {
                MemorySubcommand::List(args) => {
                    let entries = application
                        .service()
                        .list_memories()
                        .map_err(|error| AppError::Config(error.to_string()))?;
                    let memories = entries
                        .into_iter()
                        .map(|memory| MemorySummaryOutput {
                            file_name: memory.file_name,
                            name: memory.name,
                            description: memory.description,
                            memory_type: memory_type_label(memory.memory_type),
                            path: memory.path,
                        })
                        .collect::<Vec<_>>();
                    render_serialized(
                        args.format,
                        &MemoryListOutput {
                            dir: application
                                .service()
                                .memory_directory()
                                .display()
                                .to_string(),
                            count: memories.len(),
                            memories,
                        },
                    )
                }
                MemorySubcommand::Forget(args) => {
                    application
                        .service()
                        .forget_memory(args.name.as_str())
                        .map_err(|error| AppError::Config(error.to_string()))?;
                    Ok(format!("forgot memory: {}", args.name))
                }
                MemorySubcommand::Edit(args) => {
                    let path = match args.name.as_deref() {
                        Some(name) => application
                            .service()
                            .memory_entry_path(name)
                            .map_err(|error| AppError::Config(error.to_string()))?,
                        None => application
                            .service()
                            .memory_index_path()
                            .map_err(|error| AppError::Config(error.to_string()))?,
                    };
                    Ok(path.display().to_string())
                }
            }
        })();
        runtime.shutdown();
        result
    }

    pub(super) async fn render_sessions_command(
        &self,
        command: SessionsCommand,
    ) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let services = runtime.application_services();
        let queries = services.session_queries.ok_or_else(session_storage_error)?;
        let commands = services
            .execution_commands
            .ok_or_else(session_storage_error)?;
        match command
            .command
            .unwrap_or(SessionsSubcommand::List(SessionListArgs {
                limit: 20,
                offset: 0,
                view: SessionListView::All,
                anchor_session_id: None,
                format: OutputFormat::Json,
            })) {
            SessionsSubcommand::List(args) => {
                let sessions = list_all_session_summaries(queries.as_ref()).await?;
                let sessions =
                    filter_session_summaries_by_view(sessions, args.view, args.anchor_session_id)?;
                let sessions = paginate_session_summaries(sessions, args.offset, args.limit);
                render_serialized(args.format, &SessionListOutput { sessions })
            }
            SessionsSubcommand::Export(args) => {
                let bundle = queries
                    .export_session_jsonl(args.session_id)
                    .await
                    .map_err(|error| AppError::Internal(error.to_string()))?;
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
                let outcome = commands
                    .import_session_jsonl(&bundle)
                    .await
                    .map_err(|error| AppError::Internal(error.to_string()))?;
                let session = queries
                    .session_presentation(outcome.session_id)
                    .await
                    .map_err(|error| AppError::Internal(error.to_string()))?;
                let latest_event_seq = latest_event_seq(queries.as_ref(), session.id).await?;
                render_serialized(
                    args.format,
                    &SessionImportOutput {
                        session: session_detail_from_presentation(session, latest_event_seq),
                    },
                )
            }
            SessionsSubcommand::Tree(args) => {
                let mut sessions = queries
                    .list_session_tree(args.root_id)
                    .await
                    .map_err(|error| AppError::Internal(error.to_string()))?;
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
        let providers = runtime.application_services().provider_catalog;
        let services = runtime.application_services();
        let queries = services.session_queries.ok_or_else(session_storage_error)?;
        let commands = services
            .execution_commands
            .ok_or_else(session_storage_error)?;
        let session_id = selected_session_id(queries.as_ref(), args.session_id, args.last).await?;
        let output_session_id = if args.agent.is_some() {
            let agent_profile = args
                .agent
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
            let options = SessionRunOptions {
                model: default_model(providers.as_ref())?,
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
            commands
                .continue_session(SessionExecutionRequest::new(session_id, options))
                .await
                .map_err(|error| AppError::Internal(error.to_string()))?
                .session_id
        } else {
            session_id
        };
        let session = queries
            .session_presentation(output_session_id)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let latest_event_seq = latest_event_seq(queries.as_ref(), session.id).await?;
        render_serialized(
            args.format,
            &SessionOutput {
                session: session_detail_from_presentation(session, latest_event_seq),
            },
        )
    }

    pub(super) async fn render_cost_command(&self, args: CostArgs) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let services = runtime.application_services();
        let queries = services.session_queries.ok_or_else(session_storage_error)?;
        let session_id = selected_session_id(queries.as_ref(), args.session_id, args.last).await?;
        let session = queries
            .session_presentation(session_id)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let latest_event_seq = latest_event_seq(queries.as_ref(), session.id).await?;
        let output = CostOutput {
            session: session_detail_from_presentation(session, latest_event_seq),
            summary: queries
                .session_cost_summary(session_id)
                .await
                .map_err(|error| AppError::Internal(error.to_string()))?,
        };
        render_serialized(args.format, &output)
    }

    pub(super) async fn render_usage_command(&self, args: UsageArgs) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let services = runtime.application_services();
        let queries = services.session_queries.ok_or_else(session_storage_error)?;
        let query = usage_stats_query_from_args(&args)?;
        let output = queries
            .usage_stats(query)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
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
                format: OutputFormat::Json,
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
        let runtime = self.session_runtime().await?;
        let application = application_from_runtime(&runtime)?;
        let result = list_permission_rules(
            &application,
            args.search
                .as_deref()
                .map(str::trim)
                .filter(|search| !search.is_empty())
                .map(ToOwned::to_owned),
        )
        .await;
        runtime.shutdown();
        let rules = result?;
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
        let workspace_root = self.resolve_workspace_root(None)?;
        let runtime = self.session_runtime().await?;
        let application = application_from_runtime(&runtime)?;
        let command = permission_rule_write_command_from_args(workspace_root.as_path(), &args)?;
        let result = application
            .service()
            .create_permission_rule_command(command)
            .await
            .map_err(|error| AppError::Internal(error.to_string()));
        runtime.shutdown();
        let created = permission_rule_output(result?)?;
        render_serialized(args.format, &created)
    }

    pub(super) async fn render_permissions_replace_command(
        &self,
        args: PermissionsReplaceArgs,
    ) -> Result<String, AppError> {
        let workspace_root = self.resolve_workspace_root(None)?;
        let runtime = self.session_runtime().await?;
        let application = application_from_runtime(&runtime)?;
        let command =
            permission_rule_write_command_from_args(workspace_root.as_path(), &args.rule)?;
        let result = application
            .service()
            .replace_permission_rule_command(args.rule_id, command)
            .await
            .map_err(|error| AppError::Internal(error.to_string()));
        runtime.shutdown();
        let updated = permission_rule_output(result?)?;
        render_serialized(args.rule.format, &updated)
    }

    pub(super) async fn render_permissions_revoke_command(
        &self,
        args: PermissionsRevokeArgs,
    ) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let application = application_from_runtime(&runtime)?;
        let result = application
            .service()
            .revoke_permission_rule_as(args.rule_id, args.reason, Some("cli".to_owned()))
            .await
            .map_err(|error| AppError::Internal(error.to_string()));
        runtime.shutdown();
        render_serialized(args.format, &permission_rule_output(result?)?)
    }

    pub(super) async fn render_permissions_reply_command(
        &self,
        args: PermissionsReplyArgs,
    ) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let providers = runtime.application_services().provider_catalog;
        let services = runtime.application_services();
        let queries = services.session_queries.ok_or_else(session_storage_error)?;
        let commands = services
            .execution_commands
            .ok_or_else(session_storage_error)?;
        let session_id = selected_session_id(queries.as_ref(), args.session_id, args.last).await?;
        let outcome = commands
            .reply_permission(SessionPermissionReplyRequest::new(
                session_id,
                resolve_run_options(providers.as_ref(), None, None, None, None)?,
                PermissionReply {
                    request_id: args.request_id,
                    kind: permission_reply_kind_from_arg(args.kind),
                    reason: args.reason,
                    scope: args.scope.map(permission_scope_from_arg),
                },
                Some("cli".to_string()),
            ))
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let session = queries
            .session_presentation(outcome.session_id)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let latest_event_seq = latest_event_seq(queries.as_ref(), session.id).await?;
        render_serialized(
            args.format,
            &SessionOutput {
                session: session_detail_from_presentation(session, latest_event_seq),
            },
        )
    }

    pub(super) async fn render_snapshot_command(
        &self,
        args: SnapshotArgs,
    ) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let application = application_from_runtime(&runtime)?;
        let snapshot = application.snapshot_status();
        if !snapshot.registry_available {
            return Err(AppError::Config(
                "snapshot registry is not enabled in this runtime".to_owned(),
            ));
        }
        let active = snapshot
            .active
            .into_iter()
            .map(|entry| ActiveSnapshotOutput {
                session_id: entry.session_id,
                path: entry.path,
                branch: entry.branch,
                backend: entry.backend,
                created_here: entry.created_here,
            })
            .collect::<Vec<_>>();
        let managed = snapshot
            .managed
            .into_iter()
            .map(|entry| ManagedSnapshotOutput {
                path: entry.path,
                session_id: entry.session_id,
                branch: entry.branch,
                backend: entry.backend,
                registered_with_git: entry.registered_with_git,
                registered_with_rift: entry.registered_with_rift,
                stale: entry.stale,
            })
            .collect::<Vec<_>>();
        render_serialized(
            args.format,
            &SnapshotOutput {
                workspace_root: snapshot.workspace_root,
                capabilities: SnapshotCapabilitiesOutput {
                    preferred_backend: snapshot.preferred_backend,
                    git: SnapshotBackendSupportOutput {
                        available: snapshot.git.available,
                        detail: snapshot.git.detail,
                    },
                    rift: SnapshotBackendSupportOutput {
                        available: snapshot.rift.available,
                        detail: snapshot.rift.detail,
                    },
                },
                active,
                managed,
            },
        )
    }

    pub(super) async fn render_git_command(&self, args: GitArgs) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let application = application_from_runtime(&runtime)?;
        let status = application
            .git_status()
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;

        render_serialized(
            args.format,
            &GitOutput {
                workspace_root: status.workspace_root,
                git_available: status.git_available,
                repo: status.repo,
                gh_available: status.gh_available,
                branch: status.branch,
                upstream: status.upstream,
                ahead: status.ahead,
                behind: status.behind,
                staged_files: status.staged_files,
                unstaged_files: status.unstaged_files,
                untracked_files: status.untracked_files,
                changed_files: status.changed_files,
                clean: status.clean,
                snapshot_active_sessions: status.snapshot_active_sessions,
                snapshot_managed_dirs: status.snapshot_managed_dirs,
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
        let providers = runtime.application_services().provider_catalog;
        let services = runtime.application_services();
        let queries = services.session_queries.ok_or_else(session_storage_error)?;
        let control = services
            .execution_control
            .ok_or_else(session_storage_error)?;
        let commands = services
            .execution_commands
            .ok_or_else(session_storage_error)?;
        let session_id = selected_session_id(queries.as_ref(), args.session_id, args.last).await?;
        let options =
            resolve_continue_options(providers.as_ref(), control.as_ref(), session_id, &args)
                .await?;
        let outcome = commands
            .continue_session(SessionExecutionRequest::new(session_id, options))
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let session = queries
            .session_presentation(outcome.session_id)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let latest_event_seq = latest_event_seq(queries.as_ref(), session.id).await?;
        render_serialized(
            args.format,
            &SessionOutput {
                session: session_detail_from_presentation(session, latest_event_seq),
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
        let services = runtime.application_services();
        let queries = services.session_queries.ok_or_else(session_storage_error)?;
        let session = queries
            .session_presentation(args.session_id)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let latest_event_seq = latest_event_seq(queries.as_ref(), session.id).await?;
        let messages = queries
            .list_projected_messages(session.id, true)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let output = DebugSessionOutput {
            session: session_detail_from_presentation(session, latest_event_seq),
            messages: messages
                .iter()
                .map(|message| DebugMessageOutput {
                    id: message.id,
                    role: message.role,
                    state: message.state,
                    text: projected_message_visible_text(message),
                })
                .collect(),
        };
        if args.json {
            render_serialized(OutputFormat::Json, &output)
        } else {
            Ok(format_debug_session_output(&output))
        }
    }

    pub(super) async fn render_diagnostics_command(
        &self,
        args: DiagnosticsArgs,
    ) -> Result<String, AppError> {
        self.with_session_runtime_services(|services| async move {
            let configuration = services
                .configuration
                .runtime_configuration()
                .map_err(|error| AppError::Config(error.to_string()))?;
            let config = &configuration.effective_config;
            let provider_count = config
                .get("providers")
                .and_then(serde_json::Value::as_object)
                .map_or(0, |providers| providers.len());
            let plugin_count = config
                .get("plugins")
                .and_then(|plugins| plugins.get("list"))
                .and_then(serde_json::Value::as_array)
                .map_or(0, Vec::len);
            let metadata = configuration
                .configuration_document
                .get("meta")
                .cloned()
                .unwrap_or_default();
            let project_path = metadata
                .get("project_config_path")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let project_found = metadata
                .get("project_config_found")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let applied_layers = metadata
                .get("applied_layers")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|layer| layer.get("description").and_then(serde_json::Value::as_str))
                .map(str::to_owned)
                .collect();
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
                        path: configuration.config_path.display().to_string(),
                        found: configuration.config_found,
                        project_path,
                        project_found,
                        applied_layers,
                        provider_count,
                        plugin_count,
                    },
                    environment: DiagnosticsEnvironmentOutput {
                        agena_database_url_set: std::env::var_os("AGENA_DATABASE_URL").is_some(),
                        agena_database_path_set: std::env::var_os("AGENA_DATABASE_PATH").is_some(),
                        agena_adapter_log_set: std::env::var_os("AGENA_ADAPTER_LOG").is_some(),
                    },
                },
            )
        })
        .await
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
        let providers = runtime.application_services().provider_catalog;
        let services = runtime.application_services();
        let queries = services.session_queries.ok_or_else(session_storage_error)?;
        let commands = services
            .execution_commands
            .ok_or_else(session_storage_error)?;
        let options = resolve_run_options(
            providers.as_ref(),
            model,
            agent_profile,
            temperature,
            max_output_tokens,
        )?;
        let created = commands
            .create_session(SessionCreateRequest {
                title,
                parent_session_id: None,
            })
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let outcome = commands
            .submit_user_message(SessionUserMessageRequest::new(
                created.session_id,
                options,
                vec![SessionUserMessagePart::Text(TextPart {
                    text: prompt.to_owned(),
                    synthetic: false,
                    ignored: false,
                })],
            ))
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let session = queries
            .session_presentation(outcome.session_id)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        if session.workflow_state == WorkflowState::Blocked {
            return Err(AppError::Config(
                "command is blocked awaiting permission or user input".to_owned(),
            ));
        }
        let latest_event_seq = latest_event_seq(queries.as_ref(), session.id).await?;
        let text = last_assistant_text_from_projection(
            queries
                .list_projected_messages(session.id, true)
                .await
                .map_err(|error| AppError::Internal(error.to_string()))?,
        )
        .unwrap_or_default();
        if json {
            render_serialized(
                OutputFormat::Json,
                &ExecOutput {
                    session: session_detail_from_presentation(session, latest_event_seq),
                    text,
                },
            )
        } else {
            Ok(text)
        }
    }

    pub(super) async fn render_fork_command(&self, args: ForkArgs) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let services = runtime.application_services();
        let queries = services.session_queries.ok_or_else(session_storage_error)?;
        let commands = services
            .execution_commands
            .ok_or_else(session_storage_error)?;
        let outcome = commands
            .fork_session(SessionForkRequest {
                session_id: args.session_id,
                at_message_id: args.at_message,
                title: args.title,
                expected_version: None,
            })
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let forked = queries
            .session_presentation(outcome.session_id)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let latest_event_seq = latest_event_seq(queries.as_ref(), forked.id).await?;
        render_serialized(
            args.format,
            &SessionForkOutput {
                source_session_id: args.session_id,
                forked: session_detail_from_presentation(forked, latest_event_seq),
            },
        )
    }
}
