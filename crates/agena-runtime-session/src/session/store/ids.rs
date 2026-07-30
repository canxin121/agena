use std::collections::{BTreeMap, BTreeSet};

use super::{
    AppError, EventKind, Session, SessionCachePolicy, SessionStore, rewrite_copied_domain_ids,
    rewrite_event_message_ids, rewrite_event_part_ids, visit_event_message_ids,
    visit_event_part_ids,
};

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

    /// Give replayed history its own globally unique storage identities.
    ///
    /// Message and part projection tables use their ids as primary keys, so
    /// copying an event stream with the source ids would update the source
    /// projection rows to point at the destination session. Keep a stable
    /// one-to-one mapping so references such as `turn_id` and
    /// `parent_message_id` continue to describe the copied conversation.
    pub(crate) async fn remap_copied_history_ids(
        &self,
        items: &mut [EventKind],
    ) -> Result<BTreeMap<i64, i64>, AppError> {
        let mut message_ids = BTreeSet::new();
        let mut part_ids = BTreeSet::new();
        for item in items.iter() {
            visit_event_message_ids(item, |id| {
                if id > 0 {
                    message_ids.insert(id);
                }
            });
            visit_event_part_ids(item, |id| {
                if id > 0 {
                    part_ids.insert(id);
                }
            });
        }

        let message_id_map = if message_ids.is_empty() {
            BTreeMap::new()
        } else {
            let count = i64::try_from(message_ids.len()).map_err(|_| {
                AppError::Internal("too many message ids to copy into a fork".to_string())
            })?;
            let before_first = self.reserve_message_id_block(count).await?;
            message_ids
                .into_iter()
                .enumerate()
                .map(|(index, source_id)| (source_id, before_first + index as i64 + 1))
                .collect::<BTreeMap<_, _>>()
        };
        let part_id_map = if part_ids.is_empty() {
            BTreeMap::new()
        } else {
            let count = i64::try_from(part_ids.len()).map_err(|_| {
                AppError::Internal("too many part ids to copy into a fork".to_string())
            })?;
            let before_first = self.reserve_part_id_block(count).await?;
            part_ids
                .into_iter()
                .enumerate()
                .map(|(index, source_id)| (source_id, before_first + index as i64 + 1))
                .collect::<BTreeMap<_, _>>()
        };

        for item in items.iter_mut() {
            rewrite_event_message_ids(item, |id| message_id_map.get(&id).copied().unwrap_or(id));
            rewrite_event_part_ids(item, |id| part_id_map.get(&id).copied().unwrap_or(id));
        }
        rewrite_copied_domain_ids(items);
        Ok(message_id_map)
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
