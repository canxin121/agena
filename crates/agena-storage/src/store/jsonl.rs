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
    Content {
        archive: Box<crate::content::ContentArchive>,
    },
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

pub(crate) fn append_resource(
    out: &mut String,
    archive: crate::content::ContentArchive,
) -> Result<(), StoreError> {
    out.push_str(&push_line(ExportRecord::Content {
        archive: Box::new(archive),
    })?);
    Ok(())
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
    pub resources: Vec<crate::content::ContentArchive>,
}

/// Parse a JSONL bundle produced by [`serialize`].
pub fn parse(bundle: &str) -> Result<ParsedBundle, StoreError> {
    if bundle.len() > 512 * 1024 * 1024 {
        return Err(StoreError::Constraint(
            "session bundle exceeds its byte budget".into(),
        ));
    }
    let mut meta: Option<ExportRecord> = None;
    let mut parts = Vec::new();
    let mut resources = Vec::new();
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
            ExportRecord::Part(part) if meta.is_some() => parts.push(part),
            ExportRecord::Content { archive } if meta.is_some() => resources.push(*archive),
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
        resources,
    })
}

/// Validate the complete bundle before publishing anything. Resource identity
/// belongs to the owning Part; import creates a fresh owner and generation.
pub fn prepare_resource_import(
    parts: &mut [Part],
    archives: Vec<crate::content::ContentArchive>,
    session_id: i64,
    id_map: &std::collections::HashMap<i64, i64>,
) -> Result<Vec<crate::content::ContentArchive>, StoreError> {
    use std::collections::HashMap;
    let mut expected = HashMap::new();
    for part in parts.iter() {
        for reference in part.resources()? {
            if expected
                .insert(
                    reference.resource_id,
                    (part.part_id, part.origin_session_id, reference.kind),
                )
                .is_some()
            {
                return Err(StoreError::Constraint(
                    "a content resource must have exactly one owning Part".into(),
                ));
            }
        }
        for target in [part.run_id, part.parent_part_id].into_iter().flatten() {
            if !id_map.contains_key(&target) {
                return Err(StoreError::Constraint(
                    "imported Part refers to a Part outside the bundle".into(),
                ));
            }
        }
    }
    let mut restored = Vec::with_capacity(archives.len());
    let mut resources = HashMap::new();
    for mut archive in archives {
        archive.validate()?;
        let id = archive.resource.resource_id;
        let Some((part_id, origin, kind)) = expected.remove(&id) else {
            return Err(StoreError::Constraint(
                "unreferenced or duplicate content archive".into(),
            ));
        };
        if archive.resource.part_id != part_id
            || archive.resource.owner_session_id != origin
            || archive.resource.kind != kind
        {
            return Err(StoreError::Constraint(
                "content archive differs from its Part owner".into(),
            ));
        }
        let new_id = agena_domain::ContentId::new();
        archive.remap(new_id, session_id, id_map[&part_id]);
        resources.insert(id, new_id);
        restored.push(archive);
    }
    if !expected.is_empty() {
        return Err(StoreError::Constraint(
            "session bundle is missing referenced content archives".into(),
        ));
    }
    fn rewrite(
        value: &mut Value,
        resources: &HashMap<agena_domain::ContentId, agena_domain::ContentId>,
    ) {
        if value.as_object().is_some_and(|object| {
            object.len() == 2 && object.contains_key("resource_id") && object.contains_key("kind")
        }) && let Ok(mut reference) =
            serde_json::from_value::<agena_domain::ContentRef>(value.clone())
        {
            if let Some(id) = resources.get(&reference.resource_id) {
                reference.resource_id = *id;
                *value = serde_json::to_value(reference).expect("resource reference serializes");
            }
            return;
        }
        match value {
            Value::Array(values) => {
                for value in values {
                    rewrite(value, resources);
                }
            }
            Value::Object(values) => {
                for value in values.values_mut() {
                    rewrite(value, resources);
                }
            }
            _ => {}
        }
    }
    for part in parts {
        if let Some(value) = part.content.get_mut("resources") {
            rewrite(value, &resources);
        }
        if let Some(value) = part.content.get_mut("output") {
            rewrite(value, &resources);
        }
        if let Some(value) = part.content.pointer_mut("/metadata/agena.provider_trace") {
            rewrite(value, &resources);
        }
    }
    Ok(restored)
}
