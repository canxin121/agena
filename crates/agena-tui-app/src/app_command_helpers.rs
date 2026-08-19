pub(crate) fn permission_rule_params_from_draft(
    draft: &PermissionRuleDraft,
) -> UpsertPermissionRuleParams {
    match draft.subject_kind {
        PermissionRuleSubjectKind::Tool => UpsertPermissionRuleParams {
            action_key: None,
            subject_kind: Some("tool".to_string()),
            tool_name: Some(draft.tool_name.trim().to_string()),
            qualifier: non_empty_owned(draft.qualifier.clone()),
            path_access_kind: None,
            workspace_root: None,
            target_path: None,
            network_target: None,
            network_host: None,
            network_port: None,
            scope: Some(draft.scope.trim().to_string()),
            session_id: if draft.scope.trim() == "session" {
                draft.session_id.trim().parse::<i64>().ok()
            } else {
                None
            },
            mode: permission_mode_to_wire(draft.mode),
        },
        PermissionRuleSubjectKind::PathAccess => UpsertPermissionRuleParams {
            action_key: None,
            subject_kind: Some("path_access".to_string()),
            tool_name: None,
            qualifier: None,
            path_access_kind: Some(draft.path_access_kind.trim().to_string()),
            workspace_root: non_empty_owned(draft.workspace_root.clone()),
            target_path: Some(draft.target_path.trim().to_string()),
            network_target: None,
            network_host: None,
            network_port: None,
            scope: Some(draft.scope.trim().to_string()),
            session_id: if draft.scope.trim() == "session" {
                draft.session_id.trim().parse::<i64>().ok()
            } else {
                None
            },
            mode: permission_mode_to_wire(draft.mode),
        },
        PermissionRuleSubjectKind::NetworkAccess => UpsertPermissionRuleParams {
            action_key: None,
            subject_kind: Some("network_access".to_string()),
            tool_name: None,
            qualifier: None,
            path_access_kind: None,
            workspace_root: None,
            target_path: None,
            network_target: Some(draft.network_target.trim().to_string()),
            network_host: None,
            network_port: None,
            scope: Some(draft.scope.trim().to_string()),
            session_id: if draft.scope.trim() == "session" {
                draft.session_id.trim().parse::<i64>().ok()
            } else {
                None
            },
            mode: permission_mode_to_wire(draft.mode),
        },
    }
}

const fn permission_mode_to_wire(mode: PermissionMode) -> agena_api::resource::PermissionMode {
    match mode {
        PermissionMode::Allow => agena_api::resource::PermissionMode::Allow,
        PermissionMode::Auto => agena_api::resource::PermissionMode::Auto,
        PermissionMode::Ask => agena_api::resource::PermissionMode::Ask,
        PermissionMode::Deny => agena_api::resource::PermissionMode::Deny,
    }
}

pub(crate) fn parse_pr_command_args(
    args: &str,
) -> Result<(String, Option<String>, Option<String>, Option<String>)> {
    let tokens =
        shlex::split(args).ok_or_else(|| anyhow::anyhow!("invalid shell-style arguments"))?;
    let mut title_parts = Vec::new();
    let mut body = None;
    let mut base = None;
    let mut head = None;
    let mut index = 0;
    let mut parsing_options = false;

    while index < tokens.len() {
        let token = tokens[index].as_str();
        match token {
            "--body" => {
                parsing_options = true;
                index += 1;
                let value = tokens
                    .get(index)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --body"))?;
                body = Some(value.clone());
            }
            "--base" => {
                parsing_options = true;
                index += 1;
                let value = tokens
                    .get(index)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --base"))?;
                base = Some(value.clone());
            }
            "--head" => {
                parsing_options = true;
                index += 1;
                let value = tokens
                    .get(index)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --head"))?;
                head = Some(value.clone());
            }
            _ if token.starts_with("--") => {
                return Err(anyhow::anyhow!("unknown /pr option: {token}"));
            }
            _ if parsing_options => {
                return Err(anyhow::anyhow!("unexpected positional argument: {token}"));
            }
            _ => title_parts.push(tokens[index].clone()),
        }
        index += 1;
    }

    if title_parts.is_empty() {
        return Err(anyhow::anyhow!("pull request title is required"));
    }

    Ok((title_parts.join(" "), body, base, head))
}

