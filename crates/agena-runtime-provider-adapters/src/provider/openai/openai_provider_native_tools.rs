use super::{
    CompletionStreamEvent, ModelId, ProviderError, ProviderId, StructuredObject, ToolInvocation,
    ToolOutput, protocol_ids, utils,
};
use agena_provider::{
    ProviderNativeToolArtifact, ProviderNativeToolOutputBlock, ProviderNativeToolSearchResult,
};

#[derive(Debug)]
pub(super) enum OpenAiProviderNativeToolEvent {
    Started {
        stream_key: String,
        id: Option<String>,
        invocation: ToolInvocation,
        title: String,
        raw: Option<serde_json::Value>,
    },
    Completed {
        stream_key: String,
        id: Option<String>,
        invocation: ToolInvocation,
        title: String,
        summary: String,
        output_text: String,
        blocks: Vec<ProviderNativeToolOutputBlock>,
        details: ToolOutput,
        raw: Option<serde_json::Value>,
    },
}

pub(super) fn responses_provider_native_tool_event(
    provider_id: &ProviderId,
    model: &ModelId,
    event: &serde_json::Value,
) -> Result<Option<CompletionStreamEvent>, ProviderError> {
    let Some(event_type) = event.get("type").and_then(serde_json::Value::as_str) else {
        return Ok(None);
    };
    if !matches!(
        event_type,
        "response.output_item.added" | "response.output_item.done"
    ) {
        return Ok(None);
    }

    let Some(item) = event.get("item") else {
        return Ok(None);
    };
    let Some(item_kind) = item.get("type").and_then(serde_json::Value::as_str) else {
        return Ok(None);
    };

    let provider_native_tool_event = match item_kind {
        "web_search_call" => openai_web_search_tool_event(event_type, event, item)?,
        "file_search_call" => openai_file_search_tool_event(event_type, event, item)?,
        "code_interpreter_call" => openai_code_interpreter_tool_event(event_type, event, item)?,
        "image_generation_call" => openai_image_generation_tool_event(event_type, event, item)?,
        _ => None,
    };

    Ok(provider_native_tool_event.map(
        |provider_native_tool_event| match provider_native_tool_event {
            OpenAiProviderNativeToolEvent::Started {
                stream_key,
                id,
                invocation,
                title,
                raw,
            } => CompletionStreamEvent::ProviderNativeToolCallStarted {
                provider_id: provider_id.clone(),
                model: model.clone(),
                stream_key,
                id,
                invocation,
                title,
                raw,
            },
            OpenAiProviderNativeToolEvent::Completed {
                stream_key,
                id,
                invocation,
                title,
                summary,
                output_text,
                blocks,
                details,
                raw,
            } => CompletionStreamEvent::ProviderNativeToolCallCompleted {
                provider_id: provider_id.clone(),
                model: model.clone(),
                stream_key,
                id,
                invocation,
                title,
                summary,
                output_text,
                blocks,
                details,
                raw,
            },
        },
    ))
}

pub(super) fn openai_web_search_tool_event(
    event_type: &str,
    event: &serde_json::Value,
    item: &serde_json::Value,
) -> Result<Option<OpenAiProviderNativeToolEvent>, ProviderError> {
    let id = utils::normalize_optional_text(
        item.get("id")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned),
    );
    let output_index = event
        .get("output_index")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as usize);
    let stream_key = responses_provider_native_tool_stream_key(id.as_deref(), output_index)
        .ok_or_else(|| {
            ProviderError::Provider(
                "openai responses web_search_call event was missing both item id and output index"
                    .to_owned(),
            )
        })?;

    let action = item.get("action");
    let invocation = openai_web_search_invocation(action)?;
    let details = ToolOutput::from_json_payload(Some(item)).map_err(ProviderError::Provider)?;
    let raw = Some(item.clone());

    Ok(Some(if event_type == "response.output_item.added" {
        OpenAiProviderNativeToolEvent::Started {
            stream_key,
            id,
            invocation,
            title: String::new(),
            raw,
        }
    } else {
        let blocks = openai_web_search_blocks(action);
        let result_count = blocks
            .iter()
            .filter_map(|block| match block {
                ProviderNativeToolOutputBlock::SearchResults { results, .. } => Some(results.len()),
                _ => None,
            })
            .sum::<usize>();
        OpenAiProviderNativeToolEvent::Completed {
            stream_key,
            id,
            invocation,
            title: "Web search".to_string(),
            summary: format!("{result_count} results"),
            output_text: String::new(),
            blocks,
            details,
            raw,
        }
    }))
}

