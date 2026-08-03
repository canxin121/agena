use super::{
    AppError, Arc, DbErr, EventKind, MessageCheckpoint, MessagePartCheckpointedEvent,
    PersistedRuleEventMeta, ProcessorPartIdAllocator, ReservedMessageIds, ReservedProcessorIds,
    Session, SessionCachePolicy, SessionCacheStats, SessionCommit, SessionStore, Utc, access_cache,
    permission_rule_event_from_rule, session, session_from_model_db,
};

use agena_storage_sqlite::run_transaction_effects;

impl SessionStore {
    pub(crate) async fn append_history_items_inner(
        &self,
        mut session: Session,
        items: Vec<EventKind>,
        cache_policy: SessionCachePolicy,
        silent: bool,
    ) -> Result<Session, AppError> {
        if items.is_empty() {
            return Ok(session);
        }
        session.sync_workflow_state();
        let session_id = session.id;
        let now = Utc::now();
        let runtime_to_persist = session.runtime.clone();

        // Persist via the unified publisher; broadcast only when not silent.
        if silent {
            self.history.append_items_silent(session_id, items).await?;
        } else {
            self.history.append_items(session_id, items, now).await?;
        }
        let updated = run_transaction_effects(&self.db, move |txn, _effects| {
            let runtime = runtime_to_persist.clone();
            Box::pin(async move {
                let updated = session::touch_session_updated_at(txn, session_id, runtime)
                    .await?
                    .ok_or_else(|| DbErr::Custom(format!("session not found: {session_id}")))?;
                session_from_model_db(updated)
            })
        })
        .await?;

        let projection = self
            .history
            .load_projection(session_id, updated.runtime.clone())
            .await?;
        session.apply_persisted_metadata(&updated);
        session.install_projected_messages(projection.messages);
        session.runtime = projection.runtime;

        let session_for_cache = session.clone();
        access_cache(self.cache.as_ref(), |guard| {
            guard.insert(session_for_cache, cache_policy);
        });

        Ok(session)
    }

