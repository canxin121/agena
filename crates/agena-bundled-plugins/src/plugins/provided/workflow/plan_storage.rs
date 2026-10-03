//! Version checks are performed under a shared per-workspace/session gate.
//! The gate spans plugin reloads in this process, but is not a distributed DB
//! transaction. No lock is held while waiting for a user's review response.
use super::*;
use std::sync::{LazyLock, Mutex, Weak};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

type Key = (PathBuf, i64);
static PLAN_LOCKS: LazyLock<Mutex<HashMap<Key, Weak<AsyncMutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

impl WorkflowPlugin {
    pub(super) async fn plan_guard(&self) -> SdkResult<OwnedMutexGuard<()>> {
        let session = self
            .host()?
            .get_session(HostGetSessionRequest { session_id: None })
            .await?
            .session
            .id;
        let root = self
            .workspace_root()?
            .canonicalize()
            .map_err(|error| PluginError::internal_error(&error))?;
        let gate = {
            let mut locks = PLAN_LOCKS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            locks.retain(|_, gate| gate.strong_count() > 0);
            let key = (root, session);
            if let Some(gate) = locks.get(&key).and_then(Weak::upgrade) {
                gate
            } else {
                let gate = Arc::new(AsyncMutex::new(()));
                locks.insert(key, Arc::downgrade(&gate));
                gate
            }
        };
        Ok(gate.lock_owned().await)
    }
    pub(super) fn require_expected_plan(
        plan: Option<&WorkflowPlan>,
        expected: Option<&str>,
    ) -> SdkResult<()> {
        if expected.is_some_and(|expected| plan.is_none_or(|plan| plan.revision != expected)) {
            return Err(PluginError::invalid_params(
                "the plan changed since you read it; call plan.get for the current revision, then retry",
            ));
        }
        Ok(())
    }
    pub(super) async fn update_active_plan(&self, plan: &mut WorkflowPlan) -> SdkResult<()> {
        let expected = Some(plan.revision.clone());
        self.save_plan_version(plan, expected).await
    }
    pub(super) async fn save_plan_version(
        &self,
        plan: &mut WorkflowPlan,
        expected: Option<String>,
    ) -> SdkResult<()> {
        let _guard = self.plan_guard().await?;
        let actual = self.load_active_plan().await?;
        if actual.as_ref().map(|plan| plan.revision.as_str()) != expected.as_deref() {
            return Err(PluginError::invalid_params(
                "no changes were applied: the plan was edited, replaced, or cleared while this request was pending; call plan.get before retrying",
            ));
        }
        plan.revision = uuid::Uuid::new_v4().to_string();
        plan.display_warning = None;
        let value = serde_json::to_string_pretty(plan)
            .map_err(|error| PluginError::internal_error(&error))?;
        if value.len() > 1024 * 1024 {
            return Err(PluginError::invalid_params(
                "the plan is too large to store (limit 1 MiB); shorten document_markdown or the step list",
            ));
        }
        self.host()?
            .storage_set(HostStorageSetRequest {
                scope: HostStorageScope::Session,
                visibility: HostStorageVisibility::Shared,
                namespace: PLAN_NAMESPACE.into(),
                key: PLAN_KEY_ACTIVE.into(),
                value,
            })
            .await?;
        if let Err(error) = self.sync_plan_display(Some(plan)).await {
            plan.display_warning = Some(format!(
                "Plan committed, but display refresh failed: {}",
                error.failure.user.fallback
            ));
        }
        Ok(())
    }
    pub(super) async fn clear_plan_version(
        &self,
        expected: Option<String>,
    ) -> SdkResult<Option<String>> {
        let _guard = self.plan_guard().await?;
        let actual = self.load_active_plan().await?;
        if actual.as_ref().map(|plan| plan.revision.as_str()) != expected.as_deref() {
            return Err(PluginError::invalid_params(
                "the plan changed before it could be deleted; call plan.get and retry",
            ));
        }
        self.host()?
            .storage_delete(HostStorageDeleteRequest {
                scope: HostStorageScope::Session,
                visibility: HostStorageVisibility::Shared,
                namespace: PLAN_NAMESPACE.into(),
                key: PLAN_KEY_ACTIVE.into(),
            })
            .await?;
        Ok(self.sync_plan_display(None).await.err().map(|error| {
            format!(
                "Plan cleared, but display refresh failed: {}",
                error.failure.user.fallback
            )
        }))
    }
    pub(super) fn require_activation_grant(&self, requested: Option<bool>) -> SdkResult<()> {
        if requested == Some(false) && !self.config()?.plan.allow_unreviewed_activation {
            return Err(PluginError::invalid_params(
                "cannot activate the plan without review: request_approval: false is only allowed when the trusted setting agena.plan.allow_unreviewed_activation is enabled. Save the plan in planning and call plan.review to ask the user, or have the user enable that setting.",
            ));
        }
        Ok(())
    }
}
