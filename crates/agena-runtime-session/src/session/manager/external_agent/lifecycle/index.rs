use std::collections::BTreeSet;

use super::*;

static INDEX_WATCH_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(8);
const DEFAULT_WAIT_SECONDS: u16 = 20;
const MAX_WAIT_SECONDS: u16 = 30;

impl SessionManager {
    pub(super) async fn manage_native_index(
        &self,
        workspace: i64,
        action: ExternalAgentWorkspaceAction,
    ) -> Result<ExternalAgentWorkspaceResult, AppError> {
        use ExternalAgentWorkspaceAction as Action;
        let agent = match &action {
            Action::IndexList { agent, cursor, .. } => {
                if cursor
                    .as_ref()
                    .is_some_and(|value| value.len() > acp::MAX_CURSOR_BYTES)
                {
                    return Err(AppError::Config(
                        "Invalid external agent history cursor.".into(),
                    ));
                }
                *agent
            }
            Action::BatchGet { agent, native_ids } => {
                if native_ids.is_empty()
                    || native_ids.len() > acp::MAX_BATCH_IDS
                    || native_ids.iter().any(|id| invalid_index_id(id))
                    || native_ids.iter().collect::<BTreeSet<_>>().len() != native_ids.len()
                {
                    return Err(AppError::Config(format!(
                        "Specify 1 to {} distinct native conversation IDs.",
                        acp::MAX_BATCH_IDS,
                    )));
                }
                *agent
            }
            Action::SetArchived {
                agent, native_id, ..
            } => {
                if invalid_native_string(native_id) {
                    return Err(AppError::Config(
                        "Invalid external agent conversation ID.".into(),
                    ));
                }
                *agent
            }
            Action::Watch {
                agent,
                wait_seconds,
            } => {
                let wait = wait_seconds.unwrap_or(DEFAULT_WAIT_SECONDS);
                if !(1..=MAX_WAIT_SECONDS).contains(&wait) {
                    return Err(AppError::Config(format!(
                        "Wait must be 1 to {MAX_WAIT_SECONDS} seconds."
                    )));
                }
                let cwd = canonical_directory(&self.workspace_root_by_id(workspace).await?).await?;
                return self
                    .watch_native_index(agent.to_owned(), cwd, Duration::from_secs(wait.into()))
                    .await;
            }
            Action::List { .. }
            | Action::Import { .. }
            | Action::Open { .. }
            | Action::PreviewHistory { .. } => {
                unreachable!("ordinary native lifecycle request")
            }
        };
        let cwd = canonical_directory(&self.workspace_root_by_id(workspace).await?).await?;
        let _permit = self.reserve_execution()?;
        let cancel = self.shutdown.child_token();
        let fallback = matches!(
            action,
            Action::IndexList {
                archived: ExternalAgentArchiveFilter::Unarchived,
                ..
            }
        );
        let mut standard = match &action {
            Action::IndexList { .. } | Action::SetArchived { .. } => {
                Some(acp::AcpClient::spawn(agent, &cwd)?)
            }
            _ => None,
        };
        let outcome = async {
            let mut negotiation_error = None;
            if let Some(client) = standard.as_mut() {
                match client.supports_native_archive(&cancel).await {
                    Ok(true) => return standard_history(client, &cwd, action, &cancel).await,
                    Ok(false) => {}
                    Err(error)
                        if agent == ExternalAgent::ClaudeCode
                            && !matches!(error, AppError::Cancelled) =>
                    {
                        negotiation_error = Some(error);
                    }
                    Err(error) => return Err(error),
                }
            }
            let opened = match acp::AcpClient::open_index(agent, &cwd, &cancel).await {
                Ok(opened) => opened,
                Err(error) if fallback && !matches!(error, AppError::Cancelled) => {
                    tracing::debug!(%error, "could not negotiate optional Claude session index");
                    None
                }
                Err(error) => return Err(error),
            };
            let Some((mut client, capabilities)) = opened else {
                if let Some(error) = negotiation_error {
                    return Err(error);
                }
                if fallback && let Some(client) = standard.as_mut() {
                    return standard_history(client, &cwd, action, &cancel).await;
                }
                return Err(unsupported_index());
            };
            let outcome = async {
                match action {
                    Action::IndexList {
                        cursor, archived, ..
                    } => {
                        if archived != ExternalAgentArchiveFilter::Unarchived
                            && !capabilities.archive
                        {
                            return Err(unsupported_index());
                        }
                        let (sessions, next_cursor) = client
                            .list_indexed(&cwd, cursor.as_deref(), archived, &cancel)
                            .await?;
                        Ok(ExternalAgentWorkspaceResult::IndexPage {
                            sessions,
                            next_cursor,
                            capabilities,
                        })
                    }
                    Action::BatchGet { native_ids, .. } if capabilities.batch_get => {
                        let sessions = client.batch_indexed(&cwd, &native_ids, &cancel).await?;
                        Ok(ExternalAgentWorkspaceResult::Batch { sessions })
                    }
                    Action::SetArchived {
                        native_id,
                        archived,
                        ..
                    } if capabilities.archive => {
                        if invalid_index_id(&native_id) {
                            return Err(AppError::Config(
                                "Invalid external agent conversation ID.".into(),
                            ));
                        }
                        client
                            .set_archived_indexed(&cwd, native_id.clone(), archived, &cancel)
                            .await?;
                        Ok(ExternalAgentWorkspaceResult::Archive {
                            native_id,
                            archived,
                        })
                    }
                    _ => Err(unsupported_index()),
                }
            }
            .await;
            client.close().await;
            outcome
        }
        .await;
        if let Some(client) = standard {
            client.close().await;
        }
        outcome
    }