    pub(crate) async fn persist(
        &self,
        commit: SessionCommit,
        cache_policy: SessionCachePolicy,
    ) -> Result<Session, AppError> {
        let SessionCommit {
            mut session,
            checkpoints,
            mut client_events,
            persisted_rules,
        } = commit;
        session.sync_workflow_state();
        let session_id = session.id;
        let checkpoints = ordered_unique_checkpoints(&session, checkpoints)?;
        let now = Utc::now();
        let ts_ms = now.timestamp_millis();
        let mut ordered_client_events = Vec::new();
        for checkpoint in &checkpoints {
            let message = session
                .messages
                .iter()
                .find(|message| message.id == checkpoint.message_id)
                .ok_or_else(|| {
                    AppError::Internal(format!(
                        "checkpoint message {} is missing from session {session_id}",
                        checkpoint.message_id
                    ))
                })?;
            for part_id in &checkpoint.part_ids {
                let part = message
                    .parts
                    .iter()
                    .find(|part| part.id == *part_id)
                    .ok_or_else(|| {
                        AppError::Internal(format!(
                            "checkpoint part {part_id} is missing from message {}",
                            message.id
                        ))
                    })?;
                ordered_client_events.push(EventKind::MessagePartCheckpointed(
                    MessagePartCheckpointedEvent {
                        session_id,
                        execution_id: None,
                        run_id: None,
                        turn_id: None,
                        reply_id: None,
                        message_id: message.id,
                        message_role: message.role,
                        message_state: message.state,
                        message_created_at: message.created_at,
                        message_metadata: message.metadata.clone(),
                        part: part.clone(),
                        ts_ms,
                    },
                ));
            }
        }
        ordered_client_events.extend(client_events);
        client_events = ordered_client_events;
        let cache = Arc::clone(&self.cache);
        let permission_rule_transaction_writer =
            Arc::clone(&self.permission_rule_transaction_writer);
        let session_for_cache = session.clone();
        let session_runtime = session.runtime.clone();
        let (updated_session, persisted_rules_for_event) =
            run_transaction_effects(&self.db, move |txn, effects| {
                let cache = Arc::clone(&cache);
                let permission_rule_transaction_writer =
                    Arc::clone(&permission_rule_transaction_writer);
                Box::pin(async move {
                    let mut persisted_rules_for_event = Vec::new();
                    for rule in &persisted_rules {
                        let (record, created) = permission_rule_transaction_writer
                            .upsert_in_transaction(txn, rule)
                            .await
                            .map_err(|error| DbErr::Custom(error.to_string()))?;
                        persisted_rules_for_event.push(PersistedRuleEventMeta {
                            rule: rule.clone(),
                            rule_id: record.id,
                            created,
                        });
                    }

                    let updated_session =
                        session::touch_session_updated_at(txn, session_id, session_runtime)
                            .await?
                            .ok_or_else(|| {
                                DbErr::Custom(format!("session not found: {session_id}"))
                            })?;
                    let updated_session = session_from_model_db(updated_session)?;

                    let updated_session_for_cache = updated_session.clone();
                    effects.push(async move {
                        access_cache(cache.as_ref(), |guard| {
                            let mut cached_session = session_for_cache;
                            cached_session.apply_persisted_metadata(&updated_session_for_cache);
                            cached_session.refresh_derived();
                            guard.insert(cached_session, cache_policy);
                        });
                    });

                    Ok((updated_session, persisted_rules_for_event))
                })
            })
            .await?;

        // Publish every queued event after the row update commits.
        for kind in client_events {
            self.publish_event(session_id, kind).await?;
        }
        for meta in persisted_rules_for_event {
            let event_kind = if meta.rule.revoked_at_ms.is_some() {
                EventKind::PermissionRuleRevoked(permission_rule_event_from_rule(
                    meta.rule_id,
                    &meta.rule,
                    session_id,
                ))
            } else if meta.created {
                EventKind::PermissionRuleCreated(permission_rule_event_from_rule(
                    meta.rule_id,
                    &meta.rule,
                    session_id,
                ))
            } else {
                EventKind::PermissionRuleUpdated(permission_rule_event_from_rule(
                    meta.rule_id,
                    &meta.rule,
                    session_id,
                ))
            };
            self.publish_event(session_id, event_kind).await?;
        }

        session.apply_persisted_metadata(&updated_session);
        session.refresh_derived();
        Ok(session)
    }

    /// Persist lifecycle events and advance the transcript projection before
    /// returning. Execution completion relies on this synchronous projection
    /// barrier to close every correlated open artifact.
    pub(crate) async fn append_lifecycle_events(
        &self,
        session_id: i64,
        events: Vec<EventKind>,
    ) -> Result<(), AppError> {
        if events.is_empty() {
            return Ok(());
        }
        self.history
            .append_items(session_id, events, Utc::now())
            .await?;
        Ok(())
    }

    pub(crate) async fn resolve_permission_rule_snapshot(
        &self,
        session_id: Option<i64>,
        workspace_id: Option<i64>,
    ) -> Result<Vec<agena_storage::PersistedPermissionRule>, AppError> {
        self.permission_rule_repository
            .resolve_snapshot(session_id, workspace_id)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))
    }

    pub(crate) fn prune_cache(&self, cache_policy: SessionCachePolicy) {
        access_cache(self.cache.as_ref(), |guard| {
            guard.prune(cache_policy);
        });
    }

    pub(crate) fn cache_stats(&self) -> SessionCacheStats {
        access_cache(self.cache.as_ref(), |guard| guard.stats()).unwrap_or_default()
    }

    pub(crate) async fn reserve_message_ids(
        &self,
        part_count: usize,
    ) -> Result<ReservedMessageIds, AppError> {
        self.seed_id_allocator().await?;
        let message_id = self
            .ids
            .next_message_id()
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let first_part_id = self
            .ids
            .reserve_part_id_block(part_count as i64)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?
            + 1;
        let part_ids = (0..part_count)
            .map(|index| first_part_id + index as i64)
            .collect::<Vec<_>>();

        Ok(ReservedMessageIds {
            message_id,
            part_ids,
        })
    }

    pub(crate) async fn reserve_part_id(&self) -> Result<i64, AppError> {
        self.seed_id_allocator().await?;
        self.ids
            .next_part_id()
            .await
            .map_err(|error| AppError::Internal(error.to_string()))
    }

    pub(crate) async fn reserve_processor_ids(&self) -> Result<ReservedProcessorIds, AppError> {
        self.seed_id_allocator().await?;
        let message_id = self
            .ids
            .next_message_id()
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let ids = ReservedProcessorIds {
            message_id,
            part_ids: ProcessorPartIdAllocator::new(Arc::clone(&self.ids)),
        };
        Ok(ids)
    }

    pub(crate) async fn current_workspace_id(&self) -> Result<i64, AppError> {
        self.workspace_id().await
    }
}

