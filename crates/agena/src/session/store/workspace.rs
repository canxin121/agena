use super::{
    AppError, GlobalIdAllocator, SessionStore, visit_event_message_ids, visit_event_part_ids,
    workspace_crud,
};

impl SessionStore {
    pub(crate) async fn workspace_id(&self) -> Result<i64, AppError> {
        let workspace_id = self
            .workspace_id
            .get_or_try_init(|| async {
                workspace_crud::ensure_workspace_id(&self.db, self.workspace_path.as_str())
                    .await
                    .map_err(AppError::from)
            })
            .await?;
        Ok(*workspace_id)
    }

    pub(crate) async fn lookup_workspace_id(&self) -> Result<Option<i64>, AppError> {
        if let Some(workspace_id) = self.workspace_id.get() {
            return Ok(Some(*workspace_id));
        }

        let workspace_id =
            workspace_crud::get_workspace_id_by_path(&self.db, self.workspace_path.as_str())
                .await?;
        if let Some(workspace_id) = workspace_id {
            let _ = self.workspace_id.set(workspace_id);
        }
        Ok(workspace_id)
    }

    pub(crate) async fn ensure_id_allocator(
        &self,
        allocator: &mut GlobalIdAllocator,
    ) -> Result<(), AppError> {
        if allocator.initialized {
            return Ok(());
        }

        // Stream every persistent event once and take the highest message id
        // we observe. This replaces the per-session `fold_session_view` walk
        // that used to dominate startup on instances with many sessions —
        // event-store iteration is O(events) and avoids re-projecting each
        // session.
        use crate::event::{EventFilter, Scope, StoreRange};
        let filter = EventFilter::new(Scope::Global);
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
        let next_message_id = max_message_id + 1;
        let next_part_id = max_part_id + 1;

        if !allocator.initialized {
            allocator.initialized = true;
            allocator.next_message_id = next_message_id;
            allocator.next_part_id = next_part_id;
        }
        Ok(())
    }
}