    pub(super) async fn index_import_record(
        &self,
        agent: ExternalAgent,
        cwd: &Path,
        native_id: &str,
        cancel: &CancellationToken,
    ) -> Result<ExternalAgentSessionInfo, AppError> {
        let not_found =
            || AppError::Config("That native conversation is not listed in this workspace.".into());
        if invalid_index_id(native_id) {
            return Err(not_found());
        }
        let opened = match acp::AcpClient::open_index(agent, cwd, cancel).await {
            Ok(opened) => opened,
            Err(AppError::Cancelled) => return Err(AppError::Cancelled),
            Err(error) => {
                tracing::debug!(%error, "optional native history lookup was unavailable");
                None
            }
        };
        let Some((mut client, capabilities)) = opened else {
            return Err(not_found());
        };
        let row = if capabilities.batch_get {
            client
                .batch_indexed(cwd, &[native_id.to_owned()], cancel)
                .await
        } else {
            Ok(Vec::new())
        };
        client.close().await;
        Ok(row?.into_iter().next().ok_or_else(not_found)?.session)
    }

    async fn watch_native_index(
        &self,
        agent: ExternalAgent,
        cwd: PathBuf,
        wait: Duration,
    ) -> Result<ExternalAgentWorkspaceResult, AppError> {
        if agent != ExternalAgent::ClaudeCode {
            return Ok(ExternalAgentWorkspaceResult::Watch {
                supported: false,
                changed: false,
            });
        }
        let permit = INDEX_WATCH_SLOTS.try_acquire().map_err(|_| {
            AppError::Config("Too many native history subscriptions are open.".into())
        })?;
        let cancel = self.shutdown.child_token();
        let _disconnect = cancel.clone().drop_guard();
        self.control_tasks
            .spawn(async move {
                let _permit = permit;
                let Some((mut client, capabilities)) =
                    acp::AcpClient::open_index(agent, &cwd, &cancel).await?
                else {
                    return Ok(ExternalAgentWorkspaceResult::Watch {
                        supported: false,
                        changed: false,
                    });
                };
                let result = if capabilities.watch {
                    client.watch_indexed(&cwd, wait, &cancel).await
                } else {
                    Ok(false)
                };
                client.close().await;
                Ok(ExternalAgentWorkspaceResult::Watch {
                    supported: capabilities.watch,
                    changed: result?,
                })
            })
            .await
            .map_err(|error| {
                AppError::Internal(format!("Native history subscription stopped: {error}"))
            })?
    }
}

async fn standard_history(
    client: &mut acp::AcpClient,
    cwd: &Path,
    action: ExternalAgentWorkspaceAction,
    cancel: &CancellationToken,
) -> Result<ExternalAgentWorkspaceResult, AppError> {
    match action {
        ExternalAgentWorkspaceAction::IndexList {
            cursor, archived, ..
        } => {
            let archive = client.supports_native_archive(cancel).await?;
            let page = client
                .list_workspace_native(cwd, cursor.as_deref(), archived, cancel)
                .await?;
            Ok(ExternalAgentWorkspaceResult::IndexPage {
                sessions: page.sessions,
                next_cursor: page.next_cursor,
                capabilities: agena_domain::ExternalAgentIndexCapabilities {
                    archive,
                    ..Default::default()
                },
            })
        }
        ExternalAgentWorkspaceAction::SetArchived {
            native_id,
            archived,
            ..
        } => {
            client
                .set_native_archived(cwd, &native_id, archived, cancel)
                .await?;
            Ok(ExternalAgentWorkspaceResult::Archive {
                native_id,
                archived,
            })
        }
        _ => unreachable!("operation without a standard ACP history equivalent"),
    }
}

fn unsupported_index() -> AppError {
    AppError::Config("The installed agent does not support this native history operation.".into())
}

fn invalid_index_id(value: &str) -> bool {
    value.trim() != value || invalid_native_string(value)
}
