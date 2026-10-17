//! Execution failures use the same durable Part writes as every other activity.
//! The per-session gate covers lookup and insert across retries, cleanup, and
//! concurrent session opens. No client is responsible for retaining failures.
use super::*;

impl StoreAdapter {
    fn failure_gate(&self, session_id: i64) -> Arc<tokio::sync::Mutex<()>> {
        let mut gates = self.failure_gates.lock().expect("failure gates lock");
        let key = session_id;
        if let Some(gate) = gates.get(&key).and_then(std::sync::Weak::upgrade) {
            return gate;
        }
        gates.retain(|_, gate| gate.strong_count() > 0);
        let gate = Arc::new(tokio::sync::Mutex::new(()));
        gates.insert(key, Arc::downgrade(&gate));
        gate
    }

    /// Retries and repeated terminal cleanup update the same Part, preserving
    /// both its storage identity and the public failure correlation identity.
    pub(crate) async fn record_failure_part(
        &self,
        session_id: i64,
        run_id: i64,
        failure: &agena_failure::Failure,
    ) -> Result<Part, AppError> {
        self.write_failure_part(session_id, run_id, failure, None, true)
            .await
    }

    async fn write_failure_part(
        &self,
        session_id: i64,
        run_id: i64,
        failure: &agena_failure::Failure,
        retry: Option<agena_failure::RetryProgress>,
        replace: bool,
    ) -> Result<Part, AppError> {
        let gate = self.failure_gate(session_id);
        let _guard = gate.lock().await;
        let snapshot = self
            .facade
            .part_snapshot(session_id, None)
            .await
            .map_err(store_error)?;
        let existing = failure_part_for_run(snapshot.parts(), session_id, run_id);
        if !replace && let Some(part) = existing {
            return Ok(part.clone());
        }
        let mut problem = agena_failure::UserProblem::from(failure);
        let mut extra = BTreeMap::new();
        let mut retry_progress = retry;
        if let Some(part) = existing {
            if let Some(id) = part.content.pointer("/problem/id")
                && let Ok(id) = serde_json::from_value(id.clone())
            {
                problem.id = id;
            }
            if retry_progress.is_none() {
                retry_progress = part
                    .content
                    .get("retry")
                    .and_then(|retry| {
                        serde_json::from_value::<agena_failure::RetryProgress>(retry.clone()).ok()
                    })
                    .map(|mut retry| {
                        retry.outcome = agena_failure::RetryOutcome::Failed;
                        retry.next_at_ms = None;
                        retry.phase = None;
                        retry.started_at_ms = None;
                        retry
                    });
            }
        }
        let category = serde_json::to_value(problem.category)
            .expect("failure category serializes")
            .as_str()
            .map(ToOwned::to_owned);
        let message = problem.user.fallback.clone();
        extra.insert(
            "problem".to_owned(),
            serde_json::to_value(problem).expect("user problem serializes"),
        );
        let content = TypedContent::Error(part_content::ErrorContent {
            category,
            message,
            detail: None,
            retry: retry_progress,
            extra,
        });
        let new_part =
            new_part_from_content("error", PartRole::Assistant, &content, PartState::Failed)?;
        if let Some(part) = existing {
            return self
                .update_part(
                    session_id,
                    part.part_id,
                    PartDelta {
                        content: Some(new_part.content),
                        ..Default::default()
                    },
                )
                .await;
        }
        self.append_parts(session_id, run_id, vec![new_part])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "append_parts returned no failure part for run {run_id}"
                ))
            })
    }

    pub(crate) async fn record_provider_retry(
        &self,
        session_id: i64,
        run_id: i64,
        retry: &agena_domain::ProviderRetryStatus,
    ) -> Result<Part, AppError> {
        // RetryGuard supplies reviewed prose, never raw transport diagnostics.
        // Keep historical prose valid after the live retry has finished.
        let message = retry.message.replace(" Retrying automatically.", "");
        let failure = AppError::Provider(message).failure();
        self.write_failure_part(
            session_id,
            run_id,
            &failure,
            Some(agena_failure::RetryProgress {
                attempt: retry.attempt,
                max_retries: retry.max_retries,
                next_at_ms: Some(retry.next_at_ms),
                phase: Some(agena_failure::RetryPhase::Backoff),
                started_at_ms: None,
                outcome: agena_failure::RetryOutcome::Retrying,
            }),
            true,
        )
        .await
    }

    /// Progress updates mutate the existing durable Part. The caller sends
    /// one transition per request/first output, never a write for every token.
    pub(crate) async fn advance_provider_retry(
        &self,
        session_id: i64,
        run_id: i64,
        phase: agena_failure::RetryPhase,
    ) -> Result<(), AppError> {
        let gate = self.failure_gate(session_id);
        let _guard = gate.lock().await;
        let snapshot = self
            .facade
            .part_snapshot(session_id, None)
            .await
            .map_err(store_error)?;
        let Some(part) = failure_part_for_run(snapshot.parts(), session_id, run_id) else {
            return Ok(());
        };
        let Some(mut retry) =
            part.content.get("retry").cloned().and_then(|value| {
                serde_json::from_value::<agena_failure::RetryProgress>(value).ok()
            })
        else {
            return Ok(());
        };
        if retry.outcome != agena_failure::RetryOutcome::Retrying
            || retry.phase == Some(phase)
            || phase == agena_failure::RetryPhase::Backoff
            || (phase == agena_failure::RetryPhase::Requesting
                && retry.phase == Some(agena_failure::RetryPhase::Streaming))
        {
            return Ok(());
        }
        retry.phase = Some(phase);
        retry.next_at_ms = None;
        retry
            .started_at_ms
            .get_or_insert_with(|| chrono::Utc::now().timestamp_millis());
        let mut content = part.content.clone();
        content["retry"] = serde_json::to_value(retry).expect("retry progress serializes");
        self.update_part(
            session_id,
            part.part_id,
            PartDelta {
                content: Some(content),
                ..Default::default()
            },
        )
        .await?;
        Ok(())
    }

    pub(crate) async fn finish_provider_retry(
        &self,
        session_id: i64,
        run_id: i64,
        outcome: agena_failure::RetryOutcome,
    ) -> Result<Option<Part>, AppError> {
        let gate = self.failure_gate(session_id);
        let _guard = gate.lock().await;
        let snapshot = self
            .facade
            .part_snapshot(session_id, None)
            .await
            .map_err(store_error)?;
        let Some(part) = failure_part_for_run(snapshot.parts(), session_id, run_id) else {
            return Ok(None);
        };
        if part.content.get("retry").is_some_and(Value::is_object) {
            let mut content = part.content.clone();
            content["retry"]["outcome"] =
                serde_json::to_value(outcome).expect("retry outcome serializes");
            let retry = content["retry"]
                .as_object_mut()
                .expect("retry is an object");
            for field in ["next_at_ms", "phase", "started_at_ms"] {
                retry.remove(field);
            }
            return self
                .update_part(
                    session_id,
                    part.part_id,
                    PartDelta {
                        content: Some(content),
                        ..Default::default()
                    },
                )
                .await
                .map(Some);
        }
        Ok(Some(part.clone()))
    }

    /// Repair abandoned runs, including older marker-only failures and a
    /// process stopping between terminalizing a marker and writing its error.
    /// Called while opening an inactive session or settling its execution.
    pub(crate) async fn ensure_failure_parts(&self, session_id: i64) -> Result<(), AppError> {
        let snapshot = self
            .facade
            .part_snapshot(session_id, None)
            .await
            .map_err(store_error)?;
        let error_owners: BTreeSet<_> = snapshot
            .parts()
            .iter()
            .filter(|part| {
                part.kind == "error"
                    || (part.kind == "compaction"
                        && part.state == PartState::Failed
                        && part.content.get("problem").is_some_and(Value::is_object))
            })
            .filter_map(|part| part.run_id)
            .collect();
        for marker in snapshot.parts().iter().filter(|part| {
            part.origin_session_id == session_id
                && part.is_run_marker()
                && part.state == PartState::Failed
                && !error_owners.contains(&part.part_id)
        }) {
            if failure_part_for_run(snapshot.parts(), session_id, marker.part_id).is_some() {
                continue;
            }
            let restarted = marker.content.get("abort_reason").and_then(Value::as_str)
                == Some("process_restart");
            let failure = agena_failure::Failure::new(
                agena_failure::FailureCode::new(if restarted {
                    "execution.interrupted"
                } else {
                    "execution.failed"
                }),
                agena_failure::FailureCategory::Internal,
                agena_failure::FailureResponsibility::System,
                agena_failure::RetryDirective::AfterUserAction,
                agena_failure::RecoveryDirective::Retry,
                agena_failure::FailureImpact::OperationFailed,
                agena_failure::UserPresentation::new(
                    "failure.execution.failed",
                    if restarted {
                        "The server stopped before this operation finished. You can retry it."
                    } else {
                        "This operation failed. You can retry it."
                    },
                ),
            );
            self.write_failure_part(session_id, marker.part_id, &failure, None, false)
                .await?;
        }
        // A dropped provider future or cancellation grace timeout can bypass
        // processor finalization. Seal any live retry status from the durable
        // terminal marker, including replies that crossed a checkpoint.
        let snapshot = self
            .facade
            .part_snapshot(session_id, None)
            .await
            .map_err(store_error)?;
        for error in snapshot.parts().iter().filter(|part| {
            part.origin_session_id == session_id
                && part.kind == "error"
                && part
                    .content
                    .pointer("/retry/outcome")
                    .and_then(Value::as_str)
                    == Some("retrying")
        }) {
            if let Some(run_id) = error.run_id
                && let Some(outcome) = settled_retry_outcome(snapshot.parts(), run_id)
            {
                self.finish_provider_retry(session_id, run_id, outcome)
                    .await?;
            }
        }
        Ok(())
    }
}

