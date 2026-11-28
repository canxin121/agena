use crate::{Application, ApplicationError, dto::OperatorToolResource};

type DocumentKey = (i64, i64, (u64, u64));
type DocumentCell = std::sync::Arc<tokio::sync::OnceCell<Option<agena_domain::PartDocument>>>;

/// Hosted image results may already have a text resource containing the same
/// bytes as their image attachment. Render the attachment, keeping unrelated
/// text resources and the durable raw result available for explicit inspection.
fn tool_content_for_presentation(
    value: &serde_json::Value,
) -> Option<agena_runtime_contracts::part_content::ToolCallContent> {
    use agena_domain::{AttachmentKind, ContentFormat};
    use agena_runtime_contracts::part_content::ToolCallContent;

    let mut content = ToolCallContent::try_from(value).ok()?;
    // Provider diagnostics have a separate metadata reference. They are never
    // part of the human output, including the client's resource fallback.
    if let Some(trace) = content
        .metadata
        .get("agena.provider_trace")
        .and_then(|value| serde_json::from_value::<agena_domain::ContentRef>(value.clone()).ok())
    {
        content
            .resources
            .retain(|resource| resource.resource_id != trace.resource_id);
    }
    // A PTY's raw log and screen are two projections of the same output.
    // Keep the raw capture in storage, but display and subscribe to the screen
    // once. This also applies before a running tool has returned a result.
    if agena_domain::ToolPermissionConfig::canonical_shell_tool_name(&content.name).is_some()
        && content
            .resources
            .iter()
            .any(|resource| resource.kind == agena_domain::ContentKind::Terminal)
    {
        content
            .resources
            .retain(|resource| resource.kind != agena_domain::ContentKind::Log);
    }
    let Some(output) = content.output.as_mut() else {
        return Some(content);
    };
    let image_result = output
        .payload
        .as_ref()
        .and_then(|payload| payload.get("type"))
        .and_then(serde_json::Value::as_str)
        == Some("image_generation_call")
        && output
            .attachments
            .iter()
            .any(|attachment| attachment.kind == AttachmentKind::Image);
    if image_result {
        if let Some(payload) = output
            .payload
            .as_mut()
            .and_then(serde_json::Value::as_object_mut)
        {
            payload.remove("result");
        }
        let binary_resources = output
            .fields
            .iter()
            .filter(|field| field.pointer == "/result" && field.format == ContentFormat::Plain)
            .map(|field| field.resource.resource_id)
            .collect::<std::collections::BTreeSet<_>>();
        output
            .fields
            .retain(|field| !binary_resources.contains(&field.resource.resource_id));
        content
            .resources
            .retain(|resource| !binary_resources.contains(&resource.resource_id));
    }
    Some(content)
}

fn tool_part_control(
    content: &agena_runtime_contracts::part_content::ToolCallContent,
) -> Option<agena_domain::ViewBlock> {
    use agena_domain::ViewBlock;

    let payload = content.output.as_ref()?.payload.as_ref()?;
    let tool = agena_tool::normalized_tool_identity(&content.name);
    let field = |key| {
        payload
            .get(key)?
            .as_str()
            .filter(|value| !value.trim().is_empty())
            .map(str::to_owned)
    };
    match tool.as_str() {
        "shell.spawn" | "shell.open" | "shell.watch" if !content.resources.is_empty() => {
            Some(ViewBlock::ShellProcess {
                id: "shell-process".to_owned(),
                process_id: field("process_id")?,
            })
        }
        "agent.spawn" | "agent.followup" => {
            let session_id = payload.get("agent_id")?.as_i64().filter(|id| *id > 0)?;
            Some(ViewBlock::AgentSession {
                id: "agent-session".to_owned(),
                session_id,
            })
        }
        "browser.open" => Some(ViewBlock::BrowserPage {
            id: "browser-page".to_owned(),
            page_id: field("session_id")?,
        }),
        _ => None,
    }
}

fn project_tool_content(
    value: &serde_json::Value,
    sections: &[agena_api::live::ToolDetailSection],
) -> serde_json::Value {
    let mut projected = agena_api::live::project_tool_call_content(value, sections);
    if let Some(metadata) = projected
        .get_mut("metadata")
        .and_then(serde_json::Value::as_object_mut)
    {
        metadata.remove(agena_mcp_client::apps::APP_METADATA_KEY);
        metadata.remove(agena_mcp_client::apps::APP_RESULT_METADATA_KEY);
    }
    if value.get("resources").is_some()
        && let Some(content) = tool_content_for_presentation(value)
    {
        projected["resources"] = serde_json::to_value(content.resources)
            .expect("content references are JSON serializable");
    }
    projected
}

/// Rejected inputs may not form a renderable invocation. Keep their public
/// failure visible even when there is no tool document, without repeating the
/// ordinary error band in otherwise valid tool presentations.
pub(super) fn tool_document_with_failure(
    content: &agena_runtime_contracts::part_content::ToolCallContent,
    document: Option<agena_domain::PartDocument>,
) -> Option<agena_domain::PartDocument> {
    use agena_domain::{PartDocument, ViewBlock};

    if document.is_some() {
        return document;
    }
    let Some(error) = &content.error else {
        return document;
    };
    let message = error.user_message();
    if message.trim().is_empty() {
        return document;
    }
    Some(PartDocument {
        title: content.name.clone(),
        summary: String::new(),
        blocks: std::iter::once(ViewBlock::Text {
            id: Some("tool-error".into()),
            // Model feedback and private diagnostics are deliberately not
            // part of the human document.
            text: message.to_owned(),
        })
        .chain(content.resources.iter().map(|resource| ViewBlock::Content {
            id: format!("output:{}", resource.resource_id),
            resource: resource.clone(),
            format: agena_domain::ContentFormat::Plain,
        }))
        .collect(),
    })
}

/// The final state-aware title can already include the human summary as one
/// complete result suffix. Keep that fact once, without treating substrings,
/// truncated titles or independent plugin summaries as duplicates.
fn deduplicate_tool_document_summary(
    mut document: agena_domain::PartDocument,
) -> agena_domain::PartDocument {
    let title = document.title.trim();
    let summary = document.summary.trim();
    if !summary.is_empty()
        && (title == summary
            || title
                .strip_suffix(summary)
                .is_some_and(|prefix| prefix.ends_with(" · ")))
    {
        document.summary.clear();
    }
    document
}

