use super::{AppError, EventKind, Session, SessionCachePolicy, SessionStore};

impl SessionStore {
    pub(crate) async fn reserve_message_id_block(&self, count: i64) -> Result<i64, AppError> {
        debug_assert!(count > 0);
        let mut allocator = self.ids.lock().await;
        self.ensure_id_allocator(&mut allocator).await?;
        let first = allocator.next_message_id;
        allocator.next_message_id = first.saturating_add(count);
        Ok(first - 1)
    }

    pub(crate) async fn reserve_part_id_block(&self, count: i64) -> Result<i64, AppError> {
        debug_assert!(count > 0);
        let mut allocator = self.ids.lock().await;
        self.ensure_id_allocator(&mut allocator).await?;
        let first = allocator.next_part_id;
        allocator.next_part_id = first.saturating_add(count);
        Ok(first - 1)
    }

    pub(crate) async fn append_history_items(
        &self,
        session: Session,
        items: Vec<EventKind>,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        self.append_history_items_inner(session, items, cache_policy, false)
            .await
    }

    /// Same as [`Self::append_history_items`] but persists the events without
    /// broadcasting them on the in-process bus. Use for replay-only flows
    /// (fork copy, JSONL import) so subscribers don't observe historical
    /// reconstructions as fresh activity.
    pub(crate) async fn append_history_items_silent(
        &self,
        session: Session,
        items: Vec<EventKind>,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        self.append_history_items_inner(session, items, cache_policy, true)
            .await
    }
}
