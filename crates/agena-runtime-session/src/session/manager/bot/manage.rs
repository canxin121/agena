//! Native ordinary-session lifecycle. No files, project trees, or permission grants.
use super::*;
use agena_storage::kernel::{DeleteGuard, ForkGuard};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) enum LifecyclePreparation {
    Fork { through: EntryId, guard: ForkGuard },
    DeleteApproved { guard: DeleteGuard },
}

impl SessionManager {
    pub(super) async fn bot_prepare_lifecycle(
        &self,
        r: &BotRequest,
    ) -> Result<Option<LifecyclePreparation>, AppError> {
        if r.action != BotAction::Fork {
            return Ok(None);
        }
        let session = session_id(r.session_id.expect("validated target"))?;
        let history = self
            .kernel()
            .reader
            .history(session, None, 1)
            .await
            .map_err(kernel_error)?;
        let through = match r.at_message_id {
            Some(id) => {
                let id = entry_id(id)?;
                self.kernel()
                    .reader
                    .entry(session, id)
                    .await
                    .map_err(kernel_error)?;
                id
            }
            None => {
                history
                    .entries
                    .first()
                    .ok_or_else(|| {
                        invalid("cannot fork an empty conversation; create a new session instead")
                    })?
                    .id
            }
        };
        if r.title.is_some() {
            required_text(&r.title, "title", 512)?;
        }
        let snapshot = self
            .kernel()
            .reader
            .control_snapshot(session, 1)
            .await
            .map_err(kernel_error)?;
        if snapshot.metadata.metadata_version != history.metadata.metadata_version
            || snapshot.metadata.membership_generation != history.metadata.membership_generation
        {
            return Err(invalid(
                "fork source changed while resolving boundary; inspect it again",
            ));
        }
        Ok(Some(LifecyclePreparation::Fork {
            through,
            guard: ForkGuard {
                metadata_version: history.metadata.metadata_version.get(),
                entry_count: snapshot.summary.entry_count,
                title: r.title.clone(),
            },
        }))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn bot_manage(
        &self,
        source: i64,
        call: i64,
        r: &BotRequest,
        command: RequestId,
        origin: Option<i64>,
        key: Option<&str>,
        prepared: Option<&LifecyclePreparation>,
    ) -> Result<Value, AppError> {
        match r.action {
            BotAction::RegisterProject => {
                let path = required_text(&r.path, "path", 4096)?;
                if !std::path::Path::new(path).is_absolute() {
                    return Err(invalid("project path must be absolute"));
                }
                self.require_host_path_permission(
                    source,
                    call,
                    agena_domain::PathKind::Read,
                    path.into(),
                )
                .await?;
                let path = tokio::fs::canonicalize(path).await.map_err(|error| {
                    invalid(&format!(
                        "cannot resolve existing project directory: {error}"
                    ))
                })?;
                Self::require_project_path(&path)?;
                if !tokio::fs::metadata(&path)
                    .await
                    .map_err(|error| {
                        invalid(&format!("cannot inspect project directory: {error}"))
                    })?
                    .is_dir()
                {
                    return Err(invalid("project path must be an existing directory"));
                }
                self.require_operator_bot(source).await?;
                let id = self
                    .kernel_runtime
                    .workspaces
                    .ensure_id(&path.to_string_lossy())
                    .await
                    .map_err(|error| AppError::internal_error(&error))?;
                Ok(json!({"workspace_id":id,"path":path,"outcome":"registered"}))
            }
            BotAction::Create => {
                let workspace = r.workspace_id.expect("validated workspace");
                self.bot_workspace(workspace).await?;
                let title = required_text(&r.title, "title", 512)?;
                let session = persist(|| {
                    self.kernel().sessions.create_configured(
                        command,
                        workspace,
                        title.into(),
                        SessionConfiguration::default(),
                    )
                })
                .await
                .map_err(kernel_error)?
                .value;
                Ok(
                    json!({"session_id":session.id.get(),"workspace_id":workspace,"version":session.metadata_version.get(),"url":link(session.id.get()),"outcome":"created"}),
                )
            }
            BotAction::Rename | BotAction::Organize => {
                let target = r.session_id.expect("validated target");
                let title = if r.action == BotAction::Rename {
                    Some(required_text(&r.title, "title", 512)?.to_owned())
                } else {
                    None
                };
                let metadata = persist(|| {
                    self.kernel().sessions.update_metadata(
                        command,
                        session_id(target).expect("validated"),
                        MetadataChange {
                            title: title.clone(),
                            favorite: r.favorite,
                            pinned: r.pinned,
                            expected_version: r.expected_version,
                            ..Default::default()
                        },
                    )
                })
                .await
                .map_err(kernel_error)?
                .value;
                Ok(
                    json!({"session_id":target,"title":metadata.title,"favorite":metadata.favorite,"pinned":metadata.pinned,"version":metadata.metadata_version.get(),"outcome":if metadata.metadata_version.get() == r.expected_version.expect("validated version") {"unchanged"} else {"applied"},"url":link(target)}),
                )
            }
            BotAction::Fork => {
                let target = r.session_id.expect("validated target");
                let Some(LifecyclePreparation::Fork { through, guard }) = prepared else {
                    return Err(invalid(
                        "fork boundary is missing; inspect source and use a new request key",
                    ));
                };
                let fork = persist(|| {
                    self.kernel().sessions.fork_checked(
                        command,
                        session_id(target).expect("validated"),
                        *through,
                        guard.clone(),
                    )
                })
                .await
                .map_err(kernel_error)?
                .value;
                Ok(
                    json!({"session_id":fork.id.get(),"source_session_id":target,"at_message_id":through.get(),"workspace_id":fork.workspace_id,"relation_kind":"fork","version":fork.metadata_version.get(),"title":fork.title,"outcome":"forked","url":link(fork.id.get())}),
                )
            }
            BotAction::Delete => {
                let target = r.session_id.expect("validated target");
                let session = session_id(target)?;
                let guard = if let Some(LifecyclePreparation::DeleteApproved { guard }) = prepared {
                    guard.clone()
                } else {
                    self.bot_target(source, Some(target)).await?;
                    let snapshot = self
                        .kernel()
                        .reader
                        .control_snapshot(session, 1)
                        .await
                        .map_err(kernel_error)?;
                    if Some(snapshot.metadata.metadata_version.get()) != r.expected_version
                        || Some(snapshot.control_version) != r.expected_control_version
                        || Some(snapshot.summary.entry_count) != r.expected_entry_count
                    {
                        return Err(invalid(
                            "delete target changed since status; inspect it and choose a new request key before requesting confirmation",
                        ));
                    }
                    if snapshot.summary.queued_executions != 0
                        || snapshot.summary.running_executions != 0
                        || !snapshot.pending_interactions.interactions.is_empty()
                    {
                        return Err(invalid(
                            "delete target has queued, running or waiting work; resolve it and inspect status first",
                        ));
                    }
                    let workspace_id = snapshot
                        .metadata
                        .workspace_id
                        .ok_or_else(|| invalid("the target is not a project conversation"))?;
                    let workspace = self.bot_workspace(workspace_id).await?;
                    let source_session = session_id(source)?;
                    let source_message = entry_id(origin.expect("write authority"))?;
                    let source_entry = self
                        .kernel()
                        .reader
                        .entry(source_session, source_message)
                        .await
                        .map_err(kernel_error)?;
                    let guard = DeleteGuard {
                        metadata_version: snapshot.metadata.metadata_version.get(),
                        control_version: snapshot.control_version,
                        entry_count: snapshot.summary.entry_count,
                        source_session,
                        source_message,
                        source_revision: source_entry.revision,
                    };
                    drop(source_entry);
                    let title = snapshot.metadata.title.clone();
                    // Release read leases before waiting for the native interaction.
                    drop(snapshot);
                    let approval = self.request_host_user_input(source,call,crate::part::AskUserToolInput {
                        title:"Delete conversation".into(), kind:"".into(), body_markdown:String::new(),auto_resolution_ms:Some(600_000),
                        questions:vec![agena_domain::UserInputQuestion {
                            header:"Delete".into(), question:format!("Soft-delete exactly this conversation? Workspace: {} (ID {}); title: {:?}; session ID: {} ({}). This removes the conversation from the active list. Project files and folders are untouched.",workspace.display(), workspace_id,title,target,link(target)),
                            options:vec![agena_domain::UserInputOption {label:"Keep conversation".into(),description:"Leave this conversation available.".into(),custom:false},agena_domain::UserInputOption {label:"Delete conversation".into(),description:format!("Soft-delete session {target} only."),custom:false}],multiple:false,allow_custom:false,
                        }],
                    }).await?;
                    if approval.cancelled
                        || approval.timed_out
                        || approval.reply != "Delete conversation"
                    {
                        return Ok(
                            json!({"session_id":target,"outcome":"cancelled","deleted":false,"url":link(target)}),
                        );
                    }
                    self.live_tool(source, call)?;
                    self.require_operator_bot(source).await?;
                    self.require_current_user_quote(
                        source,
                        call,
                        r.user_quote.as_deref().unwrap_or(""),
                    )
                    .await?;
                    let lifecycle = LifecyclePreparation::DeleteApproved {
                        guard: guard.clone(),
                    };
                    self.save_bot_state(
                        key.expect("write key"),
                        &Intent {
                            request: r.clone(),
                            result: None,
                            lifecycle: Some(lifecycle),
                        },
                    )
                    .await?;
                    guard
                };
                let metadata = persist(|| {
                    self.kernel()
                        .sessions
                        .delete_checked(command, session, guard.clone())
                })
                .await
                .map_err(kernel_error)?
                .value;
                if let Some(monitors) = self.tool_executor().monitor_registry() {
                    monitors.stop_session(target);
                }
                Ok(
                    json!({"session_id":target,"workspace_id":metadata.workspace_id,"title":metadata.title,"version":metadata.metadata_version.get(),"outcome":"deleted","deleted":true,"soft_delete":true,"url":link(target)}),
                )
            }
            BotAction::Export => {
                let target = self.bot_target(source, r.session_id).await?;
                // This is the actual existing authenticated JSONL streaming route.
                Ok(
                    json!({"session_id":target,"url":link(target),"export_url":format!("/api/v1/sessions/{target}/export"),"authentication":"same authenticated Agena session","format":"jsonl","suggested_filename":format!("session-{target}.jsonl")}),
                )
            }
            _ => Err(invalid("unsupported lifecycle action")),
        }
    }
}
