//! Share and summary helpers for recorded rollouts.
//!
//! The reader/recorder pair handles raw JSONL persistence; this module
//! turns a rollout into something a UI or HTTP `/share/:id` route can
//! serve. Two surfaces:
//!
//! - [`SessionSummary`] — a fast scan-once summary suitable for a resume
//!   picker (frame count, last activity, tool-call count, model id).
//! - [`ShareBundle`] — the full conversation trace as a single JSON
//!   object, with optional path redaction so a snapshot can be posted to
//!   a coworker without leaking absolute home directories.
//!
//! Both types stay *agnostic* of agena's internal Message representation:
//! they only know about [`RolloutFrame`] / [`RolloutKind`], which is the
//! whole point of the rollout schema.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{RolloutError, RolloutResult};
use crate::frame::{RolloutFrame, RolloutKind, SessionMeta};
use crate::reader::RolloutReader;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub agena_version: String,
    pub started_at: DateTime<Utc>,
    pub last_event_at: DateTime<Utc>,
    pub frame_count: u64,
    pub tool_call_count: u64,
    pub assistant_message_count: u64,
    pub user_message_count: u64,
    /// Best-guess model id pulled from `SessionMeta.context.model_id`.
    pub model_id: Option<String>,
    /// Source file we summarized.
    pub source_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareBundle {
    pub schema_version: u32,
    pub summary: SessionSummary,
    pub meta: SessionMeta,
    pub frames: Vec<RolloutFrame>,
}

const SCHEMA_VERSION: u32 = 1;

/// Walk a rollout file and produce a one-pass summary.
pub fn summarize_session(path: impl AsRef<Path>) -> RolloutResult<SessionSummary> {
    let reader = RolloutReader::open(path.as_ref());
    let frames = reader.read_all()?;
    summarize_frames(path.as_ref(), &frames)
}

fn summarize_frames(source: &Path, frames: &[RolloutFrame]) -> RolloutResult<SessionSummary> {
    let first = frames
        .first()
        .ok_or_else(|| RolloutError::Malformed("rollout has no frames".into()))?;
    let RolloutKind::SessionMeta(meta) = &first.kind else {
        return Err(RolloutError::Malformed(
            "first frame must be SessionMeta".into(),
        ));
    };
    let last = frames.last().expect("checked first → has at least 1");

    let mut tool_call_count = 0u64;
    let mut assistant_message_count = 0u64;
    let mut user_message_count = 0u64;
    for frame in frames {
        match &frame.kind {
            RolloutKind::ToolCall { .. } => tool_call_count += 1,
            RolloutKind::AssistantMessage { .. } => assistant_message_count += 1,
            RolloutKind::UserMessage { .. } => user_message_count += 1,
            _ => {}
        }
    }

    let model_id = meta
        .context
        .as_object()
        .and_then(|o| o.get("model_id"))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    Ok(SessionSummary {
        session_id: meta.session_id.clone(),
        agena_version: meta.agena_version.clone(),
        started_at: first.ts,
        last_event_at: last.ts,
        frame_count: frames.len() as u64,
        tool_call_count,
        assistant_message_count,
        user_message_count,
        model_id,
        source_path: source.to_path_buf(),
    })
}

/// Build a shareable bundle from a session file. With `redact_paths` set,
/// any string field that begins with one of the workspace prefixes (or
/// the user's home directory) is rewritten to `~/<rest>` so the snapshot
/// does not leak absolute paths.
pub fn share_bundle(path: impl AsRef<Path>, opts: ShareOptions) -> RolloutResult<ShareBundle> {
    let path = path.as_ref();
    let reader = RolloutReader::open(path);
    let mut frames = reader.read_all()?;
    let summary = summarize_frames(path, &frames)?;
    let RolloutKind::SessionMeta(mut meta) = frames[0].kind.clone() else {
        return Err(RolloutError::Malformed(
            "first frame must be SessionMeta".into(),
        ));
    };

    if opts.redact_paths {
        let mut redactor = Redactor::new(opts.redact_prefixes);
        redactor.redact_value(&mut meta.context);
        for frame in &mut frames {
            redact_kind(&mut frame.kind, &mut redactor);
        }
    }

    Ok(ShareBundle {
        schema_version: SCHEMA_VERSION,
        summary,
        meta,
        frames,
    })
}

