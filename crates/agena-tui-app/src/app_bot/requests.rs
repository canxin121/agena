use super::*;

impl App {
    pub(super) fn request_bot(&mut self, operation: BotOperation) {
        self.bot.cancel();
        self.bot.error = None;
        let request_id = self.bot.request_id;
        self.current_route = Route::BotEntry(request_id);
        let client = self.application.client().clone();
        let tx = self.tx.clone();
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
                            !profile.archived
                                && profile.id == session.bot_id
                                && profile.session_id == session.session_id,
                            "Bot identity changed while opening; retry"
                        );
                        Ok(BotReply::Opened { profile, list })
                    }
                    BotOperation::Save(create, base) => {
                        let profile = if let Some((id, revision)) = base {
                            client
                                .update_bot(
                                    id,
                                    &BotUpdate {
                                        request_key: create.request_key,
                                        expected_revision: revision,
                                        name: create.name,
                                        persona: create.persona.unwrap_or_default(),
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
