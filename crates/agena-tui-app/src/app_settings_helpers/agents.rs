use super::{
    agent_editability_hint, agent_profile_overview_hint_key, app_detail_labeled_line,
    app_detail_plain_line, build_app_detail_text, localized_yes_no,
    settings_source_rows_for_config_path, settings_source_rows_for_workspace_config_path,
    settings_studio_field_items,
};

pub(crate) fn quoted_settings_segment(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

pub(crate) fn settings_studio_plugin_items(
    i18n: &I18n,
    sources: &ConfigJsonSources,
) -> Vec<SettingsStudioItem<SettingsPickerAction>> {
    let mut items =
        settings_studio_field_items(i18n, sources, SettingsStudioSectionId::PluginsTools);
    items.push(SettingsStudioItem::from_parts(
        ui_text::t(i18n, "settings-plugin-workbench-label"),
        ui_text::t(i18n, "value-open"),
        ui_text::t(i18n, "settings-plugin-workbench-detail"),
        None,
        None,
        None,
        Vec::new(),
        SettingsPickerAction::OpenPluginWorkbench,
    ));
    items
}

pub(crate) fn agent_default_summary(i18n: &I18n, default: &AgentSelectionConfig) -> String {
    let mut parts = Vec::new();
    if let Some(provider) = default
        .provider
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "inline-fact-provider").as_str(),
            provider,
        ));
    }
    if let Some(adapter) = default
        .adapter
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "inline-fact-adapter").as_str(),
            adapter,
        ));
    }
    if let Some(model) = default
        .model
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "inline-fact-model").as_str(),
            model,
        ));
    }
    if let Some(thinking_mode) = default
        .thinking_mode
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "inline-fact-thinking").as_str(),
            ui_text::thinking_mode_display_value(thinking_mode).as_str(),
        ));
    }
    if let Some(speed_mode) = default
        .speed_mode
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "inline-fact-speed").as_str(),
            ui_text::speed_mode_display_value(speed_mode).as_str(),
        ));
    }
    if let Some(verbosity) = default
        .verbosity
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "inline-fact-verbosity").as_str(),
            verbosity,
        ));
    }
    if let Some(parallel_tool_calls) = default.parallel_tool_calls {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "inline-fact-parallel-tools").as_str(),
            if parallel_tool_calls { "on" } else { "off" },
        ));
    }
    if parts.is_empty() {
        ui_text::t(i18n, "value-inherits-runtime-model-defaults")
    } else {
        join_inline_segments(parts)
    }
}

pub(crate) fn agent_permission_summary(
    i18n: &I18n,
    permission: &agena_domain::PermissionConfig,
) -> String {
    if permission.is_empty() {
        return ui_text::t(i18n, "value-inherits-runtime-defaults");
    }

    let mut parts = Vec::new();
    if let Some(path) = permission.path.as_ref() {
        let mut detail: Vec<String> = Vec::new();
        if path.workspace.is_some() {
            detail.push(ui_text::t(i18n, "value-workspace"));
        }
        if path.external.is_some() {
            detail.push(ui_text::t(i18n, "value-external"));
        }
        if !path.rules.is_empty() {
            detail.push(i18n.text_args(
                "value-rule-count",
                &agena_tui::fl_args!("count" => path.rules.len() as i64),
            ));
        }
        parts.push(i18n.text_args(
            "value-path-summary",
            &agena_tui::fl_args!(
                "detail" => if detail.is_empty() {
                    ui_text::t(i18n, "value-custom")
                } else {
                    join_inline_segments(detail)
                }
            ),
        ));
    }
    if let Some(network) = permission.network.as_ref() {
        let mut detail: Vec<String> = Vec::new();
        if network.internet.is_some() {
            detail.push(ui_text::t(i18n, "value-internet"));
        }
        if network.private.is_some() {
            detail.push(ui_text::t(i18n, "value-private"));
        }
        if network.loopback.is_some() {
            detail.push(ui_text::t(i18n, "value-loopback"));
        }
        if !network.rules.is_empty() {
            detail.push(i18n.text_args(
                "value-rule-count",
                &agena_tui::fl_args!("count" => network.rules.len() as i64),
            ));
        }
        parts.push(i18n.text_args(
            "value-network-summary",
            &agena_tui::fl_args!(
                "detail" => if detail.is_empty() {
                    ui_text::t(i18n, "value-custom")
                } else {
                    join_inline_segments(detail)
                }
            ),
        ));
    }
    if let Some(tools) = permission.tools.as_ref() {
        let mut detail: Vec<String> = Vec::new();
        if !tools.tags.is_empty() {
            detail.push(i18n.text_args(
                "value-tag-count",
                &agena_tui::fl_args!("count" => tools.tags.len() as i64),
            ));
        }
        if !tools.names.is_empty() {
            detail.push(i18n.text_args(
                "value-name-count",
                &agena_tui::fl_args!("count" => tools.names.len() as i64),
            ));
        }
        if !tools.plugin.is_empty() {
            detail.push(i18n.text_args(
                "value-plugin-override-count",
                &agena_tui::fl_args!("count" => tools.plugin.len() as i64),
            ));
        }
        if !tools.rules.is_empty() {
            detail.push(i18n.text_args(
                "value-rule-set-count",
                &agena_tui::fl_args!("count" => tools.rules.len() as i64),
            ));
        }
        parts.push(i18n.text_args(
            "value-tools-summary",
            &agena_tui::fl_args!(
                "detail" => if detail.is_empty() {
                    ui_text::t(i18n, "value-custom")
                } else {
                    join_inline_segments(detail)
                }
            ),
        ));
    }

    if parts.is_empty() {
        ui_text::t(i18n, "value-inherits-runtime-defaults")
    } else {
        join_inline_segments(parts)
    }
}