pub(super) fn openai_file_search_tool_event(
    event_type: &str,
    event: &serde_json::Value,
    item: &serde_json::Value,
) -> Result<Option<OpenAiProviderNativeToolEvent>, ProviderError> {
    let id = utils::normalize_optional_text(
        item.get("id")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned),
    );
    let output_index = event
        .get("output_index")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as usize);
    let stream_key = responses_provider_native_tool_stream_key(id.as_deref(), output_index)
        .ok_or_else(|| {
            ProviderError::Provider(
                "openai responses file_search_call event was missing both item id and output index"
                    .to_owned(),
            )
        })?;

    let queries = openai_file_search_queries(item);
    let invocation = openai_file_search_invocation(queries.as_slice())?;
    let title = openai_file_search_title(queries.as_slice());
    let details = ToolOutput::from_json_payload(Some(item)).map_err(ProviderError::Provider)?;
    let raw = Some(item.clone());

    Ok(Some(if event_type == "response.output_item.added" {
        OpenAiProviderNativeToolEvent::Started {
            stream_key,
            id,
            invocation,
            title,
            raw,
        }
    } else {
        let blocks = openai_file_search_blocks(queries.as_slice(), item.get("results"));
        let result_count = blocks
            .iter()
            .filter_map(|block| match block {
                ProviderNativeToolOutputBlock::SearchResults { results, .. } => Some(results.len()),
                _ => None,
            })
            .sum::<usize>();
        OpenAiProviderNativeToolEvent::Completed {
            stream_key,
            id,
            invocation,
            title,
            summary: format!("{result_count} results"),
            output_text: String::new(),
            blocks,
            details,
            raw,
        }
    }))
}

pub(super) fn openai_code_interpreter_tool_event(
    event_type: &str,
    event: &serde_json::Value,
    item: &serde_json::Value,
) -> Result<Option<OpenAiProviderNativeToolEvent>, ProviderError> {
    let id = utils::normalize_optional_text(
        item.get("id")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned),
    );
    let output_index = event
        .get("output_index")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as usize);
    let stream_key =
        responses_provider_native_tool_stream_key(id.as_deref(), output_index).ok_or_else(|| {
            ProviderError::Provider(
                "openai responses code_interpreter_call event was missing both item id and output index"
                    .to_owned(),
            )
        })?;

    let code = item
        .get("code")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let invocation = openai_code_interpreter_invocation(code)?;
    let title = if code.is_some() {
        "code execution".to_owned()
    } else {
        "code interpreter".to_owned()
    };
    let details = ToolOutput::from_json_payload(Some(item)).map_err(ProviderError::Provider)?;
    let raw = Some(item.clone());

    Ok(Some(if event_type == "response.output_item.added" {
        OpenAiProviderNativeToolEvent::Started {
            stream_key,
            id,
            invocation,
            title,
            raw,
        }
    } else {
        let blocks = openai_code_interpreter_blocks(item.get("outputs"));
        OpenAiProviderNativeToolEvent::Completed {
            stream_key,
            id,
            invocation,
            title,
            summary: format!("{} outputs", blocks.len()),
            output_text: openai_code_interpreter_output_text(blocks.as_slice()),
            blocks,
            details,
            raw,
        }
    }))
}

