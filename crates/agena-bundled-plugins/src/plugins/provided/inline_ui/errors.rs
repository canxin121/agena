use agena_domain::{InlineDocument, InlineUpdate};
use agena_plugin_host::sdk::{PluginError, PluginErrorKind};
use agena_runtime_plugins::plugins::inline_ui::InlineUiUpdateError;
use serde::Serialize;
use serde_json::{Value, json};

const MAX_ISSUES: usize = 8;

#[derive(Debug, Serialize)]
struct InlineIssue {
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    block_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    node_id: Option<String>,
    reason: String,
    fix: &'static str,
}

#[derive(Debug, Serialize)]
struct InlineErrorReport {
    code: &'static str,
    // Absent for uncertain storage failures. All reports in this module are
    // pre-commit validation or CAS rejection, with guaranteed no application.
    applied: bool,
    issues: Vec<InlineIssue>,
    truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_revision: Option<u64>,
}

fn safe_text(text: &str) -> String {
    let scrubbed = agena_failure::diagnostic::user_message_with_context(text, 150);
    if scrubbed.is_empty() {
        "Invalid value for this field".into()
    } else {
        scrubbed
    }
}

fn fix_for(error: &str) -> &'static str {
    if error.contains("unsupported property") || error.contains("unknown field") {
        "Use only the declared fields/properties for this block or node kind."
    } else if error.contains("expression")
        || error.contains("operator")
        || error.contains("$op")
        || error.contains("$get")
        || error.contains("lookup")
    {
        "Use an allowed $get path or $op with the required args; see guide expressions."
    } else if error.contains("state") || error.contains("bind") {
        "Declare the referenced state key and match its control kind and initial value."
    } else if error.contains("children")
        || error.contains("child")
        || error.contains("subtree")
        || error.contains("unreachable")
    {
        "Reference existing node IDs once from root; keep the node tree acyclic."
    } else if error.contains("duplicate") || error.contains("id") {
        "Use unique, stable ASCII IDs with letters, numbers, _ or -."
    } else if error.contains("revision") {
        "Read the current revision and reconcile changes before resubmitting."
    } else if error.contains("numeric") || error.contains("number") || error.contains("finite") {
        "Supply a valid finite number with the documented bounds and unit."
    } else if error.contains("chart") || error.contains("series") {
        "Check chart type, series lengths, labels and nonnegative pie values."
    } else if error.contains("visible") || error.contains("boolean") {
        "Make visible evaluate to a boolean and match the state/input type."
    } else if error.contains("action") {
        "Declare a named local action using set, toggle, reset or copy."
    } else {
        "Correct this path against guide() or request the full schema; do not guess values."
    }
}

fn path_and_reason(error: &str) -> (String, String) {
    let error = error
        .strip_prefix("invalid inline document: ")
        .unwrap_or(error);
    let parts = error.split(": ").collect::<Vec<_>>();
    let mut split = 0;
    // Rust domain validation may attach several nested locations, e.g.
    // blocks[0].program: nodes[2].props.style: unsupported property.
    while split + 1 < parts.len()
        && !parts[split].contains(' ')
        && parts[split].len() <= 180
        && parts[split]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'[' | b']'))
    {
        split += 1;
    }
    if split == 0 {
        ("document".into(), error.into())
    } else {
        (parts[..split].join("."), parts[split..].join(": "))
    }
}