/// Knobs for [`share_bundle`].
#[derive(Debug, Clone, Default)]
pub struct ShareOptions {
    pub redact_paths: bool,
    /// Extra absolute prefixes to rewrite to `~/...`. The user's home
    /// directory (when discoverable) is added automatically.
    pub redact_prefixes: Vec<PathBuf>,
}

struct Redactor {
    prefixes: Vec<(String, String)>,
}

impl Redactor {
    fn new(extra: Vec<PathBuf>) -> Self {
        let mut prefixes: Vec<(String, String)> = Vec::new();
        if let Some(home) = home_dir() {
            prefixes.push((normalize(&home), "~".to_string()));
        }
        for p in extra {
            prefixes.push((normalize(&p), "~/<workspace>".to_string()));
        }
        // Longest first so we replace `/home/u/p/foo` before `/home/u`.
        prefixes.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        Self { prefixes }
    }

    fn redact_str(&self, s: &str) -> String {
        let mut out = s.to_string();
        for (from, to) in &self.prefixes {
            if out.contains(from) {
                out = out.replace(from, to);
            }
        }
        out
    }

    fn redact_value(&self, v: &mut Value) {
        match v {
            Value::String(s) => {
                let updated = self.redact_str(s);
                *s = updated;
            }
            Value::Array(items) => items.iter_mut().for_each(|i| self.redact_value(i)),
            Value::Object(map) => {
                for (_, child) in map.iter_mut() {
                    self.redact_value(child);
                }
            }
            _ => {}
        }
    }
}

fn redact_kind(kind: &mut RolloutKind, redactor: &mut Redactor) {
    match kind {
        RolloutKind::SessionMeta(meta) => redactor.redact_value(&mut meta.context),
        RolloutKind::UserMessage { parts } | RolloutKind::AssistantMessage { parts } => {
            redactor.redact_value(parts)
        }
        RolloutKind::ToolCall { args, .. } => redactor.redact_value(args),
        RolloutKind::ToolResult { output, error, .. } => {
            redactor.redact_value(output);
            if let Some(err) = error.as_mut() {
                *err = redactor.redact_str(err);
            }
        }
        RolloutKind::Permission { request, decision } => {
            redactor.redact_value(request);
            redactor.redact_value(decision);
        }
        RolloutKind::PlanEntered { file_path, .. } => *file_path = redactor.redact_str(file_path),
        RolloutKind::PlanExited { .. } => {}
        RolloutKind::PluginEvent { payload, .. } => redactor.redact_value(payload),
    }
}

fn normalize(path: &Path) -> String {
    let mut s = path.to_string_lossy().to_string();
    while s.ends_with('/') && s.len() > 1 {
        s.pop();
    }
    s
}

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