pub(crate) fn plugin_operation_slash_name(
    entry: &agena_plugin_host::PluginOperationCatalogItem,
) -> Option<String> {
    let name = entry
        .operation
        .slash
        .as_deref()?
        .trim()
        .trim_start_matches('/');
    (!name.is_empty() && !name.chars().any(char::is_whitespace)).then(|| name.to_string())
}

pub(crate) fn plugin_operation_matches_name(
    entry: &agena_plugin_host::PluginOperationCatalogItem,
    name: &str,
) -> bool {
    let name = name.trim().trim_start_matches('/');
    plugin_operation_slash_name(entry).is_some_and(|slash| slash.eq_ignore_ascii_case(name))
        || entry.operation.aliases.iter().any(|alias| {
            alias
                .trim()
                .trim_start_matches('/')
                .eq_ignore_ascii_case(name)
        })
}

pub(crate) fn plugin_operation_matches_slash_query(
    entry: &agena_plugin_host::PluginOperationCatalogItem,
    query: &str,
) -> bool {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return true;
    }
    plugin_operation_slash_name(entry).is_some_and(|name| {
        let name = name.to_ascii_lowercase();
        name == query || name.starts_with(query.as_str())
    }) || entry.operation.aliases.iter().any(|alias| {
        let alias = alias.trim().trim_start_matches('/').to_ascii_lowercase();
        alias == query || alias.starts_with(query.as_str())
    })
}

pub(crate) fn plugin_operation_detail(
    entry: &agena_plugin_host::PluginOperationCatalogItem,
) -> String {
    let description = entry.operation.description.trim();
    if description.is_empty() {
        format!("{} | {}", entry.plugin_id, entry.operation.title)
    } else {
        format!("{} | {description}", entry.plugin_id)
    }
}

/// Whether an operation's shared SettingsContract can materialize and validate
/// a no-argument invocation. Web and TUI therefore use the same rule as the
/// server-owned operation resolver.
pub(crate) fn plugin_operation_accepts_empty_arguments(
    entry: &agena_plugin_host::PluginOperationCatalogItem,
) -> bool {
    entry.accepts_empty_input
}

pub(crate) fn file_mention_suggestion_context_for_text(
    text: &str,
    cursor: usize,
) -> Option<FileMentionSuggestionContext> {
    let token_start = text[..cursor]
        .rfind(char::is_whitespace)
        .map(|index| index + 1)
        .unwrap_or(0);
    let token = text.get(token_start..cursor)?;
    if !token.starts_with('@') || token.starts_with("@@") {
        return None;
    }
    if token[1..].contains('@') || token.contains('\n') {
        return None;
    }
    Some(FileMentionSuggestionContext {
        query: token[1..].to_string(),
        fingerprint: format!("{token}:{cursor}"),
        mention_range: token_start..cursor,
    })
}

pub(crate) fn slash_command_suggestion_context_for_text(
    text: &str,
    cursor: usize,
) -> Option<SlashCommandSuggestionContext> {
    let first_line_end = text.find('\n').unwrap_or(text.len());
    if cursor > first_line_end {
        return None;
    }
    let first_line = &text[..first_line_end];
    if !first_line.starts_with('/') || first_line.starts_with("//") {
        return None;
    }

    let name_start = 1;
    let name_end = first_line[name_start..]
        .find(char::is_whitespace)
        .map(|index| name_start + index)
        .unwrap_or(first_line.len());
    if cursor > name_end {
        return None;
    }

    let name = &first_line[name_start..name_end];
    if name.contains('/') {
        return None;
    }
    let rest_after_name = first_line[name_end..].trim_start();
    if name.is_empty() && !rest_after_name.is_empty() {
        return None;
    }

    Some(SlashCommandSuggestionContext {
        query: name.to_ascii_lowercase(),
        fingerprint: format!("{first_line}:{cursor}"),
        name_range: 0..name_end,
    })
}
use crate::Result;
use crate::{
    FileMentionSuggestionContext, PermissionMode, PermissionRuleDraft, PermissionRuleSubjectKind,
    SlashCommandSuggestionContext, UpsertPermissionRuleParams, non_empty_owned,
};

