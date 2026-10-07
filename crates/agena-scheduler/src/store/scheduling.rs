//! Small, indexed scheduling reads. Deadline reads never load job payloads.

use std::collections::BinaryHeap;

use super::*;

const MAX_DUE_BATCH_SIZE: usize = 32;

#[derive(Debug, Clone, Default)]
pub struct DueJobQuery {
    pub limit: usize,
    pub excluded_job_ids: Vec<Uuid>,
    pub excluded_session_ids: Vec<i64>,
}

impl DueJobQuery {
    pub(super) fn batch_limit(&self) -> usize {
        self.limit.min(MAX_DUE_BATCH_SIZE)
    }

    pub(super) fn allows(&self, job: &ScheduledJob) -> bool {
        !self.excluded_job_ids.contains(&job.id)
            && job
                .owner_session_id
                .is_none_or(|id| !self.excluded_session_ids.contains(&id))
    }

    fn sql_exclusions(&self) -> (String, Vec<sea_orm::Value>) {
        let mut sql = String::new();
        let mut values = Vec::new();
        if !self.excluded_job_ids.is_empty() {
            sql.push_str(" AND id NOT IN (");
            sql.push_str(&vec!["?"; self.excluded_job_ids.len()].join(","));
            sql.push(')');
            values.extend(self.excluded_job_ids.iter().map(|id| id.to_string().into()));
        }
        if !self.excluded_session_ids.is_empty() {
            sql.push_str(" AND (json_extract(job_json, '$.owner_session_id') IS NULL OR json_extract(job_json, '$.owner_session_id') NOT IN (");
            sql.push_str(&vec!["?"; self.excluded_session_ids.len()].join(","));
            sql.push_str("))");
            values.extend(self.excluded_session_ids.iter().map(|id| (*id).into()));
        }
        (sql, values)
    }
}

impl JobSnapshot {
    pub(super) fn wake_at_ms(&self) -> Option<i64> {
        if self.job.paused || self.job.completed {
            return None;
        }
        if self.claim_key.is_some() {
            return Some(
                self.renewed_at_ms
                    .map_or(0, |at| at.saturating_add(CLAIM_LEASE_MILLIS)),
            );
        }
        self.job
            .retry_at
            .or_else(|| {
                self.job
                    .pending_delivery
                    .is_none()
                    .then_some(self.job.next_fire_at)
                    .flatten()
            })
            .map(|at| at.timestamp_millis())
    }
}

impl InMemoryJobStore {
    pub(super) fn due_batch(&self, now_ms: i64, query: &DueJobQuery) -> Vec<JobSnapshot> {
        let limit = query.batch_limit();
        if limit == 0 {
            return Vec::new();
        }
        let state = self.inner.read();
        // Retain only bounded IDs while scanning; clone prompts/JSON solely
        // for the small admitted batch, rather than for every due job.
        let mut selected = BinaryHeap::with_capacity(limit + 1);
        for entry in state.jobs.values().filter(|entry| query.allows(&entry.job)) {
            if let Some(at) = entry.wake_at_ms().filter(|at| *at <= now_ms) {
                selected.push((at, entry.job.id));
                if selected.len() > limit {
                    selected.pop();
                }
            }
        }
        selected
            .into_sorted_vec()
            .into_iter()
            .filter_map(|(_, id)| state.jobs.get(&id).cloned())
            .collect()
    }
}

impl SqliteJobStore {
    pub(super) async fn check_invalid_json(&self) -> SchedulerResult<()> {
        // This partial index is empty on healthy databases.
        if let Some(row) = self.db.query_one(Statement::from_string(DatabaseBackend::Sqlite,
            "SELECT job_json, delivery_key, claimed_at_ms FROM agena_scheduler_jobs WHERE NOT json_valid(job_json) LIMIT 1")).await? {
            Self::decode_rows(vec![row]).await?;
        }
        Ok(())
    }

