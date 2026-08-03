use super::{AppError, SessionStore, visit_event_message_ids, visit_event_part_ids};

impl SessionStore {
    pub(crate) async fn workspace_id(&self) -> Result<i64, AppError> {
        let workspace_id = self
            .workspace_id
            .get_or_try_init(|| async {
                self.workspace_repository
                    .ensure_id(self.workspace_path.as_str())
                    .await
                    .map_err(|error| AppError::Internal(error.to_string()))
            })
            .await?;
        Ok(*workspace_id)
    }

    pub(crate) async fn lookup_workspace_id(&self) -> Result<Option<i64>, AppError> {
        if let Some(workspace_id) = self.workspace_id.get() {
            return Ok(Some(*workspace_id));
        }

        let workspace_id = self
            .workspace_repository
            .lookup_id(self.workspace_path.as_str())
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        if let Some(workspace_id) = workspace_id {
            let _ = self.workspace_id.set(workspace_id);
        }
        Ok(workspace_id)
    }

    /// Raise the message/part id allocator floor to the highest id already
    /// persisted in the event log. Runs at most once per process: the
    /// database-backed allocator then seeds to `MAX(current, observed_high+1)`
    /// on every subsequent call, so a restart cannot collide with ids a
    /// previous process already handed out.
    pub(crate) async fn seed_id_allocator(&self) -> Result<(), AppError> {
        self.id_seed
            .get_or_try_init(|| async {
                // Stream every persistent event once and take the highest
                // message id we observe. This replaces the per-session
                // `fold_session_view` walk that used to dominate startup on
                // instances with many sessions — event-store iteration is
                // O(events) and avoids re-projecting each session.
                use agena_domain::{EventFilter, EventScope};
                use agena_storage::StoreRange;
                let filter = EventFilter::new(EventScope::Global);
                let mut max_message_id: i64 = 0;
                let mut max_part_id: i64 = 0;
                let mut cursor: i64 = 0;
                loop {
                    let chunk = self
                        .publisher
                        .store()
                        .range(
                            &filter,
                            StoreRange {
                                after_seq_global: cursor,
                                limit: 4096,
                            },
                        )
                        .await
                        .map_err(|err| {
                            AppError::Internal(format!("scan events for id allocator: {err}"))
                        })?;
                    if chunk.is_empty() {
                        break;
                    }
                    cursor = chunk.last().map(|e| e.meta.seq_global).unwrap_or(cursor);
                    for event in &chunk {
                        visit_event_message_ids(&event.kind, |id| {
                            if id > max_message_id {
                                max_message_id = id;
                            }
                        });
                        visit_event_part_ids(&event.kind, |id| {
                            if id > max_part_id {
                                max_part_id = id;
                            }
                        });
                    }
                }
                self.ids
                    .seed_message_id(max_message_id)
                    .await
                    .map_err(|error| AppError::Internal(error.to_string()))?;
                self.ids
                    .seed_part_id(max_part_id)
                    .await
                    .map_err(|error| AppError::Internal(error.to_string()))?;
                Ok(())
            })
            .await
            .map(|_| ())
    }
}
