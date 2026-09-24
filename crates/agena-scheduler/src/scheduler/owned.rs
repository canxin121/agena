//! Model-facing scheduler scope. Host administration retains separate unscoped
//! methods; unowned host rows without a workspace owner never appear in this scope.
use super::*;

impl Scheduler {
    fn owned(job: &ScheduledJob, workspace: &str, session: Option<i64>) -> bool {
        job.owner_workspace.as_deref() == Some(workspace) && job.owner_session_id == session
    }
    pub async fn list_owned(
        &self,
        workspace: &str,
        session: Option<i64>,
    ) -> SchedulerResult<Vec<ScheduledJob>> {
        Ok(self
            .list()
            .await?
            .into_iter()
            .filter(|job| Self::owned(job, workspace, session))
            .collect())
    }
    pub async fn history_owned(
        &self,
        workspace: &str,
        session: Option<i64>,
        id: Option<uuid::Uuid>,
        limit: usize,
    ) -> SchedulerResult<Vec<SchedulerHistoryEntry>> {
        self.store
            .list_history_owned(workspace, session, id, limit)
            .await
    }
    pub async fn remove_owned(
        &self,
        workspace: &str,
        session: Option<i64>,
        id: uuid::Uuid,
    ) -> SchedulerResult<bool> {
        let Some(expected) = self
            .store
            .get(id)
            .await?
            .filter(|entry| Self::owned(&entry.job, workspace, session))
        else {
            return Ok(false);
        };
        if !self.store.remove_checked(&expected).await? {
            return Err(SchedulerError::Conflict(id));
        }
        Ok(true)
    }
    pub async fn pause_owned(
        &self,
        workspace: &str,
        session: Option<i64>,
        id: uuid::Uuid,
    ) -> SchedulerResult<Option<ScheduledJob>> {
        let Some(expected) = self
            .store
            .get(id)
            .await?
            .filter(|entry| Self::owned(&entry.job, workspace, session))
        else {
            return Ok(None);
        };
        let mut job = expected.job.clone();
        if job.pause() {
            return self.persist_edit(&expected, job).await.map(Some);
        }
        Ok(Some(job))
    }
    pub async fn resume_owned(
        &self,
        workspace: &str,
        session: Option<i64>,
        id: uuid::Uuid,
    ) -> SchedulerResult<Option<ScheduledJob>> {
        let Some(expected) = self
            .store
            .get(id)
            .await?
            .filter(|entry| Self::owned(&entry.job, workspace, session))
        else {
            return Ok(None);
        };
        let mut job = expected.job.clone();
        let changed = if expected.claim_key().is_some() && job.paused && !job.completed {
            job.paused = false;
            true
        } else {
            job.resume(Utc::now())?
        };
        if changed {
            return self.persist_edit(&expected, job).await.map(Some);
        }
        Ok(Some(job))
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn update_owned(
        &self,
        workspace: &str,
        session: Option<i64>,
        id: uuid::Uuid,
        prompt: Option<String>,
        expression: Option<String>,
        max_age_days: Option<u32>,
        misfire_policy: Option<crate::job::MisfirePolicy>,
        retry_policy: Option<crate::job::RetryPolicy>,
    ) -> SchedulerResult<Option<ScheduledJob>> {
        let Some(expected) = self
            .store
            .get(id)
            .await?
            .filter(|entry| Self::owned(&entry.job, workspace, session))
        else {
            return Ok(None);
        };
        let mut job = expected.job.clone();
        if job.update(
            prompt,
            expression,
            max_age_days,
            misfire_policy,
            retry_policy,
            Utc::now(),
        )? {
            return self.persist_edit(&expected, job).await.map(Some);
        }
        Ok(Some(job))
    }
}
