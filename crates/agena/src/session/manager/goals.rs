use super::*;

impl SessionManager {
    pub async fn get_goal(&self, session_id: i64) -> Result<Option<SessionGoal>, AppError> {
        self.store.load_goal(session_id).await
    }

    pub async fn goal_cost_summary(&self, session_id: i64) -> Result<SessionCostSummary, AppError> {
        let state = self.execution_state();
        self.store
            .goal_cost_summary(session_id, state.cache_policy())
            .await
    }

    pub async fn usage_stats(&self, query: UsageStatsQuery) -> Result<UsageStats, AppError> {
        self.store.usage_stats(query).await
    }

    pub async fn create_goal(
        &self,
        request: SessionGoalCreateRequest,
    ) -> Result<SessionGoal, AppError> {
        validate_session_goal_objective(&request.objective).map_err(AppError::Internal)?;
        let state = self.execution_state();
        let existing = self.get_session(request.session_id).await?;
        if existing.goal.is_some() {
            return Err(AppError::Internal(format!(
                "session {} already has an active goal",
                request.session_id
            )));
        }
        let updated = self
            .store
            .upsert_goal(
                request.session_id,
                request.objective,
                request.token_budget,
                state.cache_policy(),
            )
            .await?;
        let goal = updated.goal.clone().ok_or_else(|| {
            AppError::Internal(format!(
                "goal missing after create for session {}",
                request.session_id
            ))
        })?;
        let mut updated = updated;
        updated.runtime.goal.clear();
        updated
            .runtime
            .goal
            .set_pending_steering(goal.id, GoalSteeringKind::ObjectiveUpdated);
        self.persist_session_changes(updated, Vec::new(), Vec::new(), None, state.clone())
            .await?;
        let updated = self.get_session(request.session_id).await?;
        let goal = updated.goal.clone().ok_or_else(|| {
            AppError::Internal(format!(
                "goal missing after runtime update for session {}",
                request.session_id
            ))
        })?;
        self.spawn_idle_goal_run_if_needed(request.session_id, false);
        self.publish_goal_event(&goal, request.session_id).await?;
        Ok(goal)
    }

    pub async fn update_goal(
        &self,
        request: SessionGoalUpdateRequest,
    ) -> Result<SessionGoal, AppError> {
        if let Some(objective) = request.objective.as_deref() {
            validate_session_goal_objective(objective).map_err(AppError::Internal)?;
        }
        let state = self.execution_state();
        let existing = self.get_session(request.session_id).await?;
        let Some(goal_before) = existing.goal.as_ref() else {
            return Err(AppError::Internal(format!(
                "session {} has no goal to update",
                request.session_id
            )));
        };
        if request
            .expected_goal_id
            .is_some_and(|expected_goal_id| expected_goal_id != goal_before.id)
        {
            return Err(AppError::Internal(format!(
                "session {} goal changed before update",
                request.session_id
            )));
        }

        let updated = self
            .store
            .update_goal(
                request.session_id,
                GoalUpdate {
                    objective: request.objective,
                    status: request.status,
                    token_budget: request.token_budget,
                    expected_goal_id: request.expected_goal_id,
                },
                state.cache_policy(),
            )
            .await?
            .ok_or_else(|| {
                AppError::Internal(format!("session {} has no goal", request.session_id))
            })?;
        let goal = updated.goal.clone().ok_or_else(|| {
            AppError::Internal(format!(
                "goal missing after update for session {}",
                request.session_id
            ))
        })?;
        if &goal != goal_before {
            self.publish_goal_event(&goal, request.session_id).await?;
        }
        Ok(goal)
    }

    pub async fn complete_goal(&self, session_id: i64) -> Result<SessionGoal, AppError> {
        let state = self.execution_state();
        let session = self.get_session(session_id).await?;
        let goal = session.goal.ok_or_else(|| {
            AppError::Internal(format!("session {session_id} has no goal to complete"))
        })?;
        if goal.status == GoalStatus::Completed {
            return Err(AppError::Internal(format!(
                "session {session_id} goal is already completed"
            )));
        }
        let updated = self
            .store
            .complete_goal(session_id, state.cache_policy())
            .await?
            .ok_or_else(|| AppError::Internal(format!("session {session_id} has no goal")))?;
        updated.goal.as_ref().ok_or_else(|| {
            AppError::Internal(format!(
                "goal missing after completion for session {session_id}"
            ))
        })?;
        let mut updated = updated;
        updated.runtime.goal.clear();
        self.persist_session_changes(updated, Vec::new(), Vec::new(), None, state.clone())
            .await?;
        let updated = self.get_session(session_id).await?;
        let goal = updated.goal.clone().ok_or_else(|| {
            AppError::Internal(format!(
                "goal missing after runtime completion cleanup for session {session_id}"
            ))
        })?;
        self.publish_goal_event(&goal, session_id).await?;
        Ok(goal)
    }

    pub async fn clear_goal(&self, session_id: i64) -> Result<bool, AppError> {
        let state = self.execution_state();
        if self.get_goal(session_id).await?.is_none() {
            return Ok(false);
        }
        let cleared = self
            .store
            .clear_goal(session_id, state.cache_policy())
            .await?;
        if cleared {
            let mut updated = self.get_session(session_id).await?;
            if !updated.runtime.goal.is_empty() {
                updated.runtime.goal.clear();
                let _ = self
                    .persist_session_changes(updated, Vec::new(), Vec::new(), None, state.clone())
                    .await?;
            }
            self.publisher
                .publish(
                    crate::event::PublishContext::for_session(session_id),
                    EventKind::SessionGoalUpdated(SessionGoalEvent {
                        session_id,
                        goal_id: None,
                        objective: None,
                        status: None,
                        token_budget: None,
                        tokens_used: None,
                        time_used_seconds: None,
                        completed_at_ms: None,
                        ts_ms: Utc::now().timestamp_millis(),
                    }),
                )
                .await
                .map_err(|err| {
                    AppError::Internal(format!("publish goal clear event failed: {err}"))
                })?;
        }
        Ok(cleared)
    }
}
