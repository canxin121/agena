//! Presentation of the backend-owned retry facts on an ordinary error Part.
use agena_failure::{RetryOutcome, RetryPhase, RetryProgress};
use agena_tui::i18n::I18n;

use crate::{
    PartExecutionStatusResource, TranscriptActivityContent, TranscriptEntry, TranscriptPartContent,
};

pub(crate) struct RetryHeadline {
    pub title: String,
    pub progress: String,
    pub status: PartExecutionStatusResource,
}

fn duration(seconds: i64, i18n: &I18n) -> String {
    let seconds = seconds.max(0);
    if seconds < 60 {
        i18n.text_args(
            "error-part-seconds",
            &agena_tui::fl_args!("count" => seconds.to_string()),
        )
    } else if seconds < 3600 {
        i18n.text_args("error-part-minutes", &agena_tui::fl_args!("minutes" => (seconds / 60).to_string(), "seconds" => (seconds % 60).to_string()))
    } else {
        i18n.text_args("error-part-hours", &agena_tui::fl_args!("hours" => (seconds / 3600).to_string(), "minutes" => (seconds / 60 % 60).to_string()))
    }
}

pub(crate) fn retry_headline(retry: &RetryProgress, now_ms: i64, i18n: &I18n) -> RetryHeadline {
    let count = i18n.text_args(
        if retry.max_retries > 0 { "error-part-retry-count" } else { "error-part-retry-number" },
        &agena_tui::fl_args!("attempt" => retry.attempt.to_string(), "max" => retry.max_retries.to_string()),
    );
    let (title_key, status, progress) = match retry.outcome {
        RetryOutcome::Retrying => {
            let requesting = matches!(
                retry.phase,
                Some(RetryPhase::Requesting | RetryPhase::Streaming)
            );
            let timing = if requesting {
                let stage = i18n.text(if retry.phase == Some(RetryPhase::Streaming) {
                    "error-part-streaming"
                } else {
                    "error-part-requesting"
                });
                if let Some(started_at_ms) = retry.started_at_ms {
                    format!("{stage} · {}", i18n.text_args("error-part-elapsed", &agena_tui::fl_args!("duration" => duration(now_ms.saturating_sub(started_at_ms) / 1000, i18n))))
                } else {
                    stage
                }
            } else if let Some(next_at_ms) = retry.next_at_ms.filter(|next| *next > now_ms) {
                let seconds = next_at_ms.saturating_sub(now_ms).saturating_add(999) / 1000;
                format!("{} · {}",
                    i18n.text_args("error-part-in-duration", &agena_tui::fl_args!("duration" => duration(seconds, i18n))),
                    i18n.text_args("error-part-scheduled-at", &agena_tui::fl_args!("time" => crate::renderer::format_occurred_time(next_at_ms))))
            } else {
                i18n.text("error-part-awaiting-start")
            };
            (
                if requesting {
                    "error-part-retrying"
                } else {
                    "error-part-waiting"
                },
                if requesting {
                    PartExecutionStatusResource::InProgress
                } else {
                    PartExecutionStatusResource::Pending
                },
                format!("{count} · {timing}"),
            )
        }
        RetryOutcome::Recovered => (
            "error-part-recovered",
            PartExecutionStatusResource::Completed,
            i18n.text_args(
                "error-part-recovered-after",
                &agena_tui::fl_args!("attempt" => retry.attempt.to_string()),
            ),
        ),
        RetryOutcome::Failed => (
            "error-part-failed",
            PartExecutionStatusResource::Failed,
            format!(
                "{count} · {}",
                if retry.max_retries > 0 && retry.attempt >= retry.max_retries {
                    i18n.text_args(
                        "error-part-exhausted",
                        &agena_tui::fl_args!("max" => retry.max_retries.to_string()),
                    )
                } else {
                    i18n.text("error-part-stopped")
                }
            ),
        ),
        RetryOutcome::Cancelled => (
            "error-part-cancelled",
            PartExecutionStatusResource::Cancelled,
            format!("{count} · {}", i18n.text("error-part-stopped")),
        ),
    };
    RetryHeadline {
        title: i18n.text(title_key),
        status,
        progress,
    }
}

/// Cache clock for this entry only. Static history and completed retries keep
/// their rendered blocks; an expired backoff repaints once then stops ticking.
pub fn entry_retry_clock_second(entry: &TranscriptEntry, now_ms: i64) -> Option<i64> {
    entry.parts.iter().find_map(|part| {
        let retry = match &part.content {
            TranscriptPartContent::Activity(TranscriptActivityContent::Error(error)) => {
                error.retry.as_ref()
            }
            TranscriptPartContent::Activity(TranscriptActivityContent::Canonical(
                agena_domain::ActivityPayload::Error(error),
            )) => error.retry.as_ref(),
            _ => None,
        }?;
        let active = retry.outcome == RetryOutcome::Retrying
            && if matches!(
                retry.phase,
                Some(RetryPhase::Requesting | RetryPhase::Streaming)
            ) {
                retry.started_at_ms.is_some()
            } else {
                retry.next_at_ms.is_some_and(|next| next > now_ms)
            };
        active.then_some(now_ms.div_euclid(1000))
    })
}