fn is_external_agent_native_tool(
    content: &agena_runtime_contracts::part_content::ToolCallContent,
) -> bool {
    content.plugin.is_none()
        && content.tool_api_call.is_none()
        && content
            .input
            .get("agent")
            .and_then(serde_json::Value::as_str)
            .is_some()
        && !content.name.starts_with("external_agent_")
}

struct ParsedAgyCommandOutput {
    exit_code: Option<i32>,
    stdout: String,
    matched: bool,
}

fn parse_agy_command_output(text: &str) -> ParsedAgyCommandOutput {
    let normalized = text.replace("\r\n", "\n");
    let trimmed = normalized.trim_start();
    if !(trimmed.starts_with("Created At:")
        || trimmed.starts_with("Completed At:")
        || trimmed.starts_with("Command exited with code "))
    {
        return ParsedAgyCommandOutput {
            exit_code: None,
            stdout: text.to_owned(),
            matched: false,
        };
    }
    let mut exit_code = None;
    let mut lines = trimmed.lines().peekable();
    while let Some(line) = lines.peek().copied() {
        let line_trimmed = line.trim();
        if line_trimmed.starts_with("Created At:") || line_trimmed.starts_with("Completed At:") {
            lines.next();
            continue;
        }
        if let Some(rest) = line_trimmed.strip_prefix("Command exited with code ") {
            if let Ok(code) = rest.trim_end_matches('.').trim().parse::<i32>() {
                exit_code = Some(code);
            }
            lines.next();
            continue;
        }
        if line_trimmed.is_empty() {
            lines.next();
            continue;
        }
        if matches!(line_trimmed, "Command output:" | "Output:") {
            lines.next();
        }
        break;
    }
    let stdout = lines
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end_matches(['\n', '\r'])
        .to_owned();
    ParsedAgyCommandOutput {
        exit_code,
        stdout,
        matched: true,
    }
}

fn parse_injected_agena_mcp_function(name: &str) -> Option<agena_domain::ToolApiFunction> {
    let is_hex_tag = |tag: &str| {
        !tag.is_empty() && tag.len() <= 16 && tag.bytes().all(|byte| byte.is_ascii_hexdigit())
    };
    if let Some(rest) = name.strip_prefix("mcp__") {
        let (server, tool) = rest.split_once("__")?;
        let valid_server = server == "agena"
            || server
                .strip_prefix("agena_")
                .or_else(|| server.strip_prefix("agena-"))
                .is_some_and(is_hex_tag);
        if valid_server {
            return agena_domain::ToolApiFunction::from_function_name(tool);
        }
    }
    if let Some(rest) = name.strip_prefix("mcp_agena_") {
        if let Some(function) = agena_domain::ToolApiFunction::from_function_name(rest) {
            return Some(function);
        }
        if let Some((tag, tool)) = rest.split_once('_')
            && is_hex_tag(tag)
        {
            return agena_domain::ToolApiFunction::from_function_name(tool);
        }
    }
    if let Some(rest) = name.strip_prefix("mcp_agena-")
        && let Some((tag, tool)) = rest.split_once('_')
        && is_hex_tag(tag)
    {
        return agena_domain::ToolApiFunction::from_function_name(tool);
    }
    None
}

fn strip_external_agent_prefix<'a>(
    content: &'a agena_runtime_contracts::part_content::ToolCallContent,
) -> (Option<&'a str>, &'a str) {
    let raw_name = content.name.trim();
    let agent_label = serde_json::from_value::<agena_domain::ExternalAgent>(
        content.input.get("agent").cloned().unwrap_or_default(),
    )
    .ok()
    .map(|agent| agent.label().to_owned());
    let without_agent = agent_label
        .as_deref()
        .and_then(|label| {
            raw_name
                .strip_prefix(label)
                .and_then(|rest| rest.strip_prefix(" · "))
        })
        .map(str::trim)
        .filter(|rest| !rest.is_empty())
        .unwrap_or(raw_name);
    if content
        .input
        .get("session_id")
        .and_then(serde_json::Value::as_str)
        .is_some()
        && let Some((child, rest)) = without_agent.split_once(" · ")
        && !child.trim().is_empty()
        && !rest.trim().is_empty()
    {
        return (Some(child.trim()), rest.trim());
    }
    (None, without_agent)
}

fn raw_input_str<'a>(
    raw_input: Option<&'a serde_json::Map<String, serde_json::Value>>,
    keys: &[&str],
) -> Option<&'a str> {
    let map = raw_input?;
    keys.iter().find_map(|key| {
        map.get(*key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    })
}

fn raw_input_u64(
    raw_input: Option<&serde_json::Map<String, serde_json::Value>>,
    keys: &[&str],
) -> Option<u64> {
    let map = raw_input?;
    keys.iter().find_map(|key| {
        map.get(*key)
            .and_then(|value| {
                value
                    .as_u64()
                    .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
            })
            .filter(|value| *value > 0)
    })
}

fn compact_path_display(path: &str) -> String {
    let path = path.trim();
    if path.starts_with('/') {
        let mut parts = path.rsplit('/').filter(|segment| !segment.is_empty());
        let file = parts.next().unwrap_or(path);
        return file.to_owned();
    }
    path.to_owned()
}

fn first_meaningful_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(agena_tool::normalize_tool_summary)
        .filter(|line| !line.is_empty())
}

fn humanize_snake_identifier(name: &str) -> String {
    let words = name
        .split(['_', '-'])
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    if words.is_empty() {
        return name.to_owned();
    }
    let mut title = words.join(" ");
    if let Some(first) = title.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    title
}

fn external_agent_state_suffix(state: agena_domain::ToolResultState) -> Option<&'static str> {
    match state {
        agena_domain::ToolResultState::PolicyDenied => Some("permission denied"),
        agena_domain::ToolResultState::UserDeclined => Some("declined"),
        agena_domain::ToolResultState::CapabilityUnavailable
        | agena_domain::ToolResultState::ToolUnavailable => Some("unavailable"),
        agena_domain::ToolResultState::Failed => Some("failed"),
        agena_domain::ToolResultState::Cancelled => Some("cancelled"),
        agena_domain::ToolResultState::Pending
        | agena_domain::ToolResultState::Running
        | agena_domain::ToolResultState::Completed => None,
    }
}

