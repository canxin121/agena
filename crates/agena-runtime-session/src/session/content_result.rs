//! One canonical resource path for large non-streaming result fields.

use agena_domain::{ContentField, ContentFormat, ContentKind, ContentState, RawOutput};
use agena_storage::content::ContentHub;
use serde_json::Value;

const INLINE_RESULT_BYTES: usize = 64 * 1024;
const INLINE_FIELD_BYTES: usize = 32 * 1024;
const MAX_RESULT_FIELDS: usize = 32;
static RESULT_WORKERS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

/// Archive a provider diagnostic once at completion. This is independent of
/// opaque continuation state, which remains an authoritative run fact.
pub(crate) async fn archive_provider_trace(
    hub: &ContentHub,
    session_id: i64,
    part_id: i64,
    raw: Value,
) -> Result<agena_domain::ContentRef, crate::AppError> {
    let body = RESULT_WORKERS
        .run(move || serde_json::to_string(&raw).expect("JSON serializes"))
        .await
        .map_err(|error| {
            crate::AppError::Internal(format!("provider trace worker failed: {error}"))
        })?;
    let writer = hub
        .open(session_id, part_id, ContentKind::Text)
        .await
        .map_err(|error| crate::AppError::Internal(format!("open provider trace: {error}")))?;
    if let Err(error) = writer.append_text(&body).await {
        writer.record_loss(body.len(), &error);
    }
    Ok(writer
        .finalize(ContentState::Complete)
        .await
        .map_err(|error| crate::AppError::Internal(format!("finalize provider trace: {error}")))?
        .reference())
}

struct FieldBody {
    pointer: String,
    format: ContentFormat,
    body: String,
}

fn extracted_fields(output: &mut RawOutput) -> Vec<FieldBody> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(bytes.len());
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    fn bytes(value: &Value) -> usize {
        let mut counter = Counter(0);
        serde_json::to_writer(&mut counter, value).expect("JSON values serialize");
        counter.0
    }
    fn root(output: &mut RawOutput) -> Vec<FieldBody> {
        let value = std::mem::replace(
            output.payload.as_mut().expect("result payload exists"),
            Value::Null,
        );
        let (body, format) = match value {
            Value::String(body) => (body, ContentFormat::Plain),
            value => (
                serde_json::to_string(&value).expect("JSON value serializes"),
                ContentFormat::Json,
            ),
        };
        vec![FieldBody {
            pointer: String::new(),
            format,
            body,
        }]
    }
    let Some(payload) = output.payload.as_ref() else {
        return vec![];
    };
    let Some(object) = payload.as_object() else {
        return if bytes(payload) > INLINE_FIELD_BYTES {
            root(output)
        } else {
            vec![]
        };
    };
    // A bounded result cannot create thousands of field references or retain
    // an arbitrarily large map made entirely of individually small facts.
    let control_bytes = object
        .iter()
        .map(|(key, value)| {
            let size = bytes(value);
            key.len() + if size < 1024 { size } else { 4 } + 4
        })
        .sum::<usize>();
    if control_bytes > INLINE_RESULT_BYTES
        || (object.len() > MAX_RESULT_FIELDS && bytes(payload) > INLINE_RESULT_BYTES)
    {
        return root(output);
    }
    let payload = output
        .payload
        .as_mut()
        .and_then(Value::as_object_mut)
        .expect("object result");
    let mut sizes = payload
        .iter()
        .map(|(key, value)| (key.clone(), bytes(value)))
        .collect::<Vec<_>>();
    sizes.sort_unstable_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    let mut inline = sizes
        .iter()
        .map(|(key, size)| key.len() + size + 4)
        .sum::<usize>();
    let mut fields = vec![];
    for (key, size) in sizes {
        if size <= INLINE_FIELD_BYTES && inline <= INLINE_RESULT_BYTES {
            break;
        }
        // Small control facts (exit codes, cursors and identities) stay inline.
        if size < 1024 {
            continue;
        }
        let value = std::mem::replace(
            payload.get_mut(&key).expect("result field exists"),
            Value::Null,
        );
        let (body, format) = match value {
            Value::String(body) => {
                let format = match key.as_str() {
                    "diff" => ContentFormat::Diff,
                    "markdown" | "project_instructions" => ContentFormat::Markdown,
                    "code" => ContentFormat::Code,
                    _ => ContentFormat::Plain,
                };
                (body, format)
            }
            value => (
                serde_json::to_string(&value).expect("JSON value serializes"),
                ContentFormat::Json,
            ),
        };
        fields.push(FieldBody {
            pointer: format!("/{}", key.replace('~', "~0").replace('/', "~1")),
            format,
            body,
        });
        inline = inline.saturating_sub(size).saturating_add(4);
    }
    fields
}