fn ordered_unique_checkpoints(
    session: &Session,
    checkpoints: Vec<MessageCheckpoint>,
) -> Result<Vec<MessageCheckpoint>, AppError> {
    let session_order = session
        .messages
        .iter()
        .enumerate()
        .map(|(index, message)| (message.id, index))
        .collect::<std::collections::HashMap<_, _>>();
    let mut parts_by_message = std::collections::HashMap::<i64, Vec<i64>>::new();
    for checkpoint in checkpoints {
        if !session_order.contains_key(&checkpoint.message_id) {
            return Err(AppError::Internal(format!(
                "checkpoint message {} is not owned by session {}",
                checkpoint.message_id, session.id
            )));
        }
        parts_by_message
            .entry(checkpoint.message_id)
            .or_default()
            .extend(checkpoint.part_ids);
    }
    let mut checkpoints = parts_by_message
        .into_iter()
        .map(|(message_id, part_ids)| MessageCheckpoint::parts(message_id, part_ids))
        .collect::<Vec<_>>();
    checkpoints.sort_by_key(|checkpoint| {
        (
            session_order
                .get(&checkpoint.message_id)
                .copied()
                .unwrap_or(usize::MAX),
            checkpoint.message_id,
        )
    });
    Ok(checkpoints)
}

#[cfg(test)]
mod tests {
    use agena_domain::{PermissionMode, PermissionScope};
    use agena_storage::PersistedPermissionRule;
    use sea_orm::{Database, EntityTrait, PaginatorTrait};

    use super::{DbErr, run_transaction_effects, session};
    use agena_storage_sqlite::SeaPermissionRuleTransactionWriter;

    #[tokio::test]
    async fn permission_rule_upsert_rolls_back_when_session_update_fails() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("open in-memory database");
        agena_storage_sqlite::initialize_schema(&db)
            .await
            .expect("initialize schema");
        let rule = PersistedPermissionRule {
            id: None,
            created_at_ms: None,
            updated_at_ms: None,
            action_key: "test.atomic-permission".to_string(),
            mode: PermissionMode::Allow,
            scope: PermissionScope::Session,
            session_id: Some(4242),
            workspace_id: None,
            source: "transaction-rollback-test".to_string(),
            reason: None,
            operator: None,
            revoked_at_ms: None,
            revoked_reason: None,
            revoked_by: None,
        };

        let result = run_transaction_effects(&db, move |txn, _effects| {
            Box::pin(async move {
                SeaPermissionRuleTransactionWriter::upsert_in_transaction(txn, &rule)
                    .await
                    .map_err(|error| DbErr::Custom(error.to_string()))?;
                session::touch_session_updated_at(
                    txn,
                    4242,
                    crate::session::SessionRuntimeState::default(),
                )
                .await?
                .ok_or_else(|| DbErr::Custom("session not found: 4242".to_string()))?;
                Ok(())
            })
        })
        .await;

        assert!(
            result.is_err(),
            "missing session must abort the transaction"
        );
        assert_eq!(
            crate::db::entities::permission_rule::Entity::find()
                .count(&db)
                .await
                .expect("count permission rules"),
            0,
            "the failed session write must roll back its permission-rule upsert"
        );
    }
}