pub(crate) fn settings_studio_agent_browser_item(
    i18n: &I18n,
    agent_count: usize,
    default_agent: Option<&str>,
) -> SettingsStudioItem<SettingsPickerAction> {
    SettingsStudioItem::new(
        ui_text::t(i18n, "settings-agent-browser-label"),
        match default_agent {
            Some(default) => i18n.text_args(
                "settings-agent-browser-value-default",
                &agena_tui::fl_args!(
                    "count" => agent_count as i64,
                    "default" => default.to_string(),
                ),
            ),
            None => i18n.text_args(
                "settings-agent-browser-value",
                &agena_tui::fl_args!("count" => agent_count as i64),
            ),
        },
        ui_text::t(i18n, "settings-agent-browser-detail"),
        SettingsPickerAction::OpenAgentList,
    )
}

pub(crate) fn permission_layer_source_rows(
    i18n: &I18n,
    global_permission: &PermissionConfig,
    workspace_permission: &PermissionConfig,
    session: Option<&SessionPermissionStudioState>,
) -> Vec<SettingsStudioSourceRow> {
    let mut rows = vec![
        SettingsStudioSourceRow::new(
            ui_text::t(i18n, "settings-permission-layer-global"),
            permission_override_summary(i18n, global_permission),
        ),
        SettingsStudioSourceRow::new(
            ui_text::t(i18n, "settings-permission-layer-workspace"),
            permission_override_summary(i18n, workspace_permission),
        ),
    ];
    if let Some(session) = session {
        rows.push(SettingsStudioSourceRow::new(
            session
                .agent_name
                .as_deref()
                .map(|name| {
                    i18n.text_args(
                        "settings-permission-layer-agent-named",
                        &agena_tui::fl_args!("agent" => name.to_string()),
                    )
                })
                .unwrap_or_else(|| ui_text::t(i18n, "settings-permission-layer-agent")),
            session
                .agent_permission
                .as_ref()
                .map(|permission| permission_override_summary(i18n, permission))
                .unwrap_or_else(|| ui_text::t(i18n, "settings-source-unset")),
        ));
        rows.push(SettingsStudioSourceRow::new(
            ui_text::t(i18n, "settings-permission-layer-session"),
            permission_override_summary(i18n, &session.permission),
        ));
        rows.push(SettingsStudioSourceRow::new(
            ui_text::t(i18n, "settings-permission-layer-effective"),
            permission_override_summary(i18n, &session.effective_permission),
        ));
    }
    rows
}

