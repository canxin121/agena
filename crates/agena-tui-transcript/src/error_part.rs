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

#[cfg(test)]
mod tests {
    use super::*;

    fn progress() -> RetryProgress {
        RetryProgress {
            attempt: 2,
            max_retries: 10,
            next_at_ms: Some(15_001),
            phase: Some(RetryPhase::Backoff),
            started_at_ms: None,
            outcome: RetryOutcome::Retrying,
        }
    }

    fn plain(text: &str) -> String {
        text.replace(['\u{2068}', '\u{2069}'], "")
    }

    fn entry(retry: RetryProgress) -> TranscriptEntry<'static> {
        let problem = agena_failure::Failure::new(
            agena_failure::FailureCode::new("provider.timeout"),
            agena_failure::FailureCategory::Timeout,
            agena_failure::FailureResponsibility::Dependency,
            agena_failure::RetryDirective::Backoff,
            agena_failure::RecoveryDirective::Retry,
            agena_failure::FailureImpact::OperationFailed,
            agena_failure::UserPresentation::new(
                "timeout",
                "请求超时，供应商尚未返回响应。 This is a long provider failure reason.",
            ),
        )
        .into();
        TranscriptEntry {
            id: crate::TranscriptEntryId::StoredMessage(7),
            reply_id: None,
            role: None,
            state: agena_api::resource::RunStatus::InProgress,
            created_at: chrono::Utc::now(),
            parts: vec![crate::TranscriptEntryPart {
                id: crate::TranscriptContentId::StoredPart(21),
                status: PartExecutionStatusResource::Failed,
                content: TranscriptPartContent::Activity(TranscriptActivityContent::Error(
                    agena_api::part::ErrorPartResource {
                        problem,
                        retry: Some(retry),
                    },
                )),
            }],
        }
    }

    #[test]
    fn retry_entry_clock_expires_or_follows_server_phase_and_outcome() {
        let mut retry = progress();
        assert_eq!(
            entry_retry_clock_second(&entry(retry.clone()), 12_000),
            Some(12)
        );
        assert_eq!(
            entry_retry_clock_second(&entry(retry.clone()), 15_001),
            None
        );
        retry.phase = Some(RetryPhase::Requesting);
        retry.started_at_ms = Some(15_001);
        assert_eq!(
            entry_retry_clock_second(&entry(retry.clone()), 27_000),
            Some(27)
        );
        for outcome in [
            RetryOutcome::Recovered,
            RetryOutcome::Failed,
            RetryOutcome::Cancelled,
        ] {
            retry.outcome = outcome;
            assert_eq!(
                entry_retry_clock_second(&entry(retry.clone()), 27_000),
                None
            );
        }
    }

    #[test]
    fn collapsed_and_expanded_error_headers_keep_progress_in_the_normal_part_at_narrow_widths() {
        use unicode_width::UnicodeWidthStr;
        for locale in [
            "en-US", "zh-CN", "zh-TW", "de-DE", "es-ES", "fr-FR", "ja-JP", "ko-KR",
        ] {
            let i18n = I18n::resolve(Some(locale), None);
            let mut retry = progress();
            retry.next_at_ms = Some(chrono::Utc::now().timestamp_millis() + 60_000);
            let entry = entry(retry);
            for width in [12, 24, 40, 80] {
                for expanded in [false, true] {
                    let rendered = crate::render_entry_detailed(
                        &entry,
                        width,
                        &i18n,
                        &crate::TranscriptDetailDefaults {
                            activity_default_expanded: expanded,
                            kind_defaults: Default::default(),
                        },
                        &Default::default(),
                    );
                    let headline = rendered
                        .nodes
                        .iter()
                        .find(|node| {
                            matches!(
                                &node.key,
                                crate::TranscriptNodeKey::Activity {
                                    content_id: crate::TranscriptContentId::StoredPart(21),
                                    ..
                                }
                            )
                        })
                        .unwrap();
                    assert!(headline.toggleable);
                    assert_eq!(headline.expanded, expanded);
                    assert!(
                        headline.end_line > headline.start_line + 1,
                        "progress belongs to the Part headline"
                    );
                    let rows = &rendered.lines[headline.start_line..headline.end_line];
                    for row in rows {
                        assert!(
                            UnicodeWidthStr::width(row.text.as_str()) <= width as usize,
                            "{locale} at {width}: {:?}",
                            row.text
                        );
                        assert!(!row.text.contains("Unknown localization key"));
                    }
                    let progress_text = plain(
                        &rows[1..]
                            .iter()
                            .map(|row| row.copy_text.as_str())
                            .collect::<Vec<_>>()
                            .join(""),
                    );
                    assert!(progress_text.contains("2/10"), "{locale}: {progress_text}");
                    assert_eq!(
                        rendered
                            .nodes
                            .iter()
                            .filter(|node| matches!(
                                node.key,
                                crate::TranscriptNodeKey::Activity { .. }
                            ))
                            .count(),
                        1
                    );
                }
            }
        }
    }

    #[test]
    fn retry_clock_never_claims_a_request_started_from_a_deadline() {
        let i18n = I18n::english();
        let mut retry = progress();
        let waiting = retry_headline(&retry, 12_000, &i18n);
        assert_eq!(waiting.title, "Waiting to retry");
        assert!(plain(&waiting.progress).contains("Retry 2/10 · Retry in 4s"));
        assert!(plain(&retry_headline(&retry, 14_001, &i18n).progress).contains("Retry in 1s"));
        assert!(
            plain(&retry_headline(&retry, 15_001, &i18n).progress)
                .contains("Waiting for retry to start")
        );
        retry.phase = None; // JSON written by older servers.
        assert_eq!(
            retry_headline(&retry, 90_000, &i18n).status,
            PartExecutionStatusResource::Pending
        );
        retry.phase = Some(RetryPhase::Requesting);
        retry.next_at_ms = None;
        retry.started_at_ms = Some(15_001);
        let requesting = retry_headline(&retry, 27_001, &i18n);
        assert_eq!(requesting.title, "Retrying");
        assert_eq!(requesting.status, PartExecutionStatusResource::InProgress);
        assert!(plain(&requesting.progress).contains("Waiting for response · 12s elapsed"));
        retry.phase = Some(RetryPhase::Streaming);
        assert!(
            plain(&retry_headline(&retry, 27_001, &i18n).progress).contains("Receiving response")
        );
        assert!(plain(&retry_headline(&retry, 10_000, &i18n).progress).contains("0s elapsed"));
    }

    #[test]
    fn retry_terminal_outcomes_have_clear_history_labels() {
        let i18n = I18n::english();
        let mut retry = progress();
        for (outcome, title, text) in [
            (
                RetryOutcome::Recovered,
                "Recovered",
                "Recovered after 2 retries",
            ),
            (
                RetryOutcome::Failed,
                "Retry failed",
                "Automatic retry stopped",
            ),
            (
                RetryOutcome::Cancelled,
                "Retry cancelled",
                "Automatic retry stopped",
            ),
        ] {
            retry.outcome = outcome;
            let headline = retry_headline(&retry, 12_000, &i18n);
            assert_eq!(headline.title, title);
            assert!(plain(&headline.progress).contains(text));
            assert!(!headline.progress.contains("Retry in"));
        }
        retry.outcome = RetryOutcome::Failed;
        retry.attempt = 10;
        assert!(
            plain(&retry_headline(&retry, 12_000, &i18n).progress).contains("All 10 retries used")
        );
        assert_eq!(plain(&duration(65, &i18n)), "1m 5s");
        assert_eq!(plain(&duration(3661, &i18n)), "1h 1m");
    }
}