/// Convenience: summarize every JSONL under `root`. Failures are logged
/// and skipped — a single corrupt file must not hide the rest.
pub fn summarize_directory(root: impl AsRef<Path>) -> Vec<SessionSummary> {
    let mut out = Vec::new();
    for path in crate::reader::list_sessions(root) {
        match summarize_session(&path) {
            Ok(summary) => out.push(summary),
            Err(err) => {
                tracing::warn!(
                    target: "agena_rollout::share",
                    "failed to summarize {}: {err}",
                    path.display()
                );
            }
        }
    }
    out.sort_by_key(|s| std::cmp::Reverse(s.last_event_at));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_jsonl(label: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("agena-rollout-{label}-{suffix}.jsonl"))
    }

    fn frame(seq: u64, kind: RolloutKind) -> String {
        let frame = RolloutFrame {
            seq,
            ts: Utc::now(),
            kind,
        };
        serde_json::to_string(&frame).unwrap()
    }

    fn write_session(path: &Path) {
        let lines = [
            frame(
                1,
                RolloutKind::SessionMeta(SessionMeta {
                    session_id: "abc-1".into(),
                    agena_version: "0.1.0".into(),
                    context: serde_json::json!({"model_id": "claude-haiku-4-5"}),
                }),
            ),
            frame(
                2,
                RolloutKind::UserMessage {
                    parts: serde_json::json!([{"text": "ls /home/alice/project"}]),
                },
            ),
            frame(
                3,
                RolloutKind::ToolCall {
                    call_id: "c1".into(),
                    name: "bash".into(),
                    args: serde_json::json!({"command": "ls /home/alice/project"}),
                },
            ),
            frame(
                4,
                RolloutKind::ToolResult {
                    call_id: "c1".into(),
                    output: serde_json::json!({"stdout": "found at /home/alice/project/src"}),
                    duration_ms: 12,
                    error: None,
                },
            ),
            frame(
                5,
                RolloutKind::AssistantMessage {
                    parts: serde_json::json!([{"text": "done"}]),
                },
            ),
        ];
        std::fs::write(path, lines.join("\n") + "\n").unwrap();
    }

    #[test]
    fn summarize_session_counts_frames_by_kind() {
        let path = tmp_jsonl("summary");
        write_session(&path);
        let summary = summarize_session(&path).unwrap();
        assert_eq!(summary.session_id, "abc-1");
        assert_eq!(summary.agena_version, "0.1.0");
        assert_eq!(summary.frame_count, 5);
        assert_eq!(summary.tool_call_count, 1);
        assert_eq!(summary.user_message_count, 1);
        assert_eq!(summary.assistant_message_count, 1);
        assert_eq!(summary.model_id.as_deref(), Some("claude-haiku-4-5"));
        assert_eq!(summary.source_path, path);
    }

    #[test]
    fn summarize_session_errors_on_empty_file() {
        let path = tmp_jsonl("empty");
        std::fs::write(&path, "").unwrap();
        let err = summarize_session(&path).unwrap_err();
        assert!(matches!(err, RolloutError::Malformed(_)));
    }

    #[test]
    fn share_bundle_round_trips_and_keeps_paths_when_redact_off() {
        let path = tmp_jsonl("share-no-redact");
        write_session(&path);
        let bundle = share_bundle(&path, ShareOptions::default()).unwrap();
        assert_eq!(bundle.schema_version, SCHEMA_VERSION);
        assert_eq!(bundle.frames.len(), 5);
        let json = serde_json::to_string(&bundle).unwrap();
        assert!(json.contains("/home/alice/project"));
    }

    #[test]
    fn share_bundle_redacts_explicit_prefixes() {
        let path = tmp_jsonl("share-redact");
        write_session(&path);
        let bundle = share_bundle(
            &path,
            ShareOptions {
                redact_paths: true,
                redact_prefixes: vec![PathBuf::from("/home/alice/project")],
            },
        )
        .unwrap();
        let json = serde_json::to_string(&bundle).unwrap();
        assert!(!json.contains("/home/alice/project"));
        assert!(json.contains("<workspace>"));
    }

    #[test]
    fn summarize_directory_orders_by_last_event_descending() {
        let dir = std::env::temp_dir().join(format!(
            "agena-rollout-dir-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("one.jsonl");
        let b = dir.join("two.jsonl");
        write_session(&a);
        std::thread::sleep(std::time::Duration::from_millis(20));
        write_session(&b);
        let summaries = summarize_directory(&dir);
        assert_eq!(summaries.len(), 2);
        assert!(summaries[0].last_event_at >= summaries[1].last_event_at);
    }

    #[test]
    fn home_dir_redaction_rewrites_absolute_home_paths() {
        // Synthesize a frame referencing the user's actual HOME so we
        // verify the auto-discovery branch works under default options.
        let Some(home) = home_dir() else {
            return;
        };
        let leaked = format!("{}/private", home.display());
        let path = tmp_jsonl("share-home");
        let line = frame(
            1,
            RolloutKind::SessionMeta(SessionMeta {
                session_id: "h-1".into(),
                agena_version: "0.1.0".into(),
                context: serde_json::json!({"workspace": leaked}),
            }),
        );
        std::fs::write(&path, line + "\n").unwrap();
        let bundle = share_bundle(
            &path,
            ShareOptions {
                redact_paths: true,
                ..Default::default()
            },
        )
        .unwrap();
        let json = serde_json::to_string(&bundle).unwrap();
        assert!(!json.contains(home.to_string_lossy().as_ref()));
        assert!(json.contains("~/private") || json.contains("\"~"));
    }
}