pub(crate) fn settings_studio_permission_items(
    i18n: &I18n,
    sources: &ConfigJsonSources,
    global_permission: &PermissionConfig,
    workspace_permission: &PermissionConfig,
    effective_permission: &PermissionConfig,
    current_session: Option<&SessionPermissionStudioState>,
) -> Vec<SettingsStudioItem<SettingsPickerAction>> {
    let mut items = Vec::new();
    if let Some(session) = current_session {
        let effective_summary = permission_override_summary(i18n, &session.effective_permission);
        items.push(SettingsStudioItem::from_parts(
            ui_text::t(i18n, "settings-permission-effective-label"),
            effective_summary.clone(),
            i18n.text_args(
                "settings-permission-effective-detail",
                &agena_tui::fl_args!("session" => session.session_title.clone()),
            ),
            None,
            Some(effective_summary.clone()),
            Some(effective_summary),
            permission_layer_source_rows(
                i18n,
                global_permission,
                workspace_permission,
                Some(session),
            ),
            SettingsPickerAction::OpenSessionEffectivePermissionView(session.session_id),
        ));
        let session_summary = permission_override_summary(i18n, &session.permission);
        let session_effective_summary =
            permission_override_summary(i18n, &session.effective_permission);
        let session_source_rows = {
            let mut rows = permission_layer_source_rows(
                i18n,
                global_permission,
                workspace_permission,
                Some(session),
            );
            rows.push(SettingsStudioSourceRow::new(
                ui_text::t(i18n, "settings-source-row-write-target"),
                ui_text::t(i18n, "settings-source-current-session"),
            ));
            rows
        };
        items.push(SettingsStudioItem::from_parts(
            ui_text::t(i18n, "settings-permission-current-label"),
            session_summary.clone(),
            i18n.text_args(
                "settings-permission-current-detail",
                &agena_tui::fl_args!("session" => session.session_title.clone()),
            ),
            None,
            Some(session_summary.clone()),
            Some(session_effective_summary),
            session_source_rows,
            SettingsPickerAction::OpenCurrentSessionPermissionWorkbench,
        ));
    }

    let global_summary = permission_override_summary(i18n, global_permission);
    let workspace_summary = permission_override_summary(i18n, workspace_permission);
    let effective_summary = permission_override_summary(i18n, effective_permission);
    let global_source_rows = settings_source_rows_for_config_path(
        i18n,
        sources,
        "permission",
        global_summary.clone(),
        effective_summary.clone(),
    );
    items.push(SettingsStudioItem::from_parts(
        ui_text::t(i18n, "settings-permission-global-label"),
        global_summary.clone(),
        ui_text::t(i18n, "settings-permission-global-detail"),
        Some("permission".to_string()),
        Some(global_summary),
        Some(effective_summary),
        global_source_rows,
        SettingsPickerAction::OpenGlobalPermissionWorkbench,
    ));
    let workspace_effective_summary = permission_override_summary(i18n, effective_permission);
    let workspace_source_rows = settings_source_rows_for_workspace_config_path(
        i18n,
        sources,
        "permission",
        workspace_summary.clone(),
        workspace_effective_summary.clone(),
    );
    items.push(SettingsStudioItem::from_parts(
        ui_text::t(i18n, "settings-permission-workspace-label"),
        workspace_summary.clone(),
        ui_text::t(i18n, "settings-permission-workspace-detail"),
        Some("permission".to_string()),
        Some(workspace_summary),
        Some(workspace_effective_summary),
        workspace_source_rows,
        SettingsPickerAction::OpenWorkspacePermissionWorkbench,
    ));
    items
}

pub(crate) fn agent_picker_item(
    i18n: &I18n,
    agent: AgentDescriptor,
    default_agent: Option<&str>,
    config_owned: bool,
) -> (
    agena_tui::selection_picker::SelectionPickerItem,
    SelectionPickerCommand,
) {
    let storage = agent_descriptor_storage(&agent, config_owned);
    let source = match storage {
        AgentProfileStorage::BuiltIn => ui_text::t(i18n, "value-built-in"),
        AgentProfileStorage::Config => ui_text::t(i18n, "value-runtime-config"),
        AgentProfileStorage::Markdown => agent
            .source_path
            .as_ref()
            .cloned()
            .unwrap_or_else(|| ui_text::t(i18n, "value-markdown-backed")),
        AgentProfileStorage::Runtime => ui_text::t(i18n, "value-runtime-registered"),
    };
    let mut detail = vec![
        agent_scope_label_localized(i18n, agent.scope),
        agent_profile_storage_label_localized(i18n, storage),
        source,
    ];
    if default_agent.is_some_and(|name| name == agent.name.as_str()) {
        detail.push(ui_text::t(i18n, "value-default"));
    }
    let description = agent.description.trim();
    if !description.is_empty() {
        detail.push(description.to_string());
    }
    let label = agent.name.clone();
    let detail = join_inline_segments(detail);
    (
        agena_tui::selection_picker::SelectionPickerItem::new(
            format!("agent:{}", agent.name),
            label.clone(),
            detail.clone(),
            format!("{label} {detail}"),
        ),
        SelectionPickerCommand::Agent { name: agent.name },
    )
}

