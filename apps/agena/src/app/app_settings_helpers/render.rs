pub(in crate::app) fn build_app_detail_text(lines: Vec<DetailTextLine<'static>>) -> Text<'static> {
    build_detail_text(lines, &DetailTextSpec::label_width(14))
}

pub(in crate::app) fn app_detail_labeled_line(
    label: impl Into<String>,
    value: impl Into<String>,
) -> DetailTextLine<'static> {
    let label = label.into();
    let value = value.into();
    DetailTextLine::labeled(
        label,
        sanitize_terminal_text(value.as_str()),
        Style::default().fg(agena_tui_components::theme::muted_color()),
        Style::default(),
    )
}

pub(in crate::app) fn app_detail_plain_line(text: impl Into<String>) -> DetailTextLine<'static> {
    let text = text.into();
    DetailTextLine::plain(sanitize_terminal_text(text.as_str()), Style::default())
}

pub(in crate::app) fn app_detail_heading_line(text: impl Into<String>) -> DetailTextLine<'static> {
    let text = text.into();
    DetailTextLine::plain(
        sanitize_terminal_text(text.as_str()),
        Style::default()
            .fg(agena_tui_components::theme::accent_color())
            .add_modifier(Modifier::BOLD),
    )
}

pub(in crate::app) fn localized_yes_no(i18n: &I18n, value: bool) -> String {
    ui_text::t(i18n, if value { "value-yes" } else { "value-no" })
}

pub(in crate::app) fn agent_profile_overview_hint_key(
    storage: AgentProfileStorage,
) -> &'static str {
    match storage {
        AgentProfileStorage::BuiltIn => "overlay-agent-overview-built-in",
        AgentProfileStorage::Config => "overlay-agent-overview-config-editable",
        AgentProfileStorage::Markdown => "overlay-agent-overview-markdown-editable",
        AgentProfileStorage::Runtime => "overlay-agent-overview-runtime-read-only",
    }
}

pub(in crate::app) fn agent_editability_hint(i18n: &I18n, storage: AgentProfileStorage) -> String {
    ui_text::t(
        i18n,
        match storage {
            AgentProfileStorage::BuiltIn => "overlay-agent-detail-built-in-hint",
            AgentProfileStorage::Config => "overlay-agent-detail-config-editable-hint",
            AgentProfileStorage::Markdown => "overlay-agent-detail-markdown-editable-hint",
            AgentProfileStorage::Runtime => "overlay-agent-detail-runtime-read-only-hint",
        },
    )
}

pub(in crate::app) fn agent_studio_editor_config(
    i18n: &I18n,
    profile: &AgentProfile,
    field: AgentStudioField,
) -> (String, String, String, bool, Editor) {
    let multiline = matches!(
        field,
        AgentStudioField::Description | AgentStudioField::Prompt
    );
    let title = settings_edit_title(i18n, agent_studio_field_label(i18n, field).as_str());
    let prompt = agent_studio_field_prompt(i18n, field);
    let footer = editor_save_footer(i18n, multiline);
    let input = Editor::from_text(agent_studio_field_input_text(profile, field));
    (title, prompt, footer, multiline, input)
}

pub(in crate::app) fn agent_studio_field_label(i18n: &I18n, field: AgentStudioField) -> String {
    ui_text::t(
        i18n,
        match field {
            AgentStudioField::Description => "agent-studio-field-description",
            AgentStudioField::Prompt => "agent-studio-field-prompt",
            AgentStudioField::DefaultProvider => "agent-studio-field-default-provider",
            AgentStudioField::DefaultAdapter => "agent-studio-field-default-adapter",
            AgentStudioField::DefaultModel => "agent-studio-field-default-model",
        },
    )
}

pub(in crate::app) fn agent_studio_field_prompt(i18n: &I18n, field: AgentStudioField) -> String {
    ui_text::t(
        i18n,
        match field {
            AgentStudioField::Description => "agent-studio-field-prompt-description",
            AgentStudioField::Prompt => "agent-studio-field-prompt-prompt",
            AgentStudioField::DefaultProvider => "agent-studio-field-prompt-default-provider",
            AgentStudioField::DefaultAdapter => "agent-studio-field-prompt-default-adapter",
            AgentStudioField::DefaultModel => "agent-studio-field-prompt-default-model",
        },
    )
}

pub(in crate::app) fn agent_studio_field_input_text(
    profile: &AgentProfile,
    field: AgentStudioField,
) -> String {
    match field {
        AgentStudioField::Description => profile.description.clone(),
        AgentStudioField::Prompt => profile.prompt.clone(),
        AgentStudioField::DefaultProvider => profile.defaults.provider.clone().unwrap_or_default(),
        AgentStudioField::DefaultAdapter => profile.defaults.adapter.clone().unwrap_or_default(),
        AgentStudioField::DefaultModel => profile.defaults.model.clone().unwrap_or_default(),
    }
}

