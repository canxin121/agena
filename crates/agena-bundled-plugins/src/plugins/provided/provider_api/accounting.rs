use super::*;
use agena_provider::{AttributedCompletionUsage, PROVIDER_TOOL_USAGE_METADATA_KEY};

// Only terminal observations are claimed. The atomic marker is shared by
// inference/create and response/get so polling never counts an inference twice.
pub(super) async fn metadata(
    service: &Service<'_>,
    family: Family,
    input: &ResourceInput,
    response: &Value,
    warnings: &mut Vec<String>,
) -> Result<BTreeMap<String, String>> {
    if !matches!(
        family,
        Family::Inference
            | Family::Tasks
            | Family::Images
            | Family::Transcription
            | Family::Speech
            | Family::Video
            | Family::Music
            | Family::Embeddings
            | Family::Moderation
            | Family::Agents
            | Family::Batches
    ) {
        return Ok(BTreeMap::new());
    }
    let native = response.get("response").unwrap_or(response);
    let raw = if native.get("done").and_then(Value::as_bool) == Some(true) {
        native.get("response").unwrap_or(native)
    } else {
        native
    };
    let stream = stream_snapshot(service.provider, raw);
    let raw = stream.as_ref().unwrap_or(raw);
    if pending(response) || (input.stream && stream.is_none()) {
        return Ok(BTreeMap::new());
    }
    let gemini_entries = (service.provider == "gemini" && family == Family::Batches)
        .then(|| gemini_batch_entries(raw));
    let entries = response
        .get("usage_entries")
        .and_then(Value::as_array)
        .or(gemini_entries.as_ref().filter(|values| !values.is_empty()));
    let values = entries
        .map(|entries| entries.iter().collect::<Vec<_>>())
        .unwrap_or_else(|| vec![raw]);
    let mut attributed = Vec::new();
    for value in values {
        let has_usage = ["usage", "usageMetadata", "usage_metadata"]
            .iter()
            .any(|key| value.get(key).is_some_and(Value::is_object));
        if !has_usage
            && (input.action != Action::Create
                || matches!(
                    family,
                    Family::Inference | Family::Tasks | Family::Batches | Family::Agents
                ))
        {
            continue;
        }
        let model = value
            .get("model")
            .and_then(Value::as_str)
            .or(input.model.as_deref())
            .or_else(|| {
                matches!(input.action, Action::Create | Action::Compact)
                    .then_some(service.connection.model.as_deref())
                    .flatten()
            })
            .unwrap_or("");
        let model = if service.provider == "gemini" {
            model.strip_prefix("models/").unwrap_or(model)
        } else {
            model
        };
        let operation = if matches!(family, Family::Tasks | Family::Inference) {
            "inference".into()
        } else {
            format!("{family:?}").to_lowercase()
        };
        let mut data = official_service::resource_usage(
            &service.connection,
            service.provider,
            &operation,
            model,
            value,
            !matches!(
                family,
                Family::Inference | Family::Tasks | Family::Embeddings
            ),
        )?;
        let encoded = data
            .remove(PROVIDER_TOOL_USAGE_METADATA_KEY)
            .ok_or_else(|| invalid("missing provider usage metadata"))?;
        let mut usage: AttributedCompletionUsage =
            serde_json::from_str(&encoded).map_err(|e| PluginError::internal_error(&e))?;
        if let Some(count) = value
            .get("request_count")
            .or_else(|| value.pointer("/request_counts/total"))
            .and_then(Value::as_u64)
        {
            usage.usage.requests = count;
        }
        let resource = if family == Family::Batches {
            raw.get("id")
                .or_else(|| raw.get("name"))
                .and_then(Value::as_str)
                .or(input.id.as_deref())
                .map(|id| id.strip_prefix("batches/").unwrap_or(id))
        } else {
            value
                .get("id")
                .or_else(|| value.get("name"))
                .and_then(Value::as_str)
                .or(input.id.as_deref())
        };
        usage.request_id = resource.map(str::to_owned).or_else(|| {
            response
                .get("request_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
        if family != Family::Batches {
            if let Some(resource) = resource {
                if !claim(service, "inference", resource, &usage, warnings).await? {
                    continue;
                }
            }
        }
        attributed.push(usage);
    }
    let Some(mut usage) = attributed.pop() else {
        return Ok(BTreeMap::new());
    };
    if !attributed.is_empty() {
        // Preserve each batch model's attribution and pricing independently.
        attributed.push(usage);
        usage = AttributedCompletionUsage {
            provider_id: service.connection.provider_id.clone(),
            model_id: String::new(),
            operation: "batches".into(),
            request_id: input.id.clone(),
            usage: Box::new(agena_provider::CompletionUsage {
                attributed_usage: attributed,
                ..Default::default()
            }),
        };
    }
    if family == Family::Batches {
        let resource = raw
            .get("id")
            .or_else(|| raw.get("name"))
            .and_then(Value::as_str)
            .or(input.id.as_deref());
        if let Some(resource) = resource {
            if !claim(
                service,
                "batch",
                resource.strip_prefix("batches/").unwrap_or(resource),
                &usage,
                warnings,
            )
            .await?
            {
                return Ok(BTreeMap::new());
            }
        }
    }
    Ok(BTreeMap::from([(
        PROVIDER_TOOL_USAGE_METADATA_KEY.into(),
        serde_json::to_string(&usage).map_err(|e| PluginError::internal_error(&e))?,
    )]))
}

async fn claim(
    service: &Service<'_>,
    namespace: &str,
    id: &str,
    usage: &AttributedCompletionUsage,
    warnings: &mut Vec<String>,
) -> Result<bool> {
    let path = marker_path(service, namespace, id);
    let bytes = serde_json::to_vec(usage).map_err(|e| PluginError::internal_error(&e))?;
    if let Err(error) =
        crate::artifact_file::persist_new(path.clone(), bytes, "provider usage observation").await
    {
        if tokio::fs::try_exists(path).await.unwrap_or(false) {
            return Ok(false);
        }
        warnings.push(format!(
            "Usage could not be deduplicated and was not attributed: {}",
            error.failure.user.fallback
        ));
        return Ok(false);
    }
    Ok(true)
}

fn marker_path(service: &Service<'_>, namespace: &str, id: &str) -> PathBuf {
    let mut hash = Sha256::new();
    for component in [
        &service.connection.receipt_key,
        &service.connection.provider_id,
        &service.connection.base_url,
        service.provider,
        namespace,
        id,
    ] {
        hash.update(component.as_bytes());
        hash.update([0]);
    }
    agena_storage_layout::StorageLayout::current()
        .artifacts_dir(service.root)
        .join("provider-tools/usage")
        .join(format!("{}.json", hex::encode(hash.finalize())))
}

fn stream_snapshot(provider: &str, response: &Value) -> Option<Value> {
    let events = response.get("events")?.as_array()?;
    if provider == "claude" {
        if !events
            .iter()
            .any(|event| event.get("type").and_then(Value::as_str) == Some("message_stop"))
        {
            return None;
        }
        let mut message = events.iter().find_map(|e| e.get("message")).cloned()?;
        for event in events {
            if let Some(usage) = event.get("usage").and_then(Value::as_object) {
                let target = message
                    .as_object_mut()?
                    .entry("usage")
                    .or_insert_with(|| json!({}))
                    .as_object_mut()?;
                target.extend(usage.clone());
            }
        }
        return Some(message);
    }
    events.iter().rev().find_map(|event| {
        let value = event.get("response").or_else(|| event.get("interaction"))?;
        (!pending(value) && (value.get("usage").is_some() || value.get("usage_metadata").is_some()))
            .then(|| value.clone())
    })
}

pub(super) fn batch_results(bytes: &[u8]) -> Result<Vec<Value>> {
    let mut groups = BTreeMap::<String, Value>::new();
    let text =
        std::str::from_utf8(bytes).map_err(|_| invalid("batch results are not UTF-8 JSONL"))?;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let row: Value = serde_json::from_str(line)
            .map_err(|_| invalid("batch results contain invalid JSONL"))?;
        let Some(message) = row.pointer("/result/message") else {
            continue;
        };
        let Some(usage) = message.get("usage").and_then(Value::as_object) else {
            continue;
        };
        let model = message
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let group = groups
            .entry(model.clone())
            .or_insert_with(|| json!({"model":model,"usage":{},"request_count":0}));
        group["request_count"] = json!(
            group["request_count"]
                .as_u64()
                .unwrap_or(0)
                .saturating_add(1)
        );
        add_usage(
            group.get_mut("usage").unwrap(),
            &Value::Object(usage.clone()),
        );
    }
    Ok(groups.into_values().collect())
}

fn add_usage(total: &mut Value, value: &Value) {
    if let Some(values) = value.as_object() {
        if !total.is_object() {
            *total = json!({});
        }
        for (key, value) in values {
            add_usage(
                total
                    .as_object_mut()
                    .unwrap()
                    .entry(key)
                    .or_insert(Value::Null),
                value,
            );
        }
    } else if let Some(count) = value.as_u64() {
        *total = json!(total.as_u64().unwrap_or(0).saturating_add(count));
    }
}

fn gemini_batch_entries(batch: &Value) -> Vec<Value> {
    let mut groups = BTreeMap::<String, Value>::new();
    // File-backed results remain explicit downloads. Only the documented
    // inline response set is inspected, never arbitrary tool-generated JSON.
    let responses = batch
        .pointer("/output/inlinedResponses/inlinedResponses")
        .or_else(|| batch.pointer("/output/inlinedResponses"))
        .or_else(|| batch.pointer("/output/inlinedEmbedContentResponses/inlinedResponses"))
        .and_then(Value::as_array);
    for row in responses.into_iter().flatten() {
        let Some(response) = row.get("response") else {
            continue;
        };
        let Some(usage) = response
            .get("usageMetadata")
            .or_else(|| response.get("usage_metadata"))
        else {
            continue;
        };
        let model = response
            .get("modelVersion")
            .or_else(|| batch.get("model"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let group = groups
            .entry(model.to_owned())
            .or_insert_with(|| json!({"model":model,"usageMetadata":{},"request_count":0}));
        group["request_count"] = json!(
            group["request_count"]
                .as_u64()
                .unwrap_or(0)
                .saturating_add(1)
        );
        add_usage(group.get_mut("usageMetadata").unwrap(), usage);
    }
    groups.into_values().collect()
}