pub(crate) fn agent_descriptor_storage(
    agent: &AgentDescriptor,
    config_owned: bool,
) -> AgentProfileStorage {
    if matches!(agent.scope, AgentScope::Default) {
        AgentProfileStorage::BuiltIn
    } else if agent.source_path.is_some() {
        AgentProfileStorage::Markdown
    } else if config_owned {
        AgentProfileStorage::Config
    } else {
        AgentProfileStorage::Runtime
    }
}

pub(crate) fn agent_scope_label_localized(i18n: &I18n, scope: AgentScope) -> String {
    ui_text::t(
        i18n,
        match scope {
            AgentScope::Project => "value-agent-scope-project",
            AgentScope::User => "value-agent-scope-user",
            AgentScope::Default => "value-agent-scope-default",
        },
    )
}

pub(crate) fn agent_list_create_item(
    i18n: &I18n,
) -> (
    agena_tui::selection_picker::SelectionPickerItem,
    SelectionPickerCommand,
) {
    let label = ui_text::t(i18n, "overlay-agent-list-create-label");
    let detail = ui_text::t(i18n, "overlay-agent-list-create-detail");
    (
        agena_tui::selection_picker::SelectionPickerItem::new(
            "action:create-agent",
            label.clone(),
            detail.clone(),
            format!("{label} {detail}"),
        )
        .always_visible(),
        SelectionPickerCommand::AgentCreate,
    )
}

pub(crate) fn agent_list_items(
    i18n: &I18n,
    mut agents: Vec<AgentDescriptor>,
    default_agent: Option<&str>,
    config_agents: &HashSet<String>,
) -> Vec<(
    agena_tui::selection_picker::SelectionPickerItem,
    SelectionPickerCommand,
)> {
    agents.sort_by(|left, right| left.name.cmp(&right.name));
    let mut items = vec![agent_list_create_item(i18n)];
    items.extend(agents.into_iter().map(|agent| {
        let config_owned = config_agents.contains(agent.name.as_str());
        agent_picker_item(i18n, agent, default_agent, config_owned)
    }));
    items
}

pub(crate) fn session_agent_picker_items(
    i18n: &I18n,
    mut agents: Vec<AgentDescriptor>,
    current_agent: Option<&str>,
    default_agent: Option<&str>,
    config_agents: &HashSet<String>,
) -> Vec<(
    agena_tui::selection_picker::SelectionPickerItem,
    SelectionPickerCommand,
)> {
    agents.sort_by(|left, right| left.name.cmp(&right.name));
    agents
        .into_iter()
        .map(|agent| {
            let current = current_agent.is_some_and(|name| name == agent.name.as_str());
            let config_owned = config_agents.contains(agent.name.as_str());
            let (item, action) = agent_picker_item(i18n, agent, default_agent, config_owned);
            let SelectionPickerCommand::Agent { name } = action else {
                unreachable!("agent_picker_item must retain the agent action");
            };
            let item = if current {
                item.with_prefix("✓ ")
            } else {
                item
            };
            (
                agena_tui::selection_picker::SelectionPickerItem {
                    key: format!("session-agent:{name}"),
                    ..item
                },
                SelectionPickerCommand::SessionAgent { name },
            )
        })
        .collect()
}

pub(crate) fn agent_profile_storage(
    profile: &AgentProfile,
    config_owned: bool,
) -> AgentProfileStorage {
    if matches!(profile.scope, AgentScope::Default) {
        AgentProfileStorage::BuiltIn
    } else if profile.source_path.is_some() {
        AgentProfileStorage::Markdown
    } else if config_owned {
        AgentProfileStorage::Config
    } else {
        AgentProfileStorage::Runtime
    }
}

