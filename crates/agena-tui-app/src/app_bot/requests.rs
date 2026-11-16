use super::*;
use tokio::io::AsyncReadExt;

impl App {
    pub(super) fn request_bot(&mut self, operation: BotOperation) {
        self.bot.cancel();
        self.bot.error = None;
        let request_id = self.bot.request_id;
        self.current_route = Route::BotEntry(request_id);
        let client = self.application.client().clone();
        let backend = self.application.clone();
        let tx = self.tx.clone();
        let avatar_read_error = self.i18n.text("bot-avatar-read-error");
        let avatar_size_error = self.i18n.text("bot-avatar-size-error");
        let avatar_format_error = self.i18n.text("bot-avatar-format-error");
        let task = tokio::spawn(async move {
            let result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
                match operation {
                    BotOperation::List => Ok(BotReply::List(client.list_bots(true).await?)),
                    BotOperation::Edit(id) => {
                        let profile = client.get_bot(id).await?;
                        anyhow::ensure!(
                            !profile.archived && profile.id == id,
                            "Bot is archived or unavailable"
                        );
                        Ok(BotReply::Edit(profile))
                    }
                    BotOperation::Open(selected) => {
                        let list = client.list_bots(true).await?;
                        if selected.is_none()
                            && !list.items.is_empty()
                            && list.items.iter().all(|p| p.archived)
                        {
                            return Ok(BotReply::List(list));
                        }
                        let session = if let Some(id) = selected {
                            anyhow::ensure!(
                                list.items.iter().any(|p| p.id == id && !p.archived),
                                "Selected Bot is unavailable or archived; open the Bot list"
                            );
                            client.ensure_named_bot_session(id).await?
                        } else {
                            client.ensure_bot_session().await?
                        };
                        anyhow::ensure!(
                            selected.is_none_or(|id| id == session.bot_id),
                            "Server returned another Bot identity"
                        );
                        let profile = client.get_bot(session.bot_id).await?;
                        anyhow::ensure!(
                            !profile.archived && profile.id == session.bot_id,
                            "Bot identity changed while opening; retry"
                        );
                        Ok(BotReply::Opened {
                            profile,
                            list,
                            session,
                        })
                    }
                    BotOperation::LoadAvatar(path) => {
                        // Reject special paths before opening: opening a FIFO
                        // can wait indefinitely for a writer. Recheck the open
                        // handle below in case the file changed meanwhile.
                        let metadata = tokio::fs::metadata(&path).await.map_err(|error| {
                            UiFailure::invalid_with_diagnostic(avatar_read_error.clone(), error)
                        })?;
                        if !metadata.is_file() {
                            return Err(UiFailure::message(avatar_read_error).into());
                        }
                        if metadata.len() > MAX_BOT_AVATAR_FILE_BYTES as u64 {
                            return Err(UiFailure::message(avatar_size_error).into());
                        }
                        let file = tokio::fs::File::open(&path).await.map_err(|error| {
                            UiFailure::invalid_with_diagnostic(avatar_read_error.clone(), error)
                        })?;
                        let metadata = file.metadata().await.map_err(|error| {
                            UiFailure::invalid_with_diagnostic(avatar_read_error.clone(), error)
                        })?;
                        if !metadata.is_file() {
                            return Err(UiFailure::message(avatar_read_error).into());
                        }
                        if metadata.len() > MAX_BOT_AVATAR_FILE_BYTES as u64 {
                            return Err(UiFailure::message(avatar_size_error).into());
                        }
                        let mut bytes = Vec::new();
                        file.take(MAX_BOT_AVATAR_FILE_BYTES as u64 + 1)
                            .read_to_end(&mut bytes)
                            .await
                            .map_err(|error| {
                                UiFailure::invalid_with_diagnostic(avatar_read_error, error)
                            })?;
                        if bytes.len() > MAX_BOT_AVATAR_FILE_BYTES {
                            return Err(UiFailure::message(avatar_size_error).into());
                        }
                        let data =
                            tokio::task::spawn_blocking(move || bot_avatar_from_bytes(&bytes))
                                .await?
                                .map_err(|error| {
                                    UiFailure::invalid_with_diagnostic(avatar_format_error, error)
                                })?;
                        Ok(BotReply::AvatarLoaded { path, data })
                    }
                    BotOperation::Save(create, base, avatar) => {
                        let profile = if let Some((id, revision)) = base {
                            client
                                .update_bot(
                                    id,
                                    &BotUpdate {
                                        request_key: create.request_key,
                                        expected_revision: revision,
                                        name: create.name,
                                        persona: create.persona.unwrap_or_default(),
                                        avatar,
                                    },
                                )
                                .await?
                        } else {
                            client.create_bot(&create).await?
                        };
                        anyhow::ensure!(
                            !profile.archived && base.is_none_or(|(id, _)| id == profile.id),
                            "Server returned another Bot identity"
                        );
                        Ok(BotReply::Saved(profile))
                    }
                    BotOperation::Conversations(id) => {
                        let profile = client.get_bot(id).await?;
                        anyhow::ensure!(profile.id == id, "Server returned another Bot identity");
                        let owner_id = id.to_string();
                        let mut groups = BOT_SURFACES
                            .iter()
                            .map(|surface| BotConversationGroup {
                                surface,
                                rows: Vec::new(),
                            })
                            .collect::<Vec<_>>();
                        for group in &mut groups {
                            // Local is a single pinned conversation, never a
                            // list of legacy roots. Remote surfaces are paged
                            // independently so old local rows cannot crowd them out.
                            let sessions = if group.surface == "local" {
                                if let Some(session_id) = profile.local_session_id {
                                    let result = client
                                        .query_revalidated(agena_api::queries::Query::GetSession(
                                            agena_api::queries::GetSessionParams { session_id },
                                        ))
                                        .await?;
                                    let agena_api::queries::QueryResult::Session(session) = result
                                    else {
                                        anyhow::bail!(
                                            "Server returned the wrong local conversation result"
                                        );
                                    };
                                    vec![session]
                                } else {
                                    Vec::new()
                                }
                            } else {
                                backend
                                    .list_sessions(agena_api::queries::ListSessionsParams {
                                        limit: Some(200),
                                        owner_bot_id: Some(id),
                                        owner_surface: agena_domain::SessionSurface::parse(
                                            group.surface,
                                        ),
                                        exclude_subagents: true,
                                        ..Default::default()
                                    })
                                    .await?
                                    .items
                            };
                            for session in sessions {
                                if session.owner_bot_id.as_deref() != Some(owner_id.as_str())
                                    || session.owner_surface.as_deref() != Some(group.surface)
                                    || (group.surface == "local"
                                        && Some(session.id) != profile.local_session_id)
                                {
                                    continue;
                                }
                                group.rows.push(BotConversationRow {
                                    session_id: session.id,
                                    local: group.surface == "local",
                                    title: session.title,
                                    favorite: session.favorite,
                                    message_count: session.message_count,
                                    last_message_at_ms: session
                                        .last_message_at
                                        .map(|t| t.timestamp_millis()),
                                    running: matches!(
                                        session.state,
                                        agena_api::resource::SessionState::Running { .. }
                                    ),
                                });
                            }
                        }
                        for group in &mut groups {
                            // Newest first inside a group, matching the sidebar.
                            group.rows.sort_by(|a, b| {
                                b.last_message_at_ms
                                    .cmp(&a.last_message_at_ms)
                                    .then_with(|| b.session_id.cmp(&a.session_id))
                            });
                        }
                        Ok(BotReply::Conversations { bot: id, groups })
                    }
                    BotOperation::DeleteConversation { bot, session_id } => {
                        // Match the Web path: a conversation with live work is
                        // fenced and stopped before the row is removed, so the
                        // transcript never disappears mid-stream. The plain
                        // delete already settles the queue inside its command;
                        // the explicit stop additionally ends the live attempt.
                        // Best effort: an already-finished run answers with a
                        // terminal outcome, and the delete below settles the
                        // rest of the queue inside its own command.
                        let _ = client
                            .command(agena_api::commands::Command::CancelRun(
                                agena_api::commands::CancelRunParams {
                                    session_id,
                                    execution_id: None,
                                },
                            ))
                            .await;
                        let result = client
                            .command(agena_api::commands::Command::DeleteSession(
                                agena_api::commands::DeleteSessionParams {
                                    session_id,
                                    expected_version: None,
                                },
                            ))
                            .await?;
                        anyhow::ensure!(
                            matches!(
                                result,
                                agena_api::commands::CommandResult::SessionDeleted { id }
                                    if id == session_id
                            ),
                            "Server did not confirm the deletion"
                        );
                        Ok(BotReply::DeletedConversation { bot })
                    }
                    BotOperation::Archive(id, body) => {
                        let profile = client.archive_bot(id, &body).await?;
                        anyhow::ensure!(
                            profile.id == id && profile.archived,
                            "Server returned an invalid archive result"
                        );
                        Ok(BotReply::Archived(profile))
                    }
                }
            })
            .await
            .map_err(|_| UiFailure::internal("Bot request timed out"))
            .and_then(|r: anyhow::Result<BotReply>| r.map_err(UiFailure::from_backend));
            let _ = tx
                .send(AppMessage::BotSessionReady { request_id, result })
                .await;
        });
        self.bot.task = Some(task.abort_handle());
    }
}