fn append_status_suffix(title: String, state: agena_domain::ToolResultState) -> String {
    let Some(suffix) = external_agent_state_suffix(state) else {
        return title;
    };
    if title.to_ascii_lowercase().contains(suffix) {
        title
    } else {
        agena_tool::compose_tool_title(title, suffix)
    }
}

fn render_external_agent_tool_presentation(
    content: &agena_runtime_contracts::part_content::ToolCallContent,
    stored_summary: Option<&str>,
    mut blocks: Vec<agena_domain::ViewBlock>,
    mut output_blocks: Vec<agena_domain::ViewBlock>,
) -> agena_domain::PartDocument {
    use agena_domain::{PartDocument, ToolResultState, ViewBlock};
    use agena_runtime_tools::tool::human_view::BuiltinHumanRenderer;

    let raw_input = content
        .input
        .get("raw_input")
        .and_then(serde_json::Value::as_object);
    let tool_name_hint = content
        .input
        .get("name")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty());
    let tool_kind = content
        .input
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .map(str::trim);
    let tool_summary_hint = raw_input_str(raw_input, &["toolSummary", "description"]);
    let (child_prefix, stripped_title) = strip_external_agent_prefix(content);

    let output_text = content.output.as_ref().and_then(|output| {
        let text = output.text_content();
        if !text.trim().is_empty() {
            Some(text.to_owned())
        } else {
            output
                .payload
                .as_ref()
                .and_then(|payload| payload.pointer("/external_agent/output"))
                .and_then(BuiltinHumanRenderer::external_agent_output_text)
        }
    });

    let mcp_function = tool_name_hint
        .and_then(parse_injected_agena_mcp_function)
        .or_else(|| {
            stripped_title
                .strip_prefix("Run ")
                .and_then(parse_injected_agena_mcp_function)
        });
    if let Some(function) = mcp_function {
        let mut title = match function {
            agena_domain::ToolApiFunction::Call => {
                if let Some(target) = raw_input_str(raw_input, &["tool"]) {
                    agena_tool::compose_tool_title("Call tool", target)
                } else {
                    function.function_name().to_owned()
                }
            }
            agena_domain::ToolApiFunction::Help => {
                if let Some(target) = raw_input_str(raw_input, &["tool"]) {
                    agena_tool::compose_tool_title(function.function_name(), target)
                } else {
                    function.function_name().to_owned()
                }
            }
            _ => function.function_name().to_owned(),
        };
        if let Some(child) = child_prefix {
            title = agena_tool::compose_tool_title(child, title);
        }
        title = append_status_suffix(title, content.state);
        blocks.extend(output_blocks);
        let summary = output_text
            .as_deref()
            .and_then(first_meaningful_line)
            .unwrap_or_default();
        return PartDocument {
            title,
            summary,
            blocks,
        };
    }

    let command = raw_input_str(raw_input, &["CommandLine", "command", "cmd", "script"]);
    let is_command = command.is_some()
        || matches!(
            tool_name_hint,
            Some("run_command" | "bash" | "shell" | "Bash" | "execute_command")
        );
    if let Some(command) = command
        && is_command
    {
        let parsed = output_text.as_deref().map(parse_agy_command_output);
        let exit_code = parsed.as_ref().and_then(|p| p.exit_code).or_else(|| {
            content.output.as_ref().and_then(|output| {
                output
                    .payload
                    .as_ref()?
                    .pointer("/external_agent/output/exit_code")
                    .and_then(serde_json::Value::as_i64)
                    .and_then(|code| i32::try_from(code).ok())
            })
        });
        let clean_stdout = match parsed {
            Some(parsed) => parsed.stdout,
            None => String::new(),
        };
        let cwd = raw_input_str(raw_input, &["Cwd", "cwd", "workdir"]).map(str::to_owned);
        let has_command_block = blocks
            .iter()
            .chain(output_blocks.iter())
            .any(|block| matches!(block, ViewBlock::Command { .. }));
        if !has_command_block {
            if !clean_stdout.is_empty() || output_text.is_some() {
                output_blocks.retain(|block| {
                    !matches!(block.block_id(), Some("acp-result" | "acp-raw-result"))
                });
            }
            blocks.push(ViewBlock::Command {
                id: Some("acp-command".into()),
                command: command.to_owned(),
                cwd,
                exit_code,
                stdout: clean_stdout.clone(),
                stderr: String::new(),
            });
        }
        blocks.extend(output_blocks);

        let mut title = agena_tool::compose_tool_title("Run command", command);
        if let Some(child) = child_prefix {
            title = agena_tool::compose_tool_title(child, title);
        }
        if !matches!(
            content.state,
            ToolResultState::Pending | ToolResultState::Running
        ) {
            if let Some(code) = exit_code {
                let fragment = if code == 0 {
                    "passed".to_owned()
                } else {
                    format!("failed · exit {code}")
                };
                title = agena_tool::compose_tool_title(title, fragment);
            } else {
                title = append_status_suffix(title, content.state);
            }
        }
        let summary = first_meaningful_line(&clean_stdout)
            .or_else(|| {
                tool_summary_hint
                    .filter(|hint| *hint != command)
                    .map(agena_tool::normalize_tool_summary)
            })
            .unwrap_or_default();
        return PartDocument {
            title,
            summary,
            blocks,
        };
    }

    let line_range = match (
        raw_input_u64(raw_input, &["StartLine", "start_line"]),
        raw_input_u64(raw_input, &["EndLine", "end_line"]),
    ) {
        (Some(start), Some(end)) => Some(format!("L{start}-L{end}")),
        (Some(start), None) => Some(format!("L{start}+")),
        _ => None,
    };
    let raw_path = raw_input_str(
        raw_input,
        &["Path", "TargetFile", "file_path", "path", "AbsolutePath"],
    );

    let mut base_title = match tool_name_hint {
        Some("read_file" | "view_file" | "Read") => {
            let target = stripped_title
                .strip_prefix("File ")
                .map(str::trim)
                .filter(|target| !target.is_empty())
                .map(str::to_owned)
                .or_else(|| raw_path.map(compact_path_display))
                .unwrap_or_else(|| stripped_title.to_owned());
            let action = agena_tool::normalize_tool_title(format!("Read {target}"));
            match &line_range {
                Some(range) => agena_tool::compose_tool_title(action, range),
                None => action,
            }
        }
        Some("write_file" | "write_to_file" | "Write") => {
            let target = raw_path
                .map(compact_path_display)
                .or_else(|| {
                    stripped_title
                        .strip_prefix("File ")
                        .map(str::trim)
                        .filter(|target| !target.is_empty())
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| stripped_title.to_owned());
            agena_tool::normalize_tool_title(format!("Write {target}"))
        }
        Some(
            "edit_file" | "replace_file_content" | "multi_replace_file_content" | "Edit"
            | "MultiEdit",
        ) => {
            let target = raw_path
                .map(compact_path_display)
                .or_else(|| {
                    stripped_title
                        .strip_prefix("File ")
                        .map(str::trim)
                        .filter(|target| !target.is_empty())
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| stripped_title.to_owned());
            agena_tool::normalize_tool_title(format!("Edit {target}"))
        }
        Some("search_web" | "web_search" | "WebSearch") => {
            if let Some(query) = raw_input_str(raw_input, &["query", "Query", "q"]) {
                agena_tool::compose_tool_title("Search web", query)
            } else {
                "Search web".to_owned()
            }
        }
        _ if tool_kind == Some("read") && stripped_title.starts_with("File ") => {
            let target = stripped_title
                .strip_prefix("File ")
                .map(str::trim)
                .filter(|target| !target.is_empty())
                .unwrap_or(stripped_title);
            let action = agena_tool::normalize_tool_title(format!("Read {target}"));
            match &line_range {
                Some(range) => agena_tool::compose_tool_title(action, range),
                None => action,
            }
        }
        _ => {
            if let Some(hint) = tool_name_hint
                && stripped_title == hint
                && (hint.contains('_') || hint.contains('-'))
            {
                let label = humanize_snake_identifier(hint);
                match tool_summary_hint {
                    Some(summary) => agena_tool::compose_tool_title(label, summary),
                    None => agena_tool::normalize_tool_title(label),
                }
            } else {
                agena_tool::normalize_tool_title(stripped_title)
            }
        }
    };

    if let Some(child) = child_prefix {
        base_title = agena_tool::compose_tool_title(child, base_title);
    }
    let title = append_status_suffix(base_title, content.state);

    if let Some(range) = &line_range {
        let mut updated_location = false;
        for block in &mut output_blocks {
            if let ViewBlock::Table { id, rows, .. } = block
                && id.as_deref() == Some("acp-locations")
                && rows.len() == 1
                && rows[0].get(1).is_some_and(serde_json::Value::is_null)
            {
                rows[0][1] = serde_json::Value::String(range.clone());
                updated_location = true;
            }
        }
        if !updated_location
            && let Some(path) = raw_path
            && !output_blocks
                .iter()
                .any(|block| block.block_id() == Some("acp-locations"))
        {
            output_blocks.push(ViewBlock::Table {
                id: Some("acp-locations".into()),
                columns: vec!["Path".into(), "Line".into()],
                rows: vec![vec![
                    serde_json::Value::String(path.to_owned()),
                    serde_json::Value::String(range.clone()),
                ]],
            });
        }
    }

    blocks.extend(output_blocks);

    let cleaned_output_line = output_text.as_deref().and_then(|text| {
        let parsed = parse_agy_command_output(text);
        if parsed.matched {
            first_meaningful_line(&parsed.stdout)
        } else {
            first_meaningful_line(text)
        }
    });
    let summary = cleaned_output_line
        .or_else(|| {
            stored_summary
                .map(str::trim)
                .filter(|summary| !summary.is_empty() && *summary != "No output")
                .map(agena_tool::normalize_tool_summary)
        })
        .unwrap_or_default();

    PartDocument {
        title,
        summary,
        blocks,
    }
}

#[cfg(test)]
mod tool_document_summary_tests {
    use super::*;
    use agena_domain::{
        ArtifactRef, PartDocument, RawOutput, StructuredObject, ToolInvocation, ToolResultState,
        ViewBlock,
    };
    use serde_json::json;

    fn document(title: &str, summary: &str) -> PartDocument {
        PartDocument {
            title: title.to_owned(),
            summary: summary.to_owned(),
            blocks: vec![ViewBlock::Text {
                id: Some("output".into()),
                text: "Independent output remains available.".into(),
            }],
        }
    }

    #[test]
    fn completed_read_media_count_is_displayed_once_and_keeps_its_media() {
        let invocation = ToolInvocation::new(
            "fs.read_media",
            StructuredObject::try_from(json!({"path":"ai-drawings/watercolor-cat.png"})).unwrap(),
        );
        let output = RawOutput {
            payload: Some(json!({"loaded_paths":["ai-drawings/watercolor-cat.png"]})),
            ..Default::default()
        };
        let title = agena_tool::completed_tool_title_for_state(
            &invocation,
            ToolResultState::Completed,
            &output,
        );
        let summary = agena_tool::result_title_fragment_for_tool(&invocation.name, &output);
        assert_eq!(
            title,
            "Read media · ai-drawings/watercolor-cat.png · 1 file loaded"
        );
        assert_eq!(summary, "1 file loaded");
        let mut before = document(&title, &summary);
        before.blocks.push(ViewBlock::Media {
            id: Some("media".into()),
            artifact: ArtifactRef {
                uri: "agena:media:v1:test-reference".into(),
                mime: "image/png".into(),
                name: Some("watercolor-cat.png".into()),
                size_bytes: Some(1690348),
                sha256: None,
            },
        });
        let after = deduplicate_tool_document_summary(before.clone());
        assert_eq!(after.title, before.title);
        assert!(after.summary.is_empty());
        assert_eq!(after.blocks, before.blocks);
    }

    #[test]
    fn summary_deduplication_requires_an_exact_complete_title_suffix() {
        for (title, summary) in [
            ("1 file loaded", "1 file loaded"),
            ("Read media · file.png · 1 file loaded", "1 file loaded"),
            (
                "Read · 1 file loaded · truncated",
                "1 file loaded · truncated",
            ),
            ("  Read · 1 file loaded  ", " 1 file loaded "),
            ("读取媒体 · 已载入 1 个文件", "已载入 1 个文件"),
        ] {
            let before = document(title, summary);
            let after = deduplicate_tool_document_summary(before.clone());
            assert!(after.summary.is_empty(), "{title:?} / {summary:?}");
            assert_eq!(after.title, before.title);
            assert_eq!(after.blocks, before.blocks);
        }
        for (title, summary) in [
            ("Read · 11 files loaded", "1 files loaded"),
            ("Read · cached1 file loaded", "1 file loaded"),
            ("Read ·1 file loaded", "1 file loaded"),
            ("Read · 1 file lo…", "1 file loaded"),
            ("Read · 1 file loaded", "1 File Loaded"),
            ("Read · 1 file loaded", "1 file loaded from cache"),
            ("Read · 1 file loaded", "1 file loaded · all bytes verified"),
            ("Read · 1 file loaded", ""),
        ] {
            let before = document(title, summary);
            assert_eq!(deduplicate_tool_document_summary(before.clone()), before);
        }
    }

    #[test]
    fn running_and_failed_titles_keep_independent_result_or_failure_summaries() {
        let invocation = ToolInvocation::new(
            "fs.read_media",
            StructuredObject::try_from(json!({"path":"ai-drawings/watercolor-cat.png"})).unwrap(),
        );
        let output = RawOutput {
            payload: Some(json!({"loaded_paths":["ai-drawings/watercolor-cat.png"]})),
            ..Default::default()
        };
        for state in [
            ToolResultState::Pending,
            ToolResultState::Running,
            ToolResultState::Failed,
        ] {
            let title = agena_tool::completed_tool_title_for_state(&invocation, state, &output);
            for summary in ["1 file loaded", "The remaining file could not be read."] {
                let before = document(&title, summary);
                assert_eq!(deduplicate_tool_document_summary(before.clone()), before);
            }
        }
    }
}

#[cfg(test)]
mod terminal_presentation_tests {
    use super::*;
    use agena_domain::{ContentId, ContentKind, ContentRef};
    use agena_runtime_contracts::part_content::ToolCallContent;

    #[test]
    fn pty_displays_one_projection_before_and_after_tool_completion() {
        let log = ContentRef {
            resource_id: ContentId::new(),
            kind: ContentKind::Log,
        };
        let terminal = ContentRef {
            resource_id: ContentId::new(),
            kind: ContentKind::Terminal,
        };
        let text = ContentRef {
            resource_id: ContentId::new(),
            kind: ContentKind::Text,
        };
        let mut content = ToolCallContent {
            name: "shell.open".into(),
            input: serde_json::json!({"command": "seq 1 10"}),
            resources: vec![log.clone(), terminal.clone(), text.clone()],
            ..Default::default()
        };
        for output in [None, Some(agena_domain::RawOutput::default())] {
            content.output = output;
            let projected = tool_content_for_presentation(&content.as_value()).unwrap();
            assert_eq!(projected.resources, vec![terminal.clone(), text.clone()]);
            assert_eq!(content.resources[0], log);
        }
        content.resources = vec![log.clone()];
        assert_eq!(
            tool_content_for_presentation(&content.as_value())
                .unwrap()
                .resources,
            vec![log]
        );
    }
}

#[cfg(test)]
mod failure_presentation_tests {
    use super::*;
    use agena_domain::{ContentId, ContentKind, ContentRef, PartDocument, ViewBlock};
    use agena_runtime_contracts::part_content::ToolCallContent;

    fn failed_content(name: &str) -> ToolCallContent {
        use agena_failure::*;
        ToolCallContent {
            name: name.into(),
            state: agena_domain::ToolResultState::Failed,
            error: Some(agena_domain::OperationError {
                failure: Failure::new(
                    FailureCode::new("tool.invalid_input"),
                    FailureCategory::InvalidInput,
                    FailureResponsibility::Caller,
                    RetryDirective::CorrectInput,
                    RecoveryDirective::None,
                    FailureImpact::RequestRejected,
                    UserPresentation::new("tool.invalid_input", "file_path is required"),
                )
                .with_model_feedback(
                    ModelFeedback::invalid_input().with_text("private-model-sentinel"),
                ),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn failed_tools_without_output_have_public_reason_and_keep_resources() {
        for name in [
            "shell.exec",
            "fs.read",
            "fs.read_media",
            "chatgpt.image_understanding",
            "tools_list",
            "custom.plugin.tool",
        ] {
            let mut content = failed_content(name);
            // A rejected scalar is intentionally kept as input. It cannot be
            // made into a ToolInvocation merely to render the failure.
            content.input = serde_json::json!(17);
            let resource = ContentRef {
                resource_id: ContentId::new(),
                kind: ContentKind::Text,
            };
            content.resources.push(resource.clone());
            let document = tool_document_with_failure(&content, None).unwrap();
            assert_eq!(document.title, name);
            assert_eq!(
                document.blocks[0].text_value(),
                Some("file_path is required")
            );
            assert!(matches!(
                &document.blocks[1],
                ViewBlock::Content { resource: retained, .. } if retained == &resource
            ));
            assert!(
                !serde_json::to_string(&document)
                    .unwrap()
                    .contains("private-model-sentinel")
            );
            assert!(content.output.is_none());
        }
        assert!(tool_document_with_failure(&ToolCallContent::default(), None).is_none());
    }

    #[test]
    fn existing_tool_document_is_preserved_when_failure_is_available() {
        let content = failed_content("custom.plugin.tool");
        let distinct = ViewBlock::Text {
            id: Some("partial-output".into()),
            text: "Two files were already processed.".into(),
        };
        let base = PartDocument {
            title: "Custom tool".into(),
            summary: "Partial result".into(),
            blocks: vec![distinct.clone()],
        };
        assert_eq!(
            tool_document_with_failure(&content, Some(base.clone())),
            Some(base)
        );
        for same in [
            ViewBlock::Text {
                id: None,
                text: " file_path is required ".into(),
            },
            ViewBlock::Markdown {
                id: None,
                text: "file_path is required".into(),
            },
        ] {
            let base = PartDocument {
                title: "Custom tool".into(),
                summary: String::new(),
                blocks: vec![same, distinct.clone()],
            };
            assert_eq!(
                tool_document_with_failure(&content, Some(base.clone())),
                Some(base)
            );
        }
        let base = PartDocument {
            title: "Custom tool".into(),
            summary: "file_path is required".into(),
            blocks: vec![distinct],
        };
        assert_eq!(
            tool_document_with_failure(&content, Some(base.clone())),
            Some(base)
        );
    }
}

#[cfg(test)]
mod external_agent_presentation_tests {
    use super::*;
    use agena_domain::{RawOutput, ToolResultState, ViewBlock};
    use agena_runtime_contracts::part_content::ToolCallContent;
    use agena_runtime_tools::tool::human_view::BuiltinHumanRenderer;
    use serde_json::json;

    #[test]
    fn antigravity_run_command_renders_clean_command_block_without_mangled_title_or_timestamps() {
        let output = RawOutput {
            payload: Some(json!({
                "text": "",
                "external_agent": {
                    "content": [],
                    "locations": null,
                    "output": {
                        "output": "Created At: 2026-11-29T00:04:32+08:00\nCompleted At: 2026-11-29T00:04:34+08:00\nCommand exited with code 0.\n\nCommand output:\n162:pub(super) struct AcpClient {\n"
                    }
                }
            })),
            ..Default::default()
        };
        let content = ToolCallContent {
            name: "Antigravity · rg -n 'struct AcpClient' crates/acp.rs".into(),
            state: ToolResultState::Completed,
            input: json!({
                "agent": "antigravity",
                "name": "run_command",
                "kind": "execute",
                "raw_input": {
                    "CommandLine": "rg -n 'struct AcpClient' crates/acp.rs",
                    "toolSummary": "Find AcpClient struct"
                }
            }),
            output: Some(output.clone()),
            ..Default::default()
        };
        let output_blocks = BuiltinHumanRenderer::external_agent_blocks(&output).unwrap();
        let doc = render_external_agent_tool_presentation(&content, None, Vec::new(), output_blocks);

        assert!(doc.title.starts_with("Run command · rg -n 'struct AcpClient' crates/acp.rs"));
        assert!(doc.title.ends_with(" · passed"));
        assert!(!doc.title.contains("Created At"));
        assert!(!doc.title.starts_with("Rs"));
        assert_eq!(doc.summary, "162:pub(super) struct AcpClient {");
        assert_eq!(doc.blocks.len(), 1);
        match &doc.blocks[0] {
            ViewBlock::Command {
                command,
                exit_code,
                stdout,
                ..
            } => {
                assert_eq!(command, "rg -n 'struct AcpClient' crates/acp.rs");
                assert_eq!(*exit_code, Some(0));
                assert_eq!(stdout, "162:pub(super) struct AcpClient {");
            }
            other => panic!("expected Command block, got {other:?}"),
        }
    }

    #[test]
    fn antigravity_read_file_preserves_filename_line_range_and_omits_no_output() {
        let output = RawOutput {
            payload: Some(json!({
                "text": "",
                "external_agent": {
                    "content": [],
                    "locations": [{"path": "/repo/Cargo.toml", "line": null}],
                    "output": null
                }
            })),
            ..Default::default()
        };
        let content = ToolCallContent {
            name: "Antigravity · File Cargo.toml".into(),
            state: ToolResultState::Completed,
            input: json!({
                "agent": "antigravity",
                "name": "read_file",
                "kind": "read",
                "locations": [{"path": "/repo/Cargo.toml", "line": null}],
                "raw_input": {
                    "Path": "/repo/Cargo.toml",
                    "StartLine": 1,
                    "EndLine": 3
                }
            }),
            output: Some(output.clone()),
            ..Default::default()
        };
        let output_blocks = BuiltinHumanRenderer::external_agent_blocks(&output).unwrap();
        let doc = render_external_agent_tool_presentation(&content, None, Vec::new(), output_blocks);

        assert_eq!(doc.title, "Read Cargo.toml · L1-L3");
        assert_eq!(doc.summary, "");
        assert!(matches!(
            doc.blocks.as_slice(),
            [ViewBlock::Table { rows, .. }] if rows == &vec![vec![json!("/repo/Cargo.toml"), json!("L1-L3")]]
        ));
    }

    #[test]
    fn replayed_injected_agena_mcp_tools_strip_mcp_and_hex_prefixes() {
        let output = RawOutput {
            payload: Some(json!({
                "text": "",
                "external_agent": {
                    "content": [],
                    "locations": null,
                    "output": {"output": "agena.fs\nagena.shell\n"}
                }
            })),
            ..Default::default()
        };
        let content = ToolCallContent {
            name: "Antigravity · Run mcp_agena_62af60cf_plugins_list".into(),
            state: ToolResultState::Completed,
            input: json!({
                "agent": "antigravity",
                "name": "mcp_agena_62af60cf_plugins_list"
            }),
            output: Some(output.clone()),
            ..Default::default()
        };
        let output_blocks = BuiltinHumanRenderer::external_agent_blocks(&output).unwrap();
        let doc = render_external_agent_tool_presentation(&content, None, Vec::new(), output_blocks);

        assert_eq!(doc.title, "plugins_list");
        assert_eq!(doc.summary, "agena.fs");
        assert!(matches!(
            doc.blocks.as_slice(),
            [ViewBlock::Text { text, .. }] if text == "agena.fs\nagena.shell\n"
        ));
    }
}

/// Shared by list, detail and live reads. No client or transport dimension.
/// Entries share initialization and cache only bounded semantic documents.
#[derive(Default)]
pub(crate) struct PartDocumentCache {
    entries: std::sync::Mutex<std::collections::BTreeMap<DocumentKey, DocumentCell>>,
}

impl PartDocumentCache {
    fn cell(&self, key: DocumentKey) -> DocumentCell {
        let mut entries = self.entries.lock().expect("part document cache lock");
        if let Some(cell) = entries.get(&key) {
            return cell.clone();
        }
        while entries.len() >= 256 {
            entries.pop_first();
        }
        let cell = std::sync::Arc::new(tokio::sync::OnceCell::new());
        entries.insert(key, cell.clone());
        cell
    }

    fn remove_oversized(&self, key: DocumentKey, cell: &DocumentCell) {
        let mut entries = self.entries.lock().expect("part document cache lock");
        if entries
            .get(&key)
            .is_some_and(|entry| std::sync::Arc::ptr_eq(entry, cell))
        {
            entries.remove(&key);
        }
    }
}

impl Application {
    pub async fn part_document(
        &self,
        part: &agena_api::part::PartResource,
    ) -> Option<agena_domain::PartDocument> {
        if part.content["external_agent_document"] == true {
            return part.presentation.clone();
        }
        let key = (
            part.part_id,
            part.revision,
            self.plugin_runtime().projection_generation(),
        );
        let cell = self.part_documents.cell(key);
        cell.get_or_init(|| async {
            let document = self
                .render_part_presentation(&part.kind, &part.content, part.summary.as_deref())
                .await;
            if document.as_ref().is_some_and(|document| {
                serde_json::to_vec(document).map_or(true, |bytes| bytes.len() > 64 * 1024)
            }) {
                self.part_documents.remove_oversized(key, &cell);
            }
            document
        })
        .await
        .clone()
    }

    /// Read-time detail projection never grants write access to a transcript entry.
    pub(crate) async fn project_part_resource(
        &self,
        part: &mut agena_api::part::PartResource,
        sections: &[agena_api::live::ToolDetailSection],
    ) {
        part.sections = sections
            .iter()
            .map(|section| agena_api::part::LoadedPartSection {
                section: *section,
                revision: part.revision,
            })
            .collect();
        if sections.contains(&agena_api::live::ToolDetailSection::Presentation) {
            part.presentation = self.part_document(part).await;
        }
        if part.kind == agena_runtime_contracts::part_content::ToolCallContent::kind() {
            part.content = project_tool_content(&part.content, sections);
        }
    }
    /// Produce ephemeral model/human views from the invocation and the sole
    /// durable raw result. Callers may serialize the human side in a response,
    /// but must never persist either projection.
    pub async fn render_tool_result(
        &self,
        invocation: &agena_domain::ToolInvocation,
        output: &agena_domain::RawOutput,
    ) -> agena_runtime::RuntimeToolResultProjection {
        self.runtime_tools()
            .render_tool_result(invocation, output)
            .await
    }

    /// The single semantic document projection used by snapshots, detail
    /// reads and live updates. Resource identity survives tool completion;
    /// layout and disclosure policy remain the consumer's responsibility.
    pub async fn render_part_presentation(
        &self,
        kind: &str,
        value: &serde_json::Value,
        summary: Option<&str>,
    ) -> Option<agena_domain::PartDocument> {
        use agena_domain::PartDocument;
        use agena_runtime_contracts::part_content::ToolCallContent;

        if kind != ToolCallContent::kind() {
            return summary
                .filter(|summary| !summary.trim().is_empty())
                .map(|summary| PartDocument {
                    title: String::new(),
                    summary: summary.to_owned(),
                    blocks: Vec::new(),
                });
        }
        let content = tool_content_for_presentation(value)?;
        let document = self.render_tool_part_presentation(&content, summary).await;
        tool_document_with_failure(&content, document).map(deduplicate_tool_document_summary)
    }

    async fn render_tool_part_presentation(
        &self,
        content: &agena_runtime_contracts::part_content::ToolCallContent,
        summary: Option<&str>,
    ) -> Option<agena_domain::PartDocument> {
        use agena_domain::{ContentKind, PartDocument, ViewBlock};

        if let Some(call) = &content.tool_api_call
            && call.function != agena_domain::ToolApiFunction::Call
        {
            // Protocol responses have no owning plugin and use a plain-text
            // presentation; they never enter plugin result rendering.
            let text = content
                .output
                .as_ref()
                .and_then(|output| output.payload.as_ref())
                .and_then(|payload| payload.get("text"))
                .and_then(serde_json::Value::as_str);
            return Some(PartDocument {
                title: call.function.function_name().to_owned(),
                summary: summary.unwrap_or_default().to_owned(),
                blocks: text
                    .into_iter()
                    .map(|text| ViewBlock::Text {
                        id: Some("tool-api-response".into()),
                        text: text.to_owned(),
                    })
                    .chain(content.resources.iter().map(|resource| ViewBlock::Content {
                        id: format!("tool-api-response:{}", resource.resource_id),
                        resource: resource.clone(),
                        format: agena_domain::ContentFormat::Plain,
                    }))
                    .collect(),
            });
        }
        let input = agena_domain::StructuredObject::try_from(content.input.clone()).ok()?;
        let invocation = agena_domain::ToolInvocation {
            name: content.name.clone(),
            plugin_name: content.plugin.clone(),
            input,
        };
        let mut blocks = tool_part_control(content).into_iter().collect::<Vec<_>>();
        let tool_name = agena_tool::normalized_tool_identity(&content.name);
        if tool_name == "mcp.tools.call"
            && content.plugin.as_deref() == Some("agena.mcp")
            && content.state == agena_domain::ToolResultState::Completed
            && content
                .metadata
                .contains_key(agena_mcp_client::apps::APP_METADATA_KEY)
        {
            blocks.push(ViewBlock::Custom {
                id: Some("mcp-app".to_owned()),
                kind: "mcp_app".to_owned(),
                schema: serde_json::Value::Null,
                presentation: Default::default(),
            });
        }
        let shell = matches!(
            tool_name.as_str(),
            "shell.exec" | "shell.spawn" | "shell.open" | "shell.watch"
        );
        if shell && !content.resources.is_empty() {
            let command = match content.input.get("command") {
                Some(serde_json::Value::String(command)) => command.clone(),
                Some(serde_json::Value::Array(args)) => args
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" "),
                _ => String::new(),
            };
            if !command.is_empty() {
                blocks.push(ViewBlock::Command {
                    id: Some("command".to_owned()),
                    command,
                    cwd: content
                        .input
                        .get("workdir")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                    exit_code: content
                        .output
                        .as_ref()
                        .and_then(|output| output.payload.as_ref())
                        .and_then(|payload| payload.get("exit_code"))
                        .and_then(serde_json::Value::as_i64)
                        .and_then(|code| i32::try_from(code).ok()),
                    stdout: String::new(),
                    stderr: String::new(),
                });
            }
        }
        blocks.extend(content.resources.iter().map(|resource| {
            ViewBlock::Content {
                id: format!("output:{}", resource.resource_id),
                resource: resource.clone(),
                format: content
                    .output
                    .as_ref()
                    .and_then(|output| {
                        output
                            .fields
                            .iter()
                            .find(|field| field.resource.resource_id == resource.resource_id)
                    })
                    .map_or(agena_domain::ContentFormat::Plain, |field| field.format),
            }
        }));
        if is_external_agent_native_tool(content) {
            let output_blocks = match content.output.as_ref() {
                Some(output) => {
                    if let Some(acp_blocks) =
                        agena_runtime_tools::tool::human_view::BuiltinHumanRenderer::external_agent_blocks(output)
                    {
                        acp_blocks
                    } else {
                        self.render_tool_result(&invocation, output)
                            .await
                            .human
                            .blocks
                            .into_iter()
                            .filter(|block| block.text_value() != Some("No output."))
                            .collect()
                    }
                }
                None => Vec::new(),
            };
            return Some(render_external_agent_tool_presentation(
                content,
                summary,
                blocks,
                output_blocks,
            ));
        }
        let Some(output) = content.output.as_ref() else {
            return Some(PartDocument {
                title: agena_tool::tool_title_for_state(&invocation, content.state),
                summary: summary.unwrap_or_default().to_owned(),
                blocks,
            });
        };
        let projection = self.render_tool_result(&invocation, output).await;

        if !content
            .resources
            .iter()
            .any(|resource| matches!(resource.kind, ContentKind::Log | ContentKind::Terminal))
        {
            blocks.extend(projection.human.blocks.into_iter().filter(|block| {
                content
                    .output
                    .as_ref()
                    .is_none_or(|output| output.fields.is_empty())
                    || block.text_value() != Some("No output.")
            }));
        }
        Some(PartDocument {
            title: agena_tool::completed_tool_title_for_state(&invocation, content.state, output),
            summary: projection.human.summary,
            blocks,
        })
    }

    pub(crate) async fn render_transcript_presentations(
        &self,
        parts: &mut [agena_api::part::PartResource],
    ) {
        for part in parts {
            part.presentation = self.part_document(part).await;
            part.sections = vec![agena_api::part::LoadedPartSection {
                section: agena_api::live::ToolDetailSection::Presentation,
                revision: part.revision,
            }];
            if part.kind == agena_runtime_contracts::part_content::ToolCallContent::kind() {
                part.content = project_tool_content(
                    &part.content,
                    &[agena_api::live::ToolDetailSection::Presentation],
                );
            }
        }
    }

    pub async fn list_operator_tools(&self) -> Vec<OperatorToolResource> {
        self.runtime_tools()
            .available_runtime_tools()
            .await
            .into_iter()
            .map(|tool| OperatorToolResource {
                name: tool.name,
                canonical_name: Some(tool.canonical_name),
                summary: tool.summary,
                before_help: tool.before_help,
                help: tool.help,
                after_help: tool.after_help,
                tags: tool.tags,
                input_schema: tool.input_schema,
                output_schema: tool.output_schema,
                interactive: tool.interactive,
                read_only: tool.read_only,
                destructive: tool.destructive,
                open_world: tool.open_world,
                agent: tool.agent,
                plugin_id: tool.plugin_id,
            })
            .collect()
    }

    pub async fn invoke_operator_tool(
        &self,
        workspace_id: i64,
        tool: &str,
        input: Option<serde_json::Value>,
        call_id: i64,
    ) -> Result<agena_tool::ToolExecutionSummary, ApplicationError> {
        self.invoke_operator_tool_with_sandbox(workspace_id, tool, input, call_id, true)
            .await
    }

    pub async fn invoke_operator_tool_with_sandbox(
        &self,
        workspace_id: i64,
        tool: &str,
        input: Option<serde_json::Value>,
        call_id: i64,
        sandbox_enabled: bool,
    ) -> Result<agena_tool::ToolExecutionSummary, ApplicationError> {
        self.ensure_operator_workspace(workspace_id).await?;
        let tool = tool.trim();
        if tool.is_empty() {
            return Err(ApplicationError::bad_request("tool name is required"));
        }
        let input = input.unwrap_or_else(|| serde_json::json!({}));
        let input = agena_domain::StructuredObject::try_from(input).map_err(|error| {
            ApplicationError::bad_request_with_diagnostic("tool input must be an object", error)
        })?;
        let invocation = agena_domain::ToolInvocation::new(tool.to_owned(), input);
        self.runtime_tools()
            .execute_runtime_tool_with_sandbox(&invocation, call_id, sandbox_enabled)
            .await
            .map(agena_runtime::SessionToolExecutionOutcome::into_summary)
            .map_err(|error| ApplicationError::internal_error(&error))
    }

    async fn ensure_operator_workspace(&self, workspace_id: i64) -> Result<(), ApplicationError> {
        let workspace = self
            .service()
            .get_workspace(workspace_id)
            .await?
            .ok_or_else(|| {
                ApplicationError::not_found_with_diagnostic(
                    "The operator workspace was not found.",
                    format!("operator workspace not found: {workspace_id}"),
                )
            })?;
        let server_root = self.workspace_root().to_path_buf();
        let requested_root = std::path::PathBuf::from(workspace.path);
        let (server_root, requested_root) = tokio::task::spawn_blocking(move || {
            let server_root = std::fs::canonicalize(&server_root).map_err(|error| {
                format!(
                    "failed to canonicalize server workspace {}: {error}",
                    server_root.display()
                )
            })?;
            let requested_root = std::fs::canonicalize(&requested_root).map_err(|error| {
                format!(
                    "failed to canonicalize requested workspace {}: {error}",
                    requested_root.display()
                )
            })?;
            Ok::<_, String>((server_root, requested_root))
        })
        .await
        .map_err(|error| {
            ApplicationError::internal(format!(
                "operator workspace canonicalization task failed: {error}"
            ))
        })?
        .map_err(|diagnostic| {
            ApplicationError::bad_request_with_diagnostic(
                "The operator workspace cannot be resolved.",
                diagnostic,
            )
        })?;
        if server_root != requested_root {
            return Err(ApplicationError::conflict_with_diagnostic(
                "The operator workspace does not match this server.",
                format!(
                    "operator workspace mismatch: requested={}, server={}",
                    requested_root.display(),
                    server_root.display()
                ),
            ));
        }
        Ok(())
    }
}