fn location(
    value: &Value,
    raw_path: &str,
    prefix: &str,
) -> (String, Option<String>, Option<String>) {
    let mut path = raw_path.to_owned();
    let mut block_id = None;
    let mut node_id = None;
    // Parse only decimal array indices from validated paths; never echo input values.
    if let Some(after) = path.strip_prefix("blocks[")
        && let Some((index, _)) = after.split_once(']')
        && let Ok(index) = index.parse::<usize>()
    {
        let block = value
            .get("blocks")
            .and_then(Value::as_array)
            .and_then(|a| a.get(index));
        block_id = block
            .and_then(|b| b.get("id"))
            .and_then(Value::as_str)
            .filter(|s| agena_domain::ui_identifier(s))
            .map(str::to_owned);
        if let Some(after_nodes) = path.split(".nodes[").nth(1)
            && let Some((node_index, _)) = after_nodes.split_once(']')
            && let Ok(node_index) = node_index.parse::<usize>()
        {
            node_id = block
                .and_then(|b| b.pointer("/program/nodes"))
                .and_then(Value::as_array)
                .and_then(|a| a.get(node_index))
                .and_then(|n| n.get("id"))
                .and_then(Value::as_str)
                .filter(|s| agena_domain::ui_identifier(s))
                .map(str::to_owned);
        }
    }
    if path == "document" {
        path = String::new();
    }
    let path = format!("{prefix}{path}");
    let safe_path = if path.len() <= 180
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'[' | b']'))
    {
        path
    } else {
        prefix.trim_end_matches('.').to_owned()
    };
    (safe_path, block_id, node_id)
}

fn issue(value: &Value, error: &str, prefix: &str) -> InlineIssue {
    let (path, reason) = path_and_reason(error);
    let (path, block_id, node_id) = location(value, &path, prefix);
    InlineIssue {
        path,
        block_id,
        node_id,
        reason: safe_text(&reason),
        fix: fix_for(&reason),
    }
}

fn as_plugin_error(report: InlineErrorReport) -> PluginError {
    let leading = &report.issues[0];
    let detail = format!(
        "{} at {}: {}. Fix: {}{}",
        report.code,
        leading.path,
        leading.reason,
        leading.fix,
        if report.issues.len() > 1 {
            " Other field issues are in structured details."
        } else {
            ""
        }
    );
    let mut error = PluginError::invalid_params_with_data(
        report.code,
        json!({"agena_inline_ui_error": report}),
    );
    error.failure.user = agena_failure::UserPresentation::validated("inline_ui.error", detail);
    error
}

pub(super) fn invalid_document(value: &Value, error: &str) -> PluginError {
    let mut issues = vec![issue(value, error, "document.")];
    let mut truncated = false;
    if let Some(blocks) = value.get("blocks").and_then(Value::as_array) {
        for (index, block) in blocks.iter().enumerate() {
            let single = json!({"version":1,"blocks":[block]});
            if let Err(reason) = InlineDocument::from_value(single) {
                let reason = reason.replacen("blocks[0]", &format!("blocks[{index}]"), 1);
                let next = issue(value, &reason, "document.");
                if issues.iter().any(|previous| previous.path == next.path) {
                    continue;
                }
                if issues.len() == MAX_ISSUES {
                    truncated = true;
                    break;
                }
                issues.push(next);
            }
        }
    }
    as_plugin_error(InlineErrorReport {
        code: "invalid_ui",
        applied: false,
        issues,
        truncated,
        expected_revision: None,
        current_revision: None,
    })
}

