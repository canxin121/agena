//! JSONL export/import for a full session (gate 9 round-trip).
//!
//! Format: one JSON object per line. The first line is the session metadata,
//! every following line is one part. Both engines serialize through here so
//! exports are byte-identical across backends.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Part, SessionView, StoreError};
use agena_domain::{SessionLifecycleState, SessionRelationKind};

/// One line of the JSONL bundle.
///
/// `type` is used (not `kind`) so it does not collide with the `kind` field
/// carried by every `Part`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExportRecord {
    Meta {
        session_id: i64,
        parent_id: Option<i64>,
        depth: i64,
        root_id: i64,
        workspace_id: i64,
        relation_kind: SessionRelationKind,
        cutoff_part_id: Option<i64>,
        title: String,
        lifecycle_state: SessionLifecycleState,
        task_id: Option<String>,
        config_json: Option<Value>,
        provider_anchors_json: Option<Value>,
    },
    Part(Part),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_jsonl_rejects_unknown_meta_fields() {
        let line = serde_json::json!({
            "type":"meta",
            "session_id":1,
            "parent_id":null,
            "depth":0,
            "root_id":1,
            "workspace_id":1,
            "relation_kind":"root",
            "cutoff_part_id":null,
            "title":"fixture",
            "lifecycle_state":"ready",
            "task_id":null,
            "config_json":null,
            "provider_anchors_json":null,
            "obsolete":true
        });
        let bundle = format!("{}\n", serde_json::to_string(&line).unwrap());
        assert!(parse(&bundle).is_err());
    }

    #[test]
    fn current_jsonl_rejects_unknown_part_fields() {
        let part = serde_json::json!({
            "type":"part",
            "part_id":2,
            "kind":"text",
            "role":"user",
            "state":"completed",
            "content":{"text":"hello"},
            "summary":null,
            "visibility":"both",
            "parent_part_id":null,
            "run_id":1,
            "origin_session_id":1,
            "revision":0,
            "started_at_ms":1,
            "finished_at_ms":1,
            "created_at_ms":1,
            "updated_at_ms":1,
            "provider_state":null,
            "obsolete":true
        });
        assert!(serde_json::from_value::<ExportRecord>(part).is_err());
    }
}

/// Serialize a session view to the JSONL bundle.
pub fn serialize(view: &SessionView) -> Result<String, StoreError> {
    let meta = ExportRecord::Meta {
        session_id: view.meta.id,
        parent_id: view.meta.parent_id,
        depth: view.meta.depth,
        root_id: view.meta.root_id,
        workspace_id: view.meta.workspace_id,
        relation_kind: view.meta.relation_kind,
        cutoff_part_id: view.meta.cutoff_part_id,
        title: view.meta.title.clone(),
        lifecycle_state: view.meta.lifecycle_state,
        task_id: view.meta.task_id.clone(),
        config_json: view.meta.config_json.clone(),
        provider_anchors_json: view.meta.provider_anchors_json.clone(),
    };
    let mut out = push_line(meta)?;
    for part in &view.parts {
        out.push_str(&push_line(ExportRecord::Part(part.clone()))?);
    }
    Ok(out)
}

fn push_line(record: ExportRecord) -> Result<String, StoreError> {
    let mut line = serde_json::to_string(&record)
        .map_err(|error| StoreError::Serialization(format!("encode JSONL record: {error}")))?;
    line.push('\n');
    Ok(line)
}

/// A parsed JSONL bundle: the metadata line plus the ordered parts.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedBundle {
    pub session_id: i64,
    pub title: String,
    pub task_id: Option<String>,
    pub config_json: Option<Value>,
    pub provider_anchors_json: Option<Value>,
    pub parts: Vec<Part>,
}

/// Parse a JSONL bundle produced by [`serialize`].
pub fn parse(bundle: &str) -> Result<ParsedBundle, StoreError> {
    let mut meta: Option<ExportRecord> = None;
    let mut parts = Vec::new();
    for (index, line) in bundle.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let record: ExportRecord = serde_json::from_str(line).map_err(|error| {
            StoreError::Serialization(format!("decode JSONL line {}: {error}", index + 1))
        })?;
        match record {
            ExportRecord::Meta { .. } if meta.is_none() => {
                meta = Some(record);
            }
            ExportRecord::Part(part) => parts.push(part),
            other => {
                return Err(StoreError::Serialization(format!(
                    "unexpected JSONL record at line {}: {other:?}",
                    index + 1
                )));
            }
        }
    }
    let Some(ExportRecord::Meta {
        session_id,
        title,
        task_id,
        config_json,
        provider_anchors_json,
        ..
    }) = meta
    else {
        return Err(StoreError::Serialization(
            "JSONL bundle is missing its meta line".to_owned(),
        ));
    };
    Ok(ParsedBundle {
        session_id,
        title,
        task_id,
        config_json,
        provider_anchors_json,
        parts,
    })
}