    pub(super) async fn due_batch(
        &self,
        now_ms: i64,
        query: &DueJobQuery,
    ) -> SchedulerResult<Vec<JobSnapshot>> {
        let limit = query.batch_limit();
        if limit == 0 {
            return Ok(Vec::new());
        }
        let (exclude, exclusions) = query.sql_exclusions();
        let mut values = vec![now_ms.into()];
        values.extend(exclusions.clone());
        values.push((limit as i64).into());
        values.extend([CLAIM_LEASE_MILLIS.into(), lease_cutoff(now_ms).into()]);
        values.extend(exclusions);
        values.push((limit as i64).into());
        values.push((limit as i64).into());
        // Each partial index produces only scalar candidate IDs/deadlines.
        // LIMIT applies before fetching or decoding the full job payloads.
        let sql = format!(
            "SELECT jobs.job_json, jobs.delivery_key, jobs.claimed_at_ms \
             FROM agena_scheduler_jobs AS jobs JOIN ( \
               SELECT id, wake_at_ms FROM ( \
               SELECT id, COALESCE(retry_at_ms, next_fire_at_ms) AS wake_at_ms \
               FROM agena_scheduler_jobs \
               WHERE paused = 0 AND completed = 0 AND delivery_key IS NULL \
                 AND COALESCE(retry_at_ms, next_fire_at_ms) <= ? {exclude} \
               ORDER BY COALESCE(retry_at_ms, next_fire_at_ms), id LIMIT ? \
               ) UNION ALL SELECT id, wake_at_ms FROM ( \
               SELECT id, CASE WHEN claimed_at_ms IS NULL THEN 0 ELSE claimed_at_ms + ? END AS wake_at_ms \
               FROM agena_scheduler_jobs \
               WHERE paused = 0 AND completed = 0 AND delivery_key IS NOT NULL \
                 AND (claimed_at_ms IS NULL OR claimed_at_ms <= ?) {exclude} \
               ORDER BY claimed_at_ms, id LIMIT ? \
               ) \
               ORDER BY wake_at_ms, id LIMIT ? \
             ) AS due ON jobs.id = due.id ORDER BY due.wake_at_ms, due.id"
        );
        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                sql,
                values,
            ))
            .await?;
        Self::decode_rows(rows).await
    }

    pub(super) async fn next_deadline(&self, query: &DueJobQuery) -> SchedulerResult<Option<i64>> {
        let (exclude, exclusions) = query.sql_exclusions();
        let mut values = exclusions.clone();
        values.push(CLAIM_LEASE_MILLIS.into());
        values.extend(exclusions);
        // Two indexed first-row reads, including nullable abandoned leases.
        // Do not let an in-flight occurrence's old fire time cause busy waits.
        let sql = format!(
            "SELECT MIN(wake_at_ms) AS wake_at_ms FROM ( \
               SELECT wake_at_ms FROM ( \
                 SELECT COALESCE(retry_at_ms, next_fire_at_ms) AS wake_at_ms \
                 FROM agena_scheduler_jobs \
                 WHERE paused = 0 AND completed = 0 AND delivery_key IS NULL \
                   AND COALESCE(retry_at_ms, next_fire_at_ms) IS NOT NULL {exclude} \
                 ORDER BY COALESCE(retry_at_ms, next_fire_at_ms), id LIMIT 1 \
               ) UNION ALL SELECT wake_at_ms FROM ( \
                 SELECT CASE WHEN claimed_at_ms IS NULL THEN 0 ELSE claimed_at_ms + ? END AS wake_at_ms \
                 FROM agena_scheduler_jobs \
                 WHERE paused = 0 AND completed = 0 AND delivery_key IS NOT NULL {exclude} \
                 ORDER BY claimed_at_ms, id LIMIT 1 \
               ) \
             )"
        );
        let row = self
            .db
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                sql,
                values,
            ))
            .await?;
        Ok(row
            .map(|row| row.try_get::<Option<i64>>("", "wake_at_ms"))
            .transpose()?
            .flatten())
    }
}