pub(crate) fn agent_profile_storage_label_localized(
    i18n: &I18n,
    storage: AgentProfileStorage,
) -> String {
    ui_text::t(
        i18n,
        match storage {
            AgentProfileStorage::BuiltIn => "value-built-in",
            AgentProfileStorage::Config => "value-config-backed",
            AgentProfileStorage::Markdown => "value-markdown-backed",
            AgentProfileStorage::Runtime => "value-runtime-registered",
        },
    )
}

pub(crate) fn agent_profile_scope_label_localized(i18n: &I18n, profile: &AgentProfile) -> String {
    ui_text::t(
        i18n,
        match profile.scope {
            AgentScope::Project => "value-agent-scope-project",
            AgentScope::User => "value-agent-scope-user",
            AgentScope::Default => "value-agent-scope-default",
        },
    )
}

pub(crate) fn agent_profile_source_label_localized(
    i18n: &I18n,
    profile: &AgentProfile,
    storage: AgentProfileStorage,
) -> String {
    match storage {
        AgentProfileStorage::BuiltIn => ui_text::t(i18n, "value-built-in-defaults"),
        AgentProfileStorage::Config => ui_text::t(i18n, "value-runtime-config-file"),
        AgentProfileStorage::Markdown => profile
            .source_path
            .as_ref()
            .cloned()
            .unwrap_or_else(|| ui_text::t(i18n, "value-markdown-backed")),
        AgentProfileStorage::Runtime => ui_text::t(i18n, "value-runtime-registered"),
    }
}

pub(crate) fn agent_prompt_summary(i18n: &I18n, prompt: &str) -> String {
    if prompt.trim().is_empty() {
        ui_text::t(i18n, "value-unset")
    } else {
        i18n.text_args(
            "value-char-count",
            &agena_tui::fl_args!("count" => prompt.chars().count() as i64),
        )
    }
}

pub(crate) fn agent_optional_string_summary(
    i18n: &I18n,
    value: Option<&str>,
    empty_key: &str,
) -> String {
    value
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| ui_text::t(i18n, empty_key))
}

pub(crate) fn agent_studio_items(
    i18n: &I18n,
    profile: &AgentProfile,
    storage: AgentProfileStorage,
) -> Vec<AgentStudioItem<AgentStudioAction>> {
    vec![
        AgentStudioItem {
            label: ui_text::t(i18n, "agent-studio-field-description"),
            value: agent_optional_string_summary(
                i18n,
                (!profile.description.trim().is_empty()).then_some(profile.description.as_str()),
                "value-unset",
            ),
            detail: ui_text::t(i18n, "agent-studio-item-description-detail"),
            action: AgentStudioAction::Edit(AgentStudioField::Description),
        },
        AgentStudioItem {
            label: ui_text::t(i18n, "agent-studio-field-prompt"),
            value: agent_prompt_summary(i18n, profile.prompt.as_str()),
            detail: ui_text::t(i18n, "agent-studio-item-prompt-detail"),
            action: AgentStudioAction::Edit(AgentStudioField::Prompt),
        },
        AgentStudioItem {
            label: ui_text::t(i18n, "agent-studio-field-default-provider"),
            value: agent_optional_string_summary(
                i18n,
                profile.defaults.provider.as_deref(),
                "value-inherit",
            ),
            detail: ui_text::t(i18n, "agent-studio-item-default-provider-detail"),
            action: AgentStudioAction::Edit(AgentStudioField::DefaultProvider),
        },
        AgentStudioItem {
            label: ui_text::t(i18n, "agent-studio-field-default-adapter"),
            value: agent_optional_string_summary(
                i18n,
                profile.defaults.adapter.as_deref(),
                "value-inherit",
            ),
            detail: ui_text::t(i18n, "agent-studio-item-default-adapter-detail"),
            action: AgentStudioAction::Edit(AgentStudioField::DefaultAdapter),
        },
        AgentStudioItem {
            label: ui_text::t(i18n, "agent-studio-field-default-model"),
            value: agent_optional_string_summary(
                i18n,
                profile.defaults.model.as_deref(),
                "value-inherit",
            ),
            detail: ui_text::t(i18n, "agent-studio-item-default-model-detail"),
            action: AgentStudioAction::Edit(AgentStudioField::DefaultModel),
        },
        AgentStudioItem {
            label: ui_text::t(i18n, "agent-studio-item-permission-policy-label"),
            value: agent_permission_summary(i18n, &profile.permission),
            detail: ui_text::t(i18n, "agent-studio-item-permission-policy-detail"),
            action: AgentStudioAction::OpenPermissionWorkbench,
        },
        AgentStudioItem {
            label: match storage {
                AgentProfileStorage::Markdown => {
                    ui_text::t(i18n, "agent-studio-item-open-source-file")
                }
                AgentProfileStorage::Config => {
                    ui_text::t(i18n, "agent-studio-item-open-config-file")
                }
                AgentProfileStorage::BuiltIn | AgentProfileStorage::Runtime => {
                    ui_text::t(i18n, "agent-studio-item-source-label")
                }
            },
            value: agent_profile_source_label_localized(i18n, profile, storage),
            detail: match storage {
                AgentProfileStorage::Config => {
                    ui_text::t(i18n, "agent-studio-item-open-config-detail")
                }
                AgentProfileStorage::Markdown => {
                    ui_text::t(i18n, "agent-studio-item-open-source-detail")
                }
                AgentProfileStorage::BuiltIn => {
                    ui_text::t(i18n, "agent-studio-item-open-built-in-detail")
                }
                AgentProfileStorage::Runtime => {
                    ui_text::t(i18n, "agent-studio-item-open-runtime-detail")
                }
            },
            action: AgentStudioAction::OpenSource,
        },
    ]
}