/// A checkpoint can require a new run marker inside the same reply. Its retry
/// still updates the original failure Part; ownership and history never move.
fn failure_part_for_run(parts: &[Part], session_id: i64, run_id: i64) -> Option<&Part> {
    if let Some(part) = parts.iter().find(|part| {
        part.origin_session_id == session_id && part.kind == "error" && part.run_id == Some(run_id)
    }) {
        return Some(part);
    }
    let marker = parts.iter().find(|part| {
        part.origin_session_id == session_id && part.part_id == run_id && part.is_run_marker()
    })?;
    if marker.content.get("run_kind").and_then(Value::as_str) == Some("compaction") {
        return None;
    }
    let reply_id = marker
        .content
        .get("reply_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())?;
    let owners: BTreeSet<_> = parts
        .iter()
        .filter(|part| {
            part.origin_session_id == session_id
                && part.is_run_marker()
                && part.content.get("reply_id").and_then(Value::as_str) == Some(reply_id)
                && part.content.get("run_kind").and_then(Value::as_str) != Some("compaction")
        })
        .map(|part| part.part_id)
        .collect();
    // Forks share immutable history, including reply IDs. A branch's retry
    // must update its own error and never mutate an ancestor's shared Part.
    parts.iter().find(|part| {
        part.origin_session_id == session_id
            && part.kind == "error"
            && part.run_id.is_some_and(|id| owners.contains(&id))
    })
}

fn settled_retry_outcome(parts: &[Part], run_id: i64) -> Option<agena_failure::RetryOutcome> {
    let owner = parts
        .iter()
        .find(|part| part.part_id == run_id && part.is_run_marker())?;
    let marker = owner
        .content
        .get("reply_id")
        .and_then(Value::as_str)
        .and_then(|reply_id| {
            parts.iter().rev().find(|part| {
                part.origin_session_id == owner.origin_session_id
                    && part.is_run_marker()
                    && part.content.get("reply_id").and_then(Value::as_str) == Some(reply_id)
                    && part.content.get("run_kind").and_then(Value::as_str) != Some("compaction")
            })
        })
        .unwrap_or(owner);
    match marker.state {
        PartState::Completed => Some(agena_failure::RetryOutcome::Recovered),
        PartState::Failed => Some(agena_failure::RetryOutcome::Failed),
        PartState::Cancelled => Some(agena_failure::RetryOutcome::Cancelled),
        _ => None,
    }
}