pub(super) fn openai_image_generation_tool_event(
    event_type: &str,
    event: &serde_json::Value,
    item: &serde_json::Value,
) -> Result<Option<OpenAiProviderNativeToolEvent>, ProviderError> {
    let id = utils::normalize_optional_text(
        item.get("id")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned),
    );
    let output_index = event
        .get("output_index")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as usize);
    let stream_key =
        responses_provider_native_tool_stream_key(id.as_deref(), output_index).ok_or_else(|| {
            ProviderError::Provider(
                "openai responses image_generation_call event was missing both item id and output index"
                    .to_owned(),
            )
        })?;

    let revised_prompt = item
        .get("revised_prompt")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let invocation = openai_image_generation_invocation(revised_prompt)?;
    let title = revised_prompt
        .map(|prompt| format!("image generation {prompt}"))
        .unwrap_or_else(|| "image generation".to_owned());
    let details = ToolOutput::from_json_payload(Some(item)).map_err(ProviderError::Provider)?;
    let raw = Some(item.clone());

    Ok(Some(if event_type == "response.output_item.added" {
        OpenAiProviderNativeToolEvent::Started {
            stream_key,
            id,
            invocation,
            title,
            raw,
        }
    } else {
        let blocks = openai_image_generation_blocks(item);
        let image_count = blocks
            .iter()
            .filter(|block| matches!(block, ProviderNativeToolOutputBlock::Media { .. }))
            .count();
        OpenAiProviderNativeToolEvent::Completed {
            stream_key,
            id,
            invocation,
            title,
            summary: format!("{image_count} images"),
            output_text: revised_prompt.unwrap_or_default().to_owned(),
            blocks,
            details,
            raw,
        }
    }))
}

pub(super) fn responses_provider_native_tool_stream_key(
    item_id: Option<&str>,
    output_index: Option<usize>,
) -> Option<String> {
    item_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("item:{value}"))
        .or_else(|| output_index.map(|value| format!("idx:{value}")))
}