pub(crate) fn agent_studio_item_detail_text(
    i18n: &I18n,
    profile: &AgentProfile,
    item: &AgentStudioItem<AgentStudioAction>,
    storage: AgentProfileStorage,
) -> Text<'static> {
    match &item.action {
        AgentStudioAction::Edit(AgentStudioField::Description) => {
            let mut lines = vec![app_detail_plain_line(ui_text::t(
                i18n,
                "overlay-agent-detail-description-help",
            ))];
            lines.push(app_detail_plain_line(String::new()));
            if profile.description.trim().is_empty() {
                lines.push(app_detail_plain_line(ui_text::t(
                    i18n,
                    "overlay-agent-detail-description-unset",
                )));
            } else {
                lines.push(app_detail_plain_line(profile.description.clone()));
            }
            lines.push(app_detail_plain_line(String::new()));
            lines.push(app_detail_plain_line(agent_editability_hint(i18n, storage)));
            build_app_detail_text(lines)
        }
        AgentStudioAction::Edit(AgentStudioField::Prompt) => {
            let mut lines = vec![app_detail_labeled_line(
                ui_text::t(i18n, "overlay-agent-detail-prompt-length"),
                i18n.text_args(
                    "overlay-agent-detail-prompt-chars",
                    &agena_tui::fl_args!("count" => profile.prompt.chars().count() as i64),
                ),
            )];
            lines.push(app_detail_plain_line(String::new()));
            if profile.prompt.trim().is_empty() {
                lines.push(app_detail_plain_line(ui_text::t(
                    i18n,
                    "overlay-agent-detail-prompt-unset",
                )));
            } else {
                lines.push(app_detail_plain_line(profile.prompt.clone()));
            }
            lines.push(app_detail_plain_line(String::new()));
            lines.push(app_detail_plain_line(agent_editability_hint(i18n, storage)));
            build_app_detail_text(lines)
        }
        AgentStudioAction::OpenPermissionWorkbench => {
            let mut lines = vec![
                app_detail_labeled_line(
                    ui_text::t(i18n, "overlay-agent-overview-permission"),
                    agent_permission_summary(i18n, &profile.permission),
                ),
                app_detail_plain_line(String::new()),
            ];
            lines.extend(agent_permission_document_detail_lines(
                i18n,
                &profile.permission,
            ));
            lines.push(app_detail_plain_line(String::new()));
            lines.push(app_detail_plain_line(ui_text::t(
                i18n,
                if storage.editable() {
                    "overlay-agent-detail-open-permission"
                } else {
                    "overlay-agent-detail-open-permission-read-only"
                },
            )));
            build_app_detail_text(lines)
        }
        AgentStudioAction::OpenSource => build_app_detail_text(vec![
            app_detail_labeled_line(
                ui_text::t(i18n, "overlay-agent-overview-source"),
                agent_profile_source_label_localized(i18n, profile, storage),
            ),
            app_detail_labeled_line(
                ui_text::t(i18n, "overlay-agent-overview-scope"),
                agent_profile_scope_label_localized(i18n, profile),
            ),
            app_detail_plain_line(String::new()),
            app_detail_plain_line(item.detail.clone()),
        ]),
        AgentStudioAction::Edit(_) => build_app_detail_text(vec![
            app_detail_plain_line(item.detail.clone()),
            app_detail_labeled_line(
                ui_text::t(i18n, "overlay-detail-current-value"),
                item.value.clone(),
            ),
            app_detail_plain_line(String::new()),
            app_detail_plain_line(agent_editability_hint(i18n, storage)),
        ]),
    }
}

