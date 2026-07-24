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
        PermissionMode::Ask => agena_api::resource::PermissionMode::Ask,
        PermissionMode::Deny => agena_api::resource::PermissionMode::Deny,
    }
}

pub(crate) fn parse_permission_mode_token(
    i18n: &I18n,
    token: &str,
) -> std::result::Result<PermissionMode, String> {
    match token.to_ascii_lowercase().as_str() {
        "allow" => Ok(PermissionMode::Allow),
        "ask" => Ok(PermissionMode::Ask),
        "deny" => Ok(PermissionMode::Deny),
        _ => Err(ui_text::t(i18n, "permission-rule-error-invalid-mode")),
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

pub(crate) fn split_command_args_once(value: &str) -> Option<(&str, &str)> {
    let mut parts = value.splitn(2, char::is_whitespace);
    let first = parts.next()?.trim();
    let second = parts.next()?.trim();
    if first.is_empty() || second.is_empty() {
        None
    } else {
        Some((first, second))
    }
}

pub(crate) fn plugin_command_slash_name(
    entry: &agena_plugin_host::PluginCommandCatalogItem,
) -> Option<String> {
    let name = entry
        .command
        .slash
        .as_deref()?
        .trim()
        .trim_start_matches('/');
    (!name.is_empty() && !name.chars().any(char::is_whitespace)).then(|| name.to_string())
}

pub(crate) fn plugin_command_matches_name(
    entry: &agena_plugin_host::PluginCommandCatalogItem,
    name: &str,
) -> bool {
    let name = name.trim().trim_start_matches('/');
    plugin_command_slash_name(entry).is_some_and(|slash| slash.eq_ignore_ascii_case(name))
        || entry.command.aliases.iter().any(|alias| {
            alias
                .trim()
                .trim_start_matches('/')
                .eq_ignore_ascii_case(name)
        })
}

pub(crate) fn plugin_command_matches_slash_query(
    entry: &agena_plugin_host::PluginCommandCatalogItem,
    query: &str,
) -> bool {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return true;
    }
    plugin_command_slash_name(entry).is_some_and(|name| {
        let name = name.to_ascii_lowercase();
        name == query || name.starts_with(query.as_str())
    }) || entry.command.aliases.iter().any(|alias| {
        let alias = alias.trim().trim_start_matches('/').to_ascii_lowercase();
        alias == query || alias.starts_with(query.as_str())
    })
}

pub(crate) fn plugin_command_detail(entry: &agena_plugin_host::PluginCommandCatalogItem) -> String {
    let description = entry.command.description.trim();
    if description.is_empty() {
        format!("{} | {}", entry.plugin_id, entry.command.title)
    } else {
        format!("{} | {description}", entry.plugin_id)
    }
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
    FileMentionSuggestionContext, I18n, PermissionMode, PermissionRuleDraft,
    PermissionRuleSubjectKind, SlashCommandSuggestionContext, UpsertPermissionRuleParams,
    non_empty_owned, ui_text,
};

#[cfg(test)]
mod plugin_command_tests {
    use agena_plugin_host::{
        PluginCommandCatalogItem, PluginCommandDefinition, PluginKey, PluginUiAction,
    };

    use super::{
        plugin_command_matches_name, plugin_command_matches_slash_query, plugin_command_slash_name,
    };

    fn command(slash: Option<&str>, aliases: &[&str]) -> PluginCommandCatalogItem {
        PluginCommandCatalogItem {
            plugin_id: "example.commands"
                .parse::<PluginKey>()
                .expect("valid plugin id"),
            command: PluginCommandDefinition {
                id: "example.run".to_string(),
                title: "Run example".to_string(),
                description: "Run a human-visible plugin command.".to_string(),
                category: "Test".to_string(),
                slash: slash.map(str::to_string),
                aliases: aliases.iter().map(|alias| (*alias).to_string()).collect(),
                usage: None,
                location: "command_palette".to_string(),
                input_schema: None,
                handler: Some("example.run".to_string()),
                action: PluginUiAction::InvokeCommand {
                    command: "example.run".to_string(),
                    input: None,
                },
            },
        }
    }

    #[test]
    fn plugin_slash_command_uses_only_explicit_command_metadata() {
        assert_eq!(
            plugin_command_slash_name(&command(Some(" /example "), &[])).as_deref(),
            Some("example")
        );
        assert_eq!(plugin_command_slash_name(&command(None, &[])), None);
        assert_eq!(
            plugin_command_slash_name(&command(Some("/not a command"), &[])),
            None
        );
    }

    #[test]
    fn plugin_slash_command_matches_primary_name_and_declared_aliases() {
        let command = command(Some("/example"), &["demo", "/sample"]);
        assert!(plugin_command_matches_name(&command, "example"));
        assert!(plugin_command_matches_name(&command, "/DEMO"));
        assert!(plugin_command_matches_slash_query(&command, "sam"));
        assert!(!plugin_command_matches_name(&command, "unrelated-tool"));
        assert!(!plugin_command_matches_slash_query(&command, "tool"));
    }
}