#[cfg(test)]
mod plugin_operation_tests {
    use agena_plugin_host::sdk::{
        OperationDiscoverability, PluginOperationDefinition, PluginOperationTarget,
        SettingsConstraints, SettingsContract, SettingsNode, SettingsNodeKind,
    };
    use agena_plugin_host::{PluginKey, PluginOperationCatalogItem};

    use super::{
        plugin_operation_accepts_empty_arguments, plugin_operation_matches_name,
        plugin_operation_matches_slash_query, plugin_operation_slash_name,
    };

    fn operation(slash: Option<&str>, aliases: &[&str]) -> PluginOperationCatalogItem {
        PluginOperationCatalogItem {
            plugin_id: "example.operations"
                .parse::<PluginKey>()
                .expect("valid plugin id"),
            accepts_empty_input: true,
            default_input: serde_json::json!({}),
            operation: PluginOperationDefinition {
                id: "example.run".to_string(),
                title: "Run example".to_string(),
                description: "Run a human-visible plugin operation.".to_string(),
                group: "Test".to_string(),
                category: None,
                slash: slash.map(str::to_string),
                aliases: aliases.iter().map(|alias| (*alias).to_string()).collect(),
                usage: None,
                input: SettingsContract::new(SettingsNode::root_object("Input", "")),
                discoverability: OperationDiscoverability::default(),
                target: PluginOperationTarget::Method {
                    handler: "example.run".to_string(),
                },
            },
        }
    }

    #[test]
    fn plugin_slash_operation_uses_only_explicit_metadata() {
        assert_eq!(
            plugin_operation_slash_name(&operation(Some(" example "), &[])).as_deref(),
            Some("example")
        );
        assert_eq!(plugin_operation_slash_name(&operation(None, &[])), None);
        assert_eq!(
            plugin_operation_slash_name(&operation(Some("not an operation"), &[])),
            None
        );
    }

    #[test]
    fn plugin_slash_operation_matches_primary_name_and_aliases() {
        let operation = operation(Some("example"), &["demo", "sample"]);
        assert!(plugin_operation_matches_name(&operation, "example"));
        assert!(plugin_operation_matches_name(&operation, "/DEMO"));
        assert!(plugin_operation_matches_slash_query(&operation, "sam"));
        assert!(!plugin_operation_matches_name(&operation, "unrelated-tool"));
    }

    #[test]
    fn empty_argument_support_comes_from_shared_settings_contract() {
        let mut operation = operation(Some("example"), &[]);
        assert!(plugin_operation_accepts_empty_arguments(&operation));

        operation.operation.input = SettingsContract::new(SettingsNode {
            id: "root".to_string(),
            path: String::new(),
            title: "Input".to_string(),
            description: String::new(),
            required: true,
            default: None,
            constraints: SettingsConstraints::default(),
            sensitive: false,
            secret: false,
            kind: SettingsNodeKind::Object {
                fields: vec![SettingsNode {
                    id: "query".to_string(),
                    path: "/query".to_string(),
                    title: "Query".to_string(),
                    description: String::new(),
                    required: true,
                    default: None,
                    constraints: SettingsConstraints {
                        min_length: Some(1),
                        ..SettingsConstraints::default()
                    },
                    sensitive: false,
                    secret: false,
                    kind: SettingsNodeKind::Text,
                }],
            },
        });
        operation.accepts_empty_input = operation.operation.input.default_value().is_ok();
        assert!(!plugin_operation_accepts_empty_arguments(&operation));
        assert_eq!(
            operation
                .operation
                .input
                .parse_shorthand("release")
                .expect("shared shorthand parser"),
            serde_json::json!({"query":"release"})
        );
    }
}