pub(crate) fn agent_studio_overview_text(
    i18n: &I18n,
    profile: &AgentProfile,
    default_agent_name: Option<&str>,
    storage: AgentProfileStorage,
) -> Text<'static> {
    let mut lines = vec![
        app_detail_labeled_line(
            ui_text::t(i18n, "overlay-agent-overview-name"),
            profile.name.clone(),
        ),
        app_detail_labeled_line(
            ui_text::t(i18n, "overlay-agent-overview-scope"),
            agent_profile_scope_label_localized(i18n, profile),
        ),
        app_detail_labeled_line(
            ui_text::t(i18n, "overlay-agent-overview-storage"),
            agent_profile_storage_label_localized(i18n, storage),
        ),
        app_detail_labeled_line(
            ui_text::t(i18n, "overlay-agent-overview-default-agent"),
            localized_yes_no(
                i18n,
                default_agent_name.is_some_and(|name| name == profile.name.as_str()),
            ),
        ),
        app_detail_labeled_line(
            ui_text::t(i18n, "overlay-agent-overview-source"),
            agent_profile_source_label_localized(i18n, profile, storage),
        ),
        app_detail_labeled_line(
            ui_text::t(i18n, "overlay-agent-overview-permission"),
            agent_permission_summary(i18n, &profile.permission),
        ),
    ];
    if !profile.defaults.is_empty() {
        lines.push(app_detail_labeled_line(
            ui_text::t(i18n, "overlay-agent-overview-model-defaults"),
            agent_default_summary(i18n, &profile.defaults),
        ));
    }
    if !profile.description.trim().is_empty() {
        lines.push(app_detail_plain_line(String::new()));
        lines.push(app_detail_plain_line(profile.description.clone()));
    }
    lines.push(app_detail_plain_line(String::new()));
    lines.push(app_detail_plain_line(ui_text::t(
        i18n,
        agent_profile_overview_hint_key(storage),
    )));
    build_app_detail_text(lines)
}

use crate::{
    AgentDescriptor, AgentProfile, AgentProfileStorage, AgentScope, AgentSelectionConfig,
    AgentStudioAction, AgentStudioField, AgentStudioItem, ConfigJsonSources, HashSet, I18n,
    PermissionConfig, SelectionPickerCommand, SessionPermissionStudioState, SettingsPickerAction,
    SettingsStudioItem, SettingsStudioSectionId, SettingsStudioSourceRow, Text,
    agent_permission_document_detail_lines, format_key_value_segment, join_inline_segments,
    permission_override_summary, ui_text,
};

#[cfg(test)]
mod tests {
    use super::session_agent_picker_items;
    use crate::{AgentDescriptor, AgentScope, HashSet, I18n, SelectionPickerCommand};

    fn agent(name: &str) -> AgentDescriptor {
        AgentDescriptor {
            name: name.to_string(),
            description: String::new(),
            permission: Default::default(),
            defaults: Default::default(),
            allowed_tools: Default::default(),
            scope: AgentScope::User,
            source_path: None,
        }
    }

    #[test]
    fn session_agent_picker_sorts_agents_and_marks_only_the_current_one() {
        let items = session_agent_picker_items(
            &I18n::english(),
            vec![agent("review"), agent("build")],
            Some("review"),
            Some("build"),
            &HashSet::new(),
        );

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].0.label, "build");
        assert_eq!(items[1].0.label, "review");
        assert!(matches!(
            &items[0].1,
            SelectionPickerCommand::SessionAgent { name } if name == "build"
        ));
        assert!(matches!(
            &items[1].1,
            SelectionPickerCommand::SessionAgent { name } if name == "review"
        ));
        assert!(items[0].0.prefix.is_none());
        assert_eq!(items[1].0.prefix.as_deref(), Some("✓ "));
    }
}