pub(crate) async fn externalize_result(
    hub: &ContentHub,
    session_id: i64,
    part_id: i64,
    mut output: RawOutput,
) -> Result<RawOutput, crate::AppError> {
    let (mut output, bodies) = RESULT_WORKERS
        .run(move || {
            let bodies = extracted_fields(&mut output);
            (output, bodies)
        })
        .await
        .map_err(|error| {
            crate::AppError::Internal(format!("result content worker failed: {error}"))
        })?;
    for field in bodies {
        let stored = async {
            let writer = hub.open(session_id, part_id, ContentKind::Text).await?;
            let write = writer.append_text(&field.body).await;
            if let Err(error) = &write {
                writer.record_loss(field.body.len(), error);
            }
            let resource = writer.finalize(ContentState::Complete).await?;
            Ok::<_, agena_storage::store::StoreError>((
                resource.reference(),
                resource.capture_error,
            ))
        }
        .await;
        let error = match stored {
            Ok((resource, error)) => {
                output.fields.push(ContentField {
                    pointer: field.pointer,
                    resource,
                    format: field.format,
                });
                error
            }
            Err(error) => Some(error.to_string()),
        };
        if let Some(error) = error {
            output.truncated = true;
            if let Some(payload) = output.payload.as_mut().and_then(Value::as_object_mut) {
                payload.insert("output_capture_error".into(), Value::String(error));
                payload.insert(
                    "output_capture_state".into(),
                    Value::String("interrupted".into()),
                );
            }
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn many_small_fields_and_large_scalar_results_have_bounded_envelopes() {
        let hub = ContentHub::in_memory();
        let map = (0..5000)
            .map(|i| (format!("field_{i}"), serde_json::json!("x".repeat(30))))
            .collect::<serde_json::Map<_, _>>();
        for payload in [
            Value::Object(map),
            serde_json::json!("中文".repeat(20000)),
            serde_json::json!((0..20000).collect::<Vec<_>>()),
        ] {
            let output = externalize_result(
                &hub,
                1,
                2,
                RawOutput {
                    payload: Some(payload.clone()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            assert_eq!(output.fields.len(), 1);
            assert!(output.fields[0].pointer.is_empty());
            assert_eq!(output.payload, Some(Value::Null));
            assert!(serde_json::to_vec(&output).unwrap().len() < 1024);
            let body = hub
                .read_text(output.fields[0].resource.resource_id, 1024 * 1024)
                .await
                .unwrap();
            let restored = if output.fields[0].format == ContentFormat::Json {
                serde_json::from_str(&body.text).unwrap()
            } else {
                Value::String(body.text)
            };
            assert_eq!(restored, payload);
        }
    }

    #[tokio::test]
    async fn large_diff_and_json_fields_use_the_same_resources_without_duplicate_bodies() {
        let hub = ContentHub::in_memory();
        let diff = format!(
            "--- a\n+++ b\n@@ -1 +1 @@\n-{}\n+{}\n",
            "old".repeat(12000),
            "new".repeat(12000)
        );
        let rows = (0..3000)
            .map(|n| serde_json::json!({"id": n, "value": "result row"}))
            .collect::<Vec<_>>();
        let input = RawOutput {
            payload: Some(
                serde_json::json!({"exit_code": 0, "diff": diff, "rows": rows, "a/b~": "x".repeat(40000)}),
            ),
            ..Default::default()
        };
        let output = externalize_result(&hub, 1, 2, input).await.unwrap();
        assert!(serde_json::to_vec(&output).unwrap().len() < 4096);
        assert_eq!(output.payload.as_ref().unwrap()["exit_code"], 0);
        assert_eq!(output.fields.len(), 3);
        assert!(!output.truncated);
        let diff_field = output
            .fields
            .iter()
            .find(|field| field.pointer == "/diff")
            .unwrap();
        assert_eq!(diff_field.format, ContentFormat::Diff);
        assert_eq!(
            hub.read_text(diff_field.resource.resource_id, 512 * 1024)
                .await
                .unwrap()
                .text,
            diff
        );
        let rows_field = output
            .fields
            .iter()
            .find(|field| field.pointer == "/rows")
            .unwrap();
        assert_eq!(rows_field.format, ContentFormat::Json);
        assert_eq!(
            serde_json::from_str::<Value>(
                &hub.read_text(rows_field.resource.resource_id, 512 * 1024)
                    .await
                    .unwrap()
                    .text
            )
            .unwrap(),
            serde_json::json!(rows)
        );
        assert!(output.fields.iter().any(|field| field.pointer == "/a~1b~0"));
        for field in &output.fields {
            let resource = hub.describe(field.resource.resource_id).await.unwrap();
            assert_eq!((resource.owner_session_id, resource.part_id), (1, 2));
            assert_eq!(resource.state, ContentState::Complete);
        }
    }

    #[tokio::test]
    async fn model_hydration_restores_result_fields_and_document_state_without_mutating_facts() {
        use agena_domain::{ContentInput, DocumentMutation, ToolResultState};
        use agena_runtime_contracts::part_content::ToolCallContent;
        use agena_storage::store::{Part, PartRole, PartState, PartVisibility};
        let hub = ContentHub::in_memory();
        let diff = format!("--- a\n+++ b\n+{}", "line".repeat(12000));
        let output = externalize_result(
            &hub,
            1,
            2,
            RawOutput {
                payload: Some(serde_json::json!({"diff": diff, "exit_code": 0})),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let writer = hub.open(1, 2, ContentKind::Document).await.unwrap();
        for completed in 1..=2 {
            writer
                .append(ContentInput::Structured {
                    event: DocumentMutation::Progress {
                        block_id: "scan".into(),
                        phase: "scan".into(),
                        completed,
                        total: Some(2),
                        unit: None,
                    },
                })
                .await
                .unwrap();
        }
        let document = writer
            .finalize(ContentState::Complete)
            .await
            .unwrap()
            .reference();
        let mut references = output
            .fields
            .iter()
            .map(|field| field.resource.clone())
            .collect::<Vec<_>>();
        references.push(document);
        let content = ToolCallContent {
            name: "test.render".into(),
            input: serde_json::json!({}),
            call_id: 2,
            state: ToolResultState::Completed,
            output: Some(output),
            resources: references,
            ..Default::default()
        }
        .as_value();
        let now = chrono::Utc::now();
        let part = Part {
            part_id: 2,
            kind: "tool_call".into(),
            role: PartRole::Assistant,
            state: PartState::Completed,
            content: content.clone(),
            summary: None,
            visibility: PartVisibility::Both,
            parent_part_id: None,
            run_id: None,
            origin_session_id: 1,
            revision: 1,
            started_at_ms: now.timestamp_millis(),
            finished_at_ms: Some(now.timestamp_millis()),
            created_at_ms: now.timestamp_millis(),
            updated_at_ms: now.timestamp_millis(),
            provider_state: None,
        };
        let mut session = crate::Session::new(1, 1, "test", now);
        session.install_projected_parts(vec![part]);
        let projected = crate::session::prompt_window::resolve_content_for_model(&session, &hub)
            .await
            .unwrap();
        assert_eq!(session.parts[0].content, content);
        assert_eq!(
            projected.parts[0].content["output"]["payload"]["diff"],
            diff
        );
        assert_eq!(
            projected.parts[0].content["output"]["payload"]["exit_code"],
            0
        );
        let stream = projected.parts[0].content["output"]["payload"]["streamed_content"]
            .as_str()
            .unwrap();
        assert!(stream.contains("\"completed\":2"));
        assert!(
            !stream.contains("\"completed\":1"),
            "the model receives the current document rather than the event log"
        );
        assert_eq!(
            projected.parts[0].content["resources"],
            content["resources"]
        );
    }
}
