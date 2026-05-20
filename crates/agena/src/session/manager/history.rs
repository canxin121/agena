use super::*;

impl SessionManager {
    pub async fn fork_session(&self, request: SessionForkRequest) -> Result<Session, AppError> {
        let state = self.execution_state();
        let source = self
            .store
            .load_session(request.session_id, state.cache_policy())
            .await?;
        if let Some(expected) = request.expected_version
            && source.version != expected
        {
            return Err(AppError::Conflict {
                session_id: request.session_id,
                expected,
                current: source.version,
            });
        }
        let title = request
            .title
            .unwrap_or_else(|| format!("Fork of {}", source.title));
        self.store
            .fork_session(source, request.at_message_id, title, state.cache_policy())
            .await
    }

    /// External entry: cancel the in-flight turn for `session_id`. Returns
    /// `Ok(())` if a token was signalled, `Err` if no turn is active.
    pub async fn cancel_active_turn(&self, session_id: i64) -> Result<(), AppError> {
        self.turn_registry
            .cancel(session_id)
            .await
            .map_err(turn_control_to_app_error)
    }

    /// External entry: inject `parts` as a steer message into the in-flight
    /// turn for `session_id`. Returns `Err` if no turn is active or the
    /// channel was closed.
    pub async fn steer_input(
        &self,
        session_id: i64,
        parts: Vec<PartContent>,
    ) -> Result<(), AppError> {
        self.turn_registry
            .steer(session_id, parts)
            .await
            .map_err(turn_control_to_app_error)
    }

    pub async fn rewind_session(&self, request: SessionRewindRequest) -> Result<Session, AppError> {
        let state = self.execution_state();
        let source = self
            .store
            .load_session(request.session_id, state.cache_policy())
            .await?;
        if let Some(expected) = request.expected_version
            && source.version != expected
        {
            return Err(AppError::Conflict {
                session_id: request.session_id,
                expected,
                current: source.version,
            });
        }
        let title = format!("Rewind of {}", source.title);
        self.store
            .fork_session(
                source,
                Some(request.message_id),
                title,
                state.cache_policy(),
            )
            .await
    }

    pub async fn unrewind_session(
        &self,
        request: SessionUnrewindRequest,
    ) -> Result<Session, AppError> {
        let state = self.execution_state();
        let session = self
            .store
            .load_session(request.session_id, state.cache_policy())
            .await?;
        if let Some(expected) = request.expected_version
            && session.version != expected
        {
            return Err(AppError::Conflict {
                session_id: request.session_id,
                expected,
                current: session.version,
            });
        }
        tracing::warn!(
            session_id = request.session_id,
            message_id = request.message_id,
            "session unrewind is disabled; same-session provider prompts are append-only"
        );
        Ok(session)
    }

    /// Reload `session_id` and bail with [`AppError::Conflict`] if the live
    /// `version` no longer equals `expected`. Used by command handlers that
    /// take an `If-Match`-style optimistic-lock parameter.
    pub async fn assert_session_version(
        &self,
        session_id: i64,
        expected: i64,
    ) -> Result<(), AppError> {
        let session = self
            .store
            .load_session(session_id, self.execution_state().cache_policy())
            .await?;
        if session.version != expected {
            return Err(AppError::Conflict {
                session_id,
                expected,
                current: session.version,
            });
        }
        Ok(())
    }

    /// Return every persisted rewind audit entry for this session.
    ///
    /// These entries are legacy audit data from older same-session rewind
    /// behavior. New rewind requests create forked sessions and do not append
    /// checkpoints to the source session.
    pub async fn list_rewind_checkpoints(
        &self,
        session_id: i64,
    ) -> Result<Vec<crate::session::RewindCheckpoint>, AppError> {
        self.store.list_rewind_checkpoints(session_id).await
    }

    /// Serialise `session_id` as a JSONL bundle. The first line is the
    /// session header (id, parent, depth, runtime); subsequent lines are
    /// persistent event payloads in `seq_global` order.
    pub async fn export_session_jsonl(&self, session_id: i64) -> Result<String, AppError> {
        self.store.export_session_jsonl(session_id).await
    }

    /// Replay a JSONL bundle produced by [`Self::export_session_jsonl`] into
    /// this manager's workspace as a fresh session.
    pub async fn import_session_jsonl(&self, bundle: &str) -> Result<Session, AppError> {
        let state = self.execution_state();
        self.store
            .import_session_jsonl(bundle, state.cache_policy())
            .await
    }

    /// Return every session that shares the same `root_id`, ordered by
    /// `(depth, id)`. Useful for tree visualisation and bulk export.
    pub async fn list_session_tree(&self, root_id: i64) -> Result<Vec<SessionSummary>, AppError> {
        self.store.list_session_tree(root_id).await
    }
}