pub(super) fn openai_web_search_invocation(
    action: Option<&serde_json::Value>,
) -> Result<ToolInvocation, ProviderError> {
    let action_type = action
        .and_then(|value| value.get("type"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    let input = match action_type {
        "search" => {
            let detail = web_search_action_detail(action);
            StructuredObject::try_from(if detail.is_empty() {
                serde_json::json!({})
            } else {
                serde_json::json!({ "query": detail })
            })
            .map_err(ProviderError::Provider)?
        }
        "open_page" => {
            let payload = if let Some(url) = action
                .and_then(|value| value.get("url"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                serde_json::json!({ "url": url })
            } else {
                serde_json::json!({})
            };
            StructuredObject::try_from(payload).map_err(ProviderError::Provider)?
        }
        "find_in_page" => {
            let url = action
                .and_then(|value| value.get("url"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let pattern = action
                .and_then(|value| value.get("pattern"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let payload = match (url, pattern) {
                (Some(url), Some(pattern)) => serde_json::json!({
                    "url": url,
                    "pattern": pattern
                }),
                (Some(url), None) => serde_json::json!({ "url": url }),
                (None, Some(pattern)) => serde_json::json!({ "pattern": pattern }),
                (None, None) => serde_json::json!({}),
            };
            StructuredObject::try_from(payload).map_err(ProviderError::Provider)?
        }
        _ => StructuredObject::default(),
    };

    Ok(ToolInvocation::new("web.run", input))
}

pub(super) fn openai_web_search_blocks(
    action: Option<&serde_json::Value>,
) -> Vec<ProviderNativeToolOutputBlock> {
    let Some(sources) = action
        .and_then(|value| value.get("sources"))
        .and_then(serde_json::Value::as_array)
    else {
        return Vec::new();
    };

    let results = sources
        .iter()
        .filter_map(openai_web_search_result)
        .collect::<Vec<_>>();
    if results.is_empty() {
        return Vec::new();
    }

    let query = web_search_action_detail(action);
    vec![ProviderNativeToolOutputBlock::SearchResults {
        query: (!query.is_empty()).then_some(query),
        results,
    }]
}

pub(super) fn openai_file_search_queries(item: &serde_json::Value) -> Vec<String> {
    item.get("queries")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub(super) fn openai_file_search_invocation(
    queries: &[String],
) -> Result<ToolInvocation, ProviderError> {
    let input = match queries {
        [] => StructuredObject::default(),
        [query] => StructuredObject::try_from(serde_json::json!({ "query": query }))
            .map_err(ProviderError::Provider)?,
        [first, ..] => StructuredObject::try_from(serde_json::json!({
            "query": first,
            "queries": queries,
        }))
        .map_err(ProviderError::Provider)?,
    };
    Ok(ToolInvocation::new("file_search", input))
}

pub(super) fn openai_file_search_title(queries: &[String]) -> String {
    queries
        .first()
        .map(|query| format!("file search {query}"))
        .unwrap_or_else(|| "file search".to_owned())
}

pub(super) fn openai_file_search_blocks(
    queries: &[String],
    results: Option<&serde_json::Value>,
) -> Vec<ProviderNativeToolOutputBlock> {
    let Some(results) = results.and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };

    let results = results
        .iter()
        .filter_map(openai_file_search_result)
        .collect::<Vec<_>>();
    if results.is_empty() {
        return Vec::new();
    }

    let query = queries.first().cloned().map(|first| {
        if queries.len() > 1 {
            format!("{first} ...")
        } else {
            first
        }
    });
    vec![ProviderNativeToolOutputBlock::SearchResults { query, results }]
}

pub(super) fn openai_file_search_result(
    source: &serde_json::Value,
) -> Option<ProviderNativeToolSearchResult> {
    let file_id = source
        .get("file_id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let filename = source
        .get("filename")
        .or_else(|| source.get("title"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let uri = file_id
        .as_deref()
        .map(|value| format!("file:{value}"))
        .or_else(|| filename.clone())?;
    let title = filename
        .or(file_id)
        .unwrap_or_else(|| "file result".to_owned());
    let snippet = source
        .get("text")
        .or_else(|| source.get("snippet"))
        .or_else(|| source.get("summary"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let score = source
        .get("score")
        .or_else(|| source.get("rank"))
        .and_then(serde_json::Value::as_f64)
        .map(|value| value as f32);

    Some(ProviderNativeToolSearchResult {
        title,
        uri,
        snippet,
        score,
    })
}

pub(super) fn openai_code_interpreter_invocation(
    code: Option<&str>,
) -> Result<ToolInvocation, ProviderError> {
    let input = match code {
        Some(code) => StructuredObject::try_from(serde_json::json!({ "code": code }))
            .map_err(ProviderError::Provider)?,
        None => StructuredObject::default(),
    };
    Ok(ToolInvocation::new("code_execution", input))
}

pub(super) fn openai_code_interpreter_blocks(
    outputs: Option<&serde_json::Value>,
) -> Vec<ProviderNativeToolOutputBlock> {
    let Some(outputs) = outputs.and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };

    let mut blocks = Vec::new();
    for output in outputs {
        let output_type = output
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        match output_type {
            "logs" => {
                let Some(logs) = output
                    .get("logs")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                else {
                    continue;
                };
                blocks.push(ProviderNativeToolOutputBlock::Text {
                    text: logs.to_owned(),
                });
            }
            "files" => {
                let Some(files) = output.get("files").and_then(serde_json::Value::as_array) else {
                    continue;
                };
                for file in files {
                    let Some(file_id) = file
                        .get("file_id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    else {
                        continue;
                    };
                    let mime_type = file
                        .get("mime_type")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .unwrap_or("application/octet-stream")
                        .to_owned();
                    let name = file
                        .get("filename")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToOwned::to_owned)
                        .unwrap_or_else(|| file_id.to_owned());
                    blocks.push(ProviderNativeToolOutputBlock::Media {
                        mime_type: mime_type.clone(),
                        artifact: ProviderNativeToolArtifact {
                            uri: format!("file:{file_id}"),
                            mime: mime_type,
                            name: Some(name),
                            size_bytes: None,
                            sha256: None,
                        },
                    });
                }
            }
            _ => {
                let pretty = serde_json::to_string_pretty(output)
                    .unwrap_or_else(|_| output.to_string())
                    .trim()
                    .to_owned();
                if !pretty.is_empty() {
                    blocks.push(ProviderNativeToolOutputBlock::Text { text: pretty });
                }
            }
        }
    }

    blocks
}

pub(super) fn openai_code_interpreter_output_text(
    blocks: &[ProviderNativeToolOutputBlock],
) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            ProviderNativeToolOutputBlock::Text { text } if !text.trim().is_empty() => {
                Some(text.trim())
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(super) fn openai_image_generation_invocation(
    revised_prompt: Option<&str>,
) -> Result<ToolInvocation, ProviderError> {
    let input = match revised_prompt {
        Some(prompt) => StructuredObject::try_from(serde_json::json!({ "description": prompt }))
            .map_err(ProviderError::Provider)?,
        None => StructuredObject::default(),
    };
    Ok(ToolInvocation::new("image_generation", input))
}

pub(super) fn openai_image_generation_blocks(
    item: &serde_json::Value,
) -> Vec<ProviderNativeToolOutputBlock> {
    let Some(result) = item
        .get("result")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Vec::new();
    };

    let mime_type = item
        .get("mime_type")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("image/png")
        .to_owned();
    let extension = mime_type
        .strip_prefix("image/")
        .filter(|value| !value.is_empty())
        .unwrap_or("png");
    let extension = extension.to_owned();
    let data_url = format!("data:{mime_type};base64,{result}");

    vec![ProviderNativeToolOutputBlock::Media {
        mime_type: mime_type.clone(),
        artifact: ProviderNativeToolArtifact {
            uri: data_url,
            mime: mime_type,
            name: Some(format!("generated-image.{extension}")),
            size_bytes: None,
            sha256: None,
        },
    }]
}

pub(super) fn openai_web_search_result(
    source: &serde_json::Value,
) -> Option<ProviderNativeToolSearchResult> {
    let uri = source
        .get("url")
        .or_else(|| source.get("uri"))
        .or_else(|| source.get("link"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_owned();
    let title = source
        .get("title")
        .or_else(|| source.get("name"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| uri.clone());
    let snippet = source
        .get("snippet")
        .or_else(|| source.get("summary"))
        .or_else(|| source.get("text"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let score = source
        .get("score")
        .or_else(|| source.get("rank"))
        .and_then(serde_json::Value::as_f64)
        .map(|value| value as f32);

    Some(ProviderNativeToolSearchResult {
        title,
        uri,
        snippet,
        score,
    })
}

pub(super) fn web_search_action_detail(action: Option<&serde_json::Value>) -> String {
    let Some(action) = action else {
        return String::new();
    };
    let action_type = action
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    match action_type {
        "search" => action
            .get("query")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| {
                let items = action
                    .get("queries")
                    .and_then(serde_json::Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let first = items
                    .first()
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or_default()
                    .to_owned();
                if items.len() > 1 && !first.is_empty() {
                    format!("{first} ...")
                } else {
                    first
                }
            }),
        "open_page" => action
            .get("url")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_default(),
        "find_in_page" => {
            let url = action
                .get("url")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let pattern = action
                .get("pattern")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            match (pattern, url) {
                (Some(pattern), Some(url)) => format!("'{pattern}' in {url}"),
                (Some(pattern), None) => format!("'{pattern}'"),
                (None, Some(url)) => url.to_owned(),
                (None, None) => String::new(),
            }
        }
        _ => String::new(),
    }
}

pub(super) fn responses_output_call_id(call_id: Option<&str>) -> Option<String> {
    call_id
        .and_then(protocol_ids::openai_responses_call_id)
        .map(String::from)
}

pub(super) fn responses_input_call_id(raw: &str) -> Option<String> {
    protocol_ids::openai_responses_call_id(raw).map(String::from)
}
