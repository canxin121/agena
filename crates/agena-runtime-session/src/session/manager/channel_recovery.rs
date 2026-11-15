//! Explicit host review may reconnect an unchanged channel scope to its history.
use super::*;

impl SessionManager {
    /// Recover a compatible channel epoch after a local provider/source review.
    /// The caller owns that review and the binding gate; this method separately
    /// requires the proposed authority to pass the live policy verifier.
    pub async fn recover_channel_session_authority<F, Fut>(
        &self,
        id: i64,
        expected: agena_domain::ChannelAuthority,
        current: agena_domain::ChannelAuthority,
        current_scope_key: &str,
        commit: F,
    ) -> Result<(), AppError>
    where
        F: FnOnce() -> Fut + Send,
        Fut: std::future::Future<Output = Result<(), AppError>> + Send,
    {
        let session = session_id(id)?;
        let dispatch = self.session_dispatch(session);
        let _submission = dispatch.submission.lock().await;
        let verifier = self.channel_verifier.get().ok_or_else(|| {
            AppError::Config("channel authorization service is unavailable".into())
        })?;
        verifier
            .authorize(&current)
            .await
            .map_err(AppError::Config)?;
        let snapshot = self
            .kernel()
            .reader
            .control_snapshot(session, 1)
            .await
            .map_err(kernel_error)?;
        // A retry after the committed CAS must not cancel work admitted under
        // this new epoch. The binding repository may still be reconciling its
        // own record after that first commit.
        if snapshot.configuration.channel_authority.as_ref() == Some(&current)
            && snapshot
                .configuration
                .owner
                .as_ref()
                .is_none_or(|owner| owner.scope_key.as_deref() == Some(current_scope_key))
        {
            return commit().await;
        }
        let mut recovery = ChannelAuthorityRecovery {
            metadata_version: snapshot.metadata.metadata_version.get(),
            control_version: snapshot.control_version,
            expected,
            current,
            scope_key: current_scope_key.to_owned(),
        };
        recovery
            .validate(Utc::now().timestamp_millis())
            .map_err(kernel_error)?;
        let stored = snapshot
            .configuration
            .channel_authority
            .as_ref()
            .ok_or_else(|| {
                AppError::Config("channel recovery requires an existing channel authority".into())
            })?;
        // A previous review may have committed the kernel CAS while the
        // binding repository failed. A later review can advance that exact
        // compatible intermediate epoch, but can never replace a newer one.
        let mut compatible = recovery.expected.clone();
        compatible.epoch = stored.epoch;
        compatible.role = stored.role;
        if stored.epoch < recovery.expected.epoch
            || stored.epoch >= recovery.current.epoch
            || &compatible != stored
        {
            return Err(AppError::Config(
                "channel session changed before recovery; inspect its current authority".into(),
            ));
        }
        recovery.expected = stored.clone();
        recovery
            .validate(Utc::now().timestamp_millis())
            .map_err(kernel_error)?;
        drop(snapshot);

        // Keep admission fenced while cancelling both queued and in-flight
        // inputs and native descendants. Their durable attempts are terminal
        // before the new authority can become visible.
        self.cancel_channel_tree_with_locked_root(id).await?;
        tokio::time::timeout(Duration::from_secs(30), self.wait_session_idle(id))
            .await
            .map_err(|_| {
                AppError::Config(
                    "old channel execution is still stopping; retry recovery after it settles"
                        .into(),
                )
            })?;
        let _execution = dispatch
            .execution
            .try_lock()
            .map_err(|_| AppError::Config("old channel execution is still stopping".into()))?;
        verifier
            .authorize(&recovery.current)
            .await
            .map_err(AppError::Config)?;
        let snapshot = self
            .kernel()
            .reader
            .control_snapshot(session, 1)
            .await
            .map_err(kernel_error)?;
        recovery.metadata_version = snapshot.metadata.metadata_version.get();
        recovery.control_version = snapshot.control_version;
        drop(snapshot);
        let request = RequestId::new();
        persist(|| {
            self.kernel()
                .sessions
                .recover_channel_authority(request, session, recovery.clone())
        })
        .await
        .map_err(kernel_error)?;
        // Keep local submissions fenced through record reconciliation and
        // old-output suppression. On error the caller must fault the binding
        // within this callback before releasing admission.
        commit().await
    }
}