pub(in crate::app) fn apply_agent_studio_field_to_profile(
    profile: &mut AgentProfile,
    field: AgentStudioField,
    input: &str,
) {
    let trimmed = input.trim();
    match field {
        AgentStudioField::Description => {
            profile.description = if trimmed.is_empty() {
                String::new()
            } else {
                input.to_string()
            };
        }
        AgentStudioField::Prompt => {
            profile.prompt = if trimmed.is_empty() {
                String::new()
            } else {
                input.to_string()
            };
        }
        AgentStudioField::DefaultProvider => {
            profile.defaults.provider = (!trimmed.is_empty()).then(|| trimmed.to_string());
        }
        AgentStudioField::DefaultAdapter => {
            profile.defaults.adapter = (!trimmed.is_empty()).then(|| trimmed.to_string());
        }
        AgentStudioField::DefaultModel => {
            profile.defaults.model = (!trimmed.is_empty()).then(|| trimmed.to_string());
        }
    }
}

pub(in crate::app) fn agent_studio_field_setting_value(
    _i18n: &I18n,
    agent_name: &str,
    field: AgentStudioField,
    input: &str,
) -> UiResult<(String, Option<JsonValue>)> {
    let trimmed = input.trim();
    let path = match field {
        AgentStudioField::Description => agent_config_path(agent_name, "description"),
        AgentStudioField::Prompt => agent_config_path(agent_name, "prompt"),
        AgentStudioField::DefaultProvider => agent_config_path(agent_name, "defaults.provider"),
        AgentStudioField::DefaultAdapter => agent_config_path(agent_name, "defaults.adapter"),
        AgentStudioField::DefaultModel => agent_config_path(agent_name, "defaults.model"),
    };
    let value = match field {
        AgentStudioField::Description | AgentStudioField::Prompt => {
            (!trimmed.is_empty()).then_some(JsonValue::String(input.to_string()))
        }
        AgentStudioField::DefaultProvider
        | AgentStudioField::DefaultAdapter
        | AgentStudioField::DefaultModel => {
            (!trimmed.is_empty()).then_some(JsonValue::String(trimmed.to_string()))
        }
    };
    Ok((path, value))
}

#[derive(serde::Serialize)]
struct AgentMarkdownTools<'a> {
    allow: &'a [String],
}

#[derive(serde::Serialize)]
struct AgentMarkdownFrontmatter<'a> {
    #[serde(skip_serializing_if = "str::is_empty")]
    description: &'a str,
    #[serde(skip_serializing_if = "PermissionConfig::is_empty")]
    permission: &'a PermissionConfig,
    #[serde(skip_serializing_if = "AgentSelectionConfig::is_empty")]
    defaults: &'a AgentSelectionConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<AgentMarkdownTools<'a>>,
}

pub(in crate::app) fn agent_markdown_document(profile: &AgentProfile) -> UiResult<String> {
    let frontmatter = AgentMarkdownFrontmatter {
        description: profile.description.as_str(),
        permission: &profile.permission,
        defaults: &profile.defaults,
        tools: (!profile.allowed_tools.is_empty()).then_some(AgentMarkdownTools {
            allow: profile.allowed_tools.as_slice(),
        }),
    };
    let prompt = profile.prompt.trim_start_matches('\n');
    let frontmatter_empty = frontmatter.description.trim().is_empty()
        && frontmatter.permission.is_empty()
        && frontmatter.defaults.is_empty()
        && frontmatter.tools.is_none();
    if frontmatter_empty {
        return Ok(if prompt.is_empty() {
            String::new()
        } else {
            format!("{}\n", prompt.trim_end_matches('\n'))
        });
    }
    let yaml = serde_yaml::to_string(&frontmatter).map_err(|error| error.to_string())?;
    let yaml = yaml
        .strip_prefix("---\n")
        .unwrap_or(yaml.as_str())
        .trim_end();
    Ok(format!(
        "---\n{yaml}\n---\n{}\n",
        prompt.trim_end_matches('\n')
    ))
}
use crate::app::{
    AgentProfile, AgentProfileStorage, AgentSelectionConfig, AgentStudioField, DetailTextLine,
    DetailTextSpec, Editor, I18n, JsonValue, Modifier, PermissionConfig, Style, Text, UiResult,
    agent_config_path, build_detail_text, editor_save_footer, sanitize_terminal_text,
    settings_edit_title, ui_text,
};