pub(super) fn invalid_update(update: &InlineUpdate, error: InlineUiUpdateError) -> PluginError {
    match error {
        InlineUiUpdateError::RevisionConflict { expected, current } => {
            as_plugin_error(InlineErrorReport {
                code: "revision_conflict",
                applied: false,
                issues: vec![InlineIssue {
                    path: "update.expected_revision".into(),
                    block_id: None,
                    node_id: None,
                    reason: format!("expected {expected}, current {current}"),
                    fix: "Read the current UI, reconcile concurrent edits and deliberately retry.",
                }],
                truncated: false,
                expected_revision: Some(expected),
                current_revision: Some(current),
            })
        }
        InlineUiUpdateError::Invalid(reason) => {
            let (value, prefix) = match &update.document {
                Some(document) => (
                    serde_json::to_value(document).unwrap_or(Value::Null),
                    "update.document.",
                ),
                None => (Value::Null, "update.patch."),
            };
            let mut item = issue(&value, &reason, prefix);
            if let Some(patch) = &update.patch {
                item.block_id = Some(patch.block_id.clone());
                // A candidate's flat nodes index is not the index of a patch
                // upsert. Match an unsupported property to an actual supplied
                // upsert by key only when there is exactly one match; otherwise
                // avoid inventing a node ID or a misleading input path.
                let property = reason
                    .split(".props.")
                    .nth(1)
                    .and_then(|part| part.split(": ").next())
                    .filter(|key| agena_domain::ui_identifier(key));
                let matching = property.and_then(|key| {
                    let mut matches = patch
                        .nodes
                        .iter()
                        .enumerate()
                        .filter(|(_, node)| node.props.contains_key(key));
                    let first = matches.next()?;
                    matches
                        .next()
                        .is_none()
                        .then_some((first.0, first.1.id.as_str(), key))
                });
                if let Some((index, node_id, key)) = matching {
                    item.path = format!("update.patch.nodes[{index}].props.{key}");
                    item.node_id = Some(node_id.to_owned());
                } else if reason.contains("panel block not found")
                    || reason.contains("patch target")
                {
                    item.path = "update.patch.block_id".into();
                } else if reason.contains("cannot remove") || reason.contains("remove") {
                    item.path = "update.patch.remove".into();
                } else if !patch.data.is_empty()
                    && patch.nodes.is_empty()
                    && patch.remove.is_empty()
                {
                    item.path = "update.patch.data".into();
                } else if !patch.nodes.is_empty()
                    && patch.data.is_empty()
                    && patch.remove.is_empty()
                {
                    item.path = "update.patch.nodes".into();
                } else {
                    item.path = "update.patch".into();
                }
            } else if update.document.is_none() {
                item.path = "update".into();
            }
            as_plugin_error(InlineErrorReport {
                code: "invalid_ui",
                applied: false,
                issues: vec![item],
                truncated: false,
                expected_revision: Some(update.expected_revision),
                current_revision: None,
            })
        }
        InlineUiUpdateError::NotFound => as_plugin_error(InlineErrorReport {
            code: "not_found",
            applied: false,
            issues: vec![InlineIssue {
                path: "update.reference".into(),
                block_id: None,
                node_id: None,
                reason: "UI is not present in this session".into(),
                fix: "Use an existing reference from this session; read or prepare as appropriate.",
            }],
            truncated: false,
            expected_revision: Some(update.expected_revision),
            current_revision: None,
        }),
        // An I/O failure may be observed after an atomic filesystem write.
        // Do not claim applied=false, nor instruct a blind retry.
        InlineUiUpdateError::Storage(error) => PluginError::from_kind_with_public_detail(
            PluginErrorKind::Internal,
            error,
            "UI storage failed; commit status is uncertain. Inspect the reference before any retry.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_block_errors_are_bounded_with_explicit_truncation() {
        let blocks = (0..12)
            .map(|index| {
                json!({
                    "type":"paragraph", "id":format!("block{index}"),
                    "text":"Example", "unknown_property":true,
                })
            })
            .collect::<Vec<_>>();
        let document = json!({"version":1,"blocks":blocks});
        let reason = InlineDocument::from_value(document.clone()).unwrap_err();
        let error = invalid_document(&document, &reason);
        let report = &error.diagnostic.data.as_ref().unwrap()["agena_inline_ui_error"];
        assert_eq!(report["code"], "invalid_ui");
        assert_eq!(report["applied"], false);
        assert_eq!(report["truncated"], true);
        let issues = report["issues"].as_array().unwrap();
        assert_eq!(issues.len(), MAX_ISSUES);
        let paths = issues
            .iter()
            .map(|item| item["path"].as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(paths.len(), MAX_ISSUES);
        assert!(issues.iter().all(|item| {
            item["block_id"]
                .as_str()
                .is_some_and(|id| id.starts_with("block"))
        }));
        assert!(
            !error.failure.user.fallback.contains("Example"),
            "do not echo input content"
        );
    }
}
