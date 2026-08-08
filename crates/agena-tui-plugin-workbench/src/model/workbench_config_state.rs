pub fn recompute_plugin_config_state(plugin: &mut PluginWorkbenchPlugin) {
    plugin.draft_override = derive_override_value(&plugin.default_config, &plugin.draft_config);
    plugin.dirty = normalize_override_value(plugin.draft_override.clone())
        != normalize_override_value(plugin.saved_override.clone());
    plugin.diagnostics = validate_config_value(
        plugin.schema.as_ref(),
        &plugin.draft_config,
        plugin.schema_missing,
    );
    plugin
        .diagnostics
        .extend(plugin_semantic_diagnostics(plugin));
    plugin.runtime_diagnostics = runtime_diagnostics(&plugin.status);
    plugin.diff = diff_config_values(&plugin.saved_config, &plugin.draft_config);
    plugin.sections = build_config_sections(plugin);
    plugin.config_status = config_status_for_plugin(plugin);
}

pub(crate) fn plugin_config_error_count(plugin: &PluginWorkbenchPlugin) -> usize {
    plugin
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
        .count()
}

pub fn plugin_save_block_reason(plugin: &PluginWorkbenchPlugin) -> Option<String> {
    let errors = plugin_config_error_count(plugin);
    (errors > 0).then(|| {
        format!(
            "cannot save {} config with {errors} error(s)",
            plugin.plugin_id
        )
    })
}

pub(crate) fn config_status_for_plugin(plugin: &PluginWorkbenchPlugin) -> PluginConfigStatus {
    if !plugin.runtime_diagnostics.is_empty() {
        return PluginConfigStatus {
            kind: PluginConfigStatusKind::RuntimeIssue,
            label: format!("Runtime issue {}", plugin.runtime_diagnostics.len()),
        };
    }
    if plugin.dirty {
        return PluginConfigStatus {
            kind: PluginConfigStatusKind::NeedsRestart,
            label: "Needs restart".to_owned(),
        };
    }
    let errors = plugin_config_error_count(plugin);
    if errors > 0 {
        return PluginConfigStatus {
            kind: PluginConfigStatusKind::Invalid,
            label: format!("Invalid {errors}"),
        };
    }
    let warnings = plugin
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Warning)
        .count();
    if warnings > 0 {
        return PluginConfigStatus {
            kind: if plugin.schema_missing {
                PluginConfigStatusKind::SchemaMissing
            } else {
                PluginConfigStatusKind::Warning
            },
            label: if plugin.schema_missing {
                "Schema missing".to_owned()
            } else {
                format!("Warning {warnings}")
            },
        };
    }
    if plugin.configured_plugin_value.is_none() {
        return PluginConfigStatus {
            kind: PluginConfigStatusKind::Missing,
            label: "Missing".to_owned(),
        };
    }
    PluginConfigStatus {
        kind: PluginConfigStatusKind::Valid,
        label: "Valid".to_owned(),
    }
}

pub(crate) fn normalize_override_value(value: JsonValue) -> JsonValue {
    match value {
        JsonValue::Object(object) => JsonValue::Object(
            object
                .into_iter()
                .map(|(key, value)| (key, normalize_override_value(value)))
                .collect(),
        ),
        JsonValue::Array(items) => JsonValue::Array(
            items
                .into_iter()
                .map(normalize_override_value)
                .collect::<Vec<_>>(),
        ),
        other => other,
    }
}

pub fn persisted_plugin_config_value(plugin: &PluginWorkbenchPlugin) -> JsonValue {
    normalize_override_value(plugin.draft_override.clone())
}

pub(crate) fn derive_override_value(default: &JsonValue, effective: &JsonValue) -> JsonValue {
    derive_override_option(default, effective).unwrap_or(JsonValue::Null)
}

pub(crate) fn derive_override_option(
    default: &JsonValue,
    effective: &JsonValue,
) -> Option<JsonValue> {
    if default == effective {
        return None;
    }
    match (default, effective) {
        (JsonValue::Object(default), JsonValue::Object(effective)) => {
            let mut patch = JsonMap::new();
            let keys = default
                .keys()
                .chain(effective.keys())
                .cloned()
                .collect::<BTreeSet<_>>();
            for key in keys {
                let child_default = default.get(key.as_str()).unwrap_or(&JsonValue::Null);
                let child_effective = effective.get(key.as_str()).unwrap_or(&JsonValue::Null);
                if let Some(child_patch) = derive_override_option(child_default, child_effective) {
                    patch.insert(key, child_patch);
                }
            }
            (!patch.is_empty()).then_some(JsonValue::Object(patch))
        }
        (JsonValue::Array(default), JsonValue::Array(effective)) => {
            (default != effective).then_some(JsonValue::Array(effective.clone()))
        }
        _ => Some(effective.clone()),
    }
}

pub fn row_paths(row: &ConfigRowView) -> Vec<&ConfigPath> {
    std::iter::once(&row.primary_path)
        .chain(row.additional_paths.iter())
        .collect()
}

pub(crate) fn reset_effective_value_at_path(
    value: &mut JsonValue,
    default_root: &JsonValue,
    path: &[PathSegment],
) -> bool {
    let path = path.to_vec();
    let before = get_value_at_path(value, &path)
        .cloned()
        .unwrap_or(JsonValue::Null);
    if let Some(default_value) = get_value_at_path(default_root, &path).cloned() {
        set_value_at_path(value, &path, default_value);
    } else if remove_value_at_path(value, &path).is_none() {
        return false;
    }
    get_value_at_path(value, &path)
        .cloned()
        .unwrap_or(JsonValue::Null)
        != before
}

pub(crate) fn path_present_in_value(value: &JsonValue, path: &[PathSegment]) -> bool {
    let mut cursor = value;
    for segment in path {
        match segment {
            PathSegment::Key(key) => {
                let Some(next) = cursor.as_object().and_then(|object| object.get(key)) else {
                    return false;
                };
                cursor = next;
            }
            PathSegment::Index(index) => {
                let Some(next) = cursor.as_array().and_then(|items| items.get(*index)) else {
                    return false;
                };
                cursor = next;
            }
        }
    }
    true
}

pub(crate) fn path_is_prefix_of(prefix: &[PathSegment], path: &[PathSegment]) -> bool {
    prefix.len() <= path.len()
        && prefix
            .iter()
            .zip(path.iter())
            .all(|(left, right)| left == right)
}

pub(crate) fn section_row_count(section: &ConfigSectionView, view: PluginConfigView) -> usize {
    match &section.body {
        ConfigSectionBody::Overview { .. } => 0,
        ConfigSectionBody::Form { groups, .. } => groups
            .iter()
            .map(|group| {
                group
                    .rows
                    .iter()
                    .filter(|row| row_visible(row, view))
                    .count()
            })
            .sum(),
    }
}

pub fn section_row_at(
    section: &ConfigSectionView,
    view: PluginConfigView,
    index: usize,
) -> Option<&ConfigRowView> {
    let ConfigSectionBody::Form { groups, .. } = &section.body else {
        return None;
    };
    let mut visible_index = 0usize;
    for group in groups {
        for row in &group.rows {
            if !row_visible(row, view) {
                continue;
            }
            if visible_index == index {
                return Some(row);
            }
            visible_index += 1;
        }
    }
    None
}

pub fn find_row_position(
    plugin: &PluginWorkbenchPlugin,
    view: PluginConfigView,
    path: &[PathSegment],
) -> Option<(usize, usize)> {
    for (section_index, section) in plugin.sections.iter().enumerate() {
        let mut row_index = 0usize;
        let ConfigSectionBody::Form { groups, .. } = &section.body else {
            continue;
        };
        for group in groups {
            for row in &group.rows {
                if !row_visible(row, view) {
                    continue;
                }
                if row.primary_path.as_slice() == path
                    || row
                        .additional_paths
                        .iter()
                        .any(|candidate| candidate.as_slice() == path)
                {
                    return Some((section_index, row_index));
                }
                row_index += 1;
            }
        }
    }
    None
}

pub fn find_best_section_row_for_path(
    plugin: &PluginWorkbenchPlugin,
    view: PluginConfigView,
    target_path: &[PathSegment],
) -> Option<(usize, usize, ConfigRowView)> {
    let mut best: Option<(usize, usize, usize, ConfigRowView)> = None;
    for (section_index, section) in plugin.sections.iter().enumerate() {
        let mut row_index = 0usize;
        let ConfigSectionBody::Form { groups, .. } = &section.body else {
            continue;
        };
        for group in groups {
            for row in &group.rows {
                if !row_visible(row, view) {
                    continue;
                }
                if let Some(prefix_len) = row_best_path_prefix_len(row, target_path) {
                    let replace = best
                        .as_ref()
                        .is_none_or(|(_, _, current_len, _)| prefix_len > *current_len);
                    if replace {
                        best = Some((section_index, row_index, prefix_len, row.clone()));
                    }
                }
                row_index += 1;
            }
        }
    }
    best.map(|(section_index, row_index, _, row)| (section_index, row_index, row))
}

pub fn find_best_drilldown_row_for_path(
    overlay: &PluginConfigDrilldownOverlay,
    view: PluginConfigView,
    target_path: &[PathSegment],
) -> Option<(usize, ConfigRowView)> {
    let mut best: Option<(usize, usize, ConfigRowView)> = None;
    let mut visible_index = 0usize;
    for group in &overlay.groups {
        for row in &group.rows {
            if !row_visible(row, view) {
                continue;
            }
            if let Some(prefix_len) = row_best_path_prefix_len(row, target_path) {
                let replace = best
                    .as_ref()
                    .is_none_or(|(_, current_len, _)| prefix_len > *current_len);
                if replace {
                    best = Some((visible_index, prefix_len, row.clone()));
                }
            }
            visible_index += 1;
        }
    }
    best.map(|(row_index, _, row)| (row_index, row))
}

pub(crate) fn row_best_path_prefix_len(
    row: &ConfigRowView,
    target_path: &[PathSegment],
) -> Option<usize> {
    std::iter::once(&row.primary_path)
        .chain(row.additional_paths.iter())
        .filter(|candidate| path_is_prefix_of(candidate.as_slice(), target_path))
        .map(|candidate| candidate.len())
        .max()
}

pub fn move_selected_config_section(dialog: &mut PluginWorkbenchOverlay, delta: isize) {
    let item_count = dialog
        .selected_plugin()
        .map(|plugin| plugin.sections.len())
        .unwrap_or_default();
    move_index(&mut dialog.selected_section, item_count, delta);
    dialog.selected_node = 0;
    dialog.clamp_selection();
}

pub fn move_selected_bottom_panel_row(dialog: &mut PluginWorkbenchOverlay, delta: isize) {
    let item_count = if dialog.show_diff {
        dialog
            .selected_plugin()
            .map(|plugin| plugin.diff.len())
            .unwrap_or_default()
    } else {
        dialog
            .selected_plugin()
            .map(plugin_all_diagnostics)
            .map(|diagnostics| diagnostics.len())
            .unwrap_or_default()
    };
    if dialog.show_diff {
        move_index(&mut dialog.selected_diff_row, item_count, delta);
    } else {
        move_index(&mut dialog.selected_diagnostic, item_count, delta);
    }
    dialog.clamp_selection();
}

pub fn select_config_path(
    dialog: &mut PluginWorkbenchOverlay,
    plugin_id: &str,
    path: &[PathSegment],
) {
    let Some(plugin) = dialog
        .plugins
        .iter()
        .find(|plugin| plugin.plugin_id == plugin_id)
    else {
        return;
    };
    if let Some((section_index, row_index)) = find_row_position(plugin, dialog.config_view, path) {
        dialog.selected_section = section_index;
        dialog.selected_node = row_index;
    }
    dialog.clamp_selection();
}

pub(crate) fn row_visible(_row: &ConfigRowView, _view: PluginConfigView) -> bool {
    true
}

pub fn row_rename_action_allowed(plugin: &PluginWorkbenchPlugin, path: &[PathSegment]) -> bool {
    let Some((parent_path, key)) = path_key_info(path) else {
        return false;
    };
    let Some(root_schema) = plugin.schema.as_ref() else {
        return true;
    };
    let Some(parent_schema) =
        schema_for_path(root_schema, root_schema, &plugin.draft_config, &parent_path)
    else {
        return true;
    };
    !schema_declared_property_keys(&parent_schema).contains(&key)
}

pub(crate) fn array_item_primary_action(
    info: ArrayItemActionInfo,
) -> Option<ConfigRowPrimaryAction> {
    if info.can_insert_after {
        Some(ConfigRowPrimaryAction::InsertAfter)
    } else if info.can_duplicate {
        Some(ConfigRowPrimaryAction::Duplicate)
    } else if info.can_move_down {
        Some(ConfigRowPrimaryAction::MoveDown)
    } else if info.can_move_up {
        Some(ConfigRowPrimaryAction::MoveUp)
    } else if info.can_remove {
        Some(ConfigRowPrimaryAction::Remove)
    } else {
        None
    }
}

pub fn config_row_primary_action(
    plugin: &PluginWorkbenchPlugin,
    editor: &ConfigRowEditor,
    primary_path: &[PathSegment],
    additional_paths: &[ConfigPath],
) -> Option<ConfigRowPrimaryAction> {
    if let Some(info) = array_item_action_info(plugin, primary_path)
        && info.has_any_action()
    {
        return array_item_primary_action(info);
    }
    if let ConfigRowEditor::Structured { path } = editor
        && let Some(value) = get_value_at_path(&plugin.draft_config, path)
    {
        match value {
            JsonValue::Object(_) => {
                if object_add_field_block_reason(plugin.schema.as_ref(), &plugin.draft_config, path)
                    .is_none()
                {
                    return Some(ConfigRowPrimaryAction::AddField);
                }
            }
            JsonValue::Array(_) if can_append_array_item(plugin, path.as_slice()) => {
                return Some(ConfigRowPrimaryAction::AddItem);
            }
            _ => {}
        }
    }
    (additional_paths.is_empty() && row_rename_action_allowed(plugin, primary_path))
        .then_some(ConfigRowPrimaryAction::Rename)
}

pub(crate) fn config_row_action_display(
    plugin: &PluginWorkbenchPlugin,
    editor: &ConfigRowEditor,
    primary_path: &[PathSegment],
    additional_paths: &[ConfigPath],
) -> Option<String> {
    config_row_primary_action(plugin, editor, primary_path, additional_paths)
        .map(ConfigRowPrimaryAction::label)
        .map(str::to_owned)
}

pub(crate) fn config_row_cell_fallback_order(
    layout: ConfigGroupLayout,
    preferred: ConfigRowCell,
) -> &'static [ConfigRowCell] {
    match layout {
        ConfigGroupLayout::Standard => match preferred {
            ConfigRowCell::Type => &[
                ConfigRowCell::Type,
                ConfigRowCell::Value,
                ConfigRowCell::Default,
                ConfigRowCell::Action,
                ConfigRowCell::State,
            ],
            ConfigRowCell::Value => &[
                ConfigRowCell::Value,
                ConfigRowCell::Default,
                ConfigRowCell::Action,
                ConfigRowCell::State,
                ConfigRowCell::Type,
            ],
            ConfigRowCell::SecondaryValue => &[
                ConfigRowCell::Value,
                ConfigRowCell::Default,
                ConfigRowCell::Action,
                ConfigRowCell::State,
                ConfigRowCell::Type,
            ],
            ConfigRowCell::Default => &[
                ConfigRowCell::Default,
                ConfigRowCell::Action,
                ConfigRowCell::State,
                ConfigRowCell::Value,
                ConfigRowCell::Type,
            ],
            ConfigRowCell::Action => &[
                ConfigRowCell::Action,
                ConfigRowCell::State,
                ConfigRowCell::Default,
                ConfigRowCell::Value,
                ConfigRowCell::Type,
            ],
            ConfigRowCell::State => &[
                ConfigRowCell::State,
                ConfigRowCell::Action,
                ConfigRowCell::Default,
                ConfigRowCell::Value,
                ConfigRowCell::Type,
            ],
        },
        ConfigGroupLayout::Pair { .. } => match preferred {
            ConfigRowCell::Type | ConfigRowCell::Default | ConfigRowCell::Value => &[
                ConfigRowCell::Value,
                ConfigRowCell::SecondaryValue,
                ConfigRowCell::Action,
                ConfigRowCell::State,
            ],
            ConfigRowCell::SecondaryValue => &[
                ConfigRowCell::SecondaryValue,
                ConfigRowCell::Value,
                ConfigRowCell::Action,
                ConfigRowCell::State,
            ],
            ConfigRowCell::Action => &[
                ConfigRowCell::Action,
                ConfigRowCell::State,
                ConfigRowCell::SecondaryValue,
                ConfigRowCell::Value,
            ],
            ConfigRowCell::State => &[
                ConfigRowCell::State,
                ConfigRowCell::Action,
                ConfigRowCell::SecondaryValue,
                ConfigRowCell::Value,
            ],
        },
    }
}

pub(crate) fn row_cells(row: &ConfigRowView, layout: ConfigGroupLayout) -> Vec<ConfigRowCell> {
    match layout {
        ConfigGroupLayout::Standard => {
            let mut cells = Vec::new();
            if row.type_mode.is_switchable() {
                cells.push(ConfigRowCell::Type);
            }
            cells.push(ConfigRowCell::Value);
            cells.push(ConfigRowCell::Default);
            if row.action_display.is_some() {
                cells.push(ConfigRowCell::Action);
            }
            cells.push(ConfigRowCell::State);
            cells
        }
        ConfigGroupLayout::Pair { .. } => {
            let mut cells = vec![ConfigRowCell::Value, ConfigRowCell::SecondaryValue];
            if row.action_display.is_some() {
                cells.push(ConfigRowCell::Action);
            }
            cells.push(ConfigRowCell::State);
            cells
        }
    }
}

pub fn normalize_config_row_cell(
    row: &ConfigRowView,
    layout: ConfigGroupLayout,
    preferred: ConfigRowCell,
) -> ConfigRowCell {
    let cells = row_cells(row, layout);
    config_row_cell_fallback_order(layout, preferred)
        .iter()
        .copied()
        .find(|candidate| cells.contains(candidate))
        .or_else(|| cells.first().copied())
        .unwrap_or(ConfigRowCell::Value)
}

pub fn move_config_row_cell(
    row: &ConfigRowView,
    layout: ConfigGroupLayout,
    current: ConfigRowCell,
    delta: isize,
) -> Option<ConfigRowCell> {
    let cells = row_cells(row, layout);
    let current = normalize_config_row_cell(row, layout, current);
    let index = cells
        .iter()
        .position(|cell| *cell == current)
        .unwrap_or_default();
    let next = (index as isize + delta).clamp(0, cells.len().saturating_sub(1) as isize) as usize;
    (next != index).then(|| cells[next])
}

pub(crate) fn config_row_cell_label(
    row: &ConfigRowView,
    layout: ConfigGroupLayout,
    cell: ConfigRowCell,
) -> &'static str {
    match (layout, normalize_config_row_cell(row, layout, cell)) {
        (ConfigGroupLayout::Standard, ConfigRowCell::Type) => "Type",
        (ConfigGroupLayout::Standard, ConfigRowCell::Value) => "Value",
        (ConfigGroupLayout::Standard, ConfigRowCell::Default) => "Default",
        (ConfigGroupLayout::Standard, ConfigRowCell::Action) => "Action",
        (ConfigGroupLayout::Standard, ConfigRowCell::State) => "State",
        (ConfigGroupLayout::Standard, _) => "Value",
        (ConfigGroupLayout::Pair { left_label, .. }, ConfigRowCell::Value) => left_label,
        (ConfigGroupLayout::Pair { right_label, .. }, ConfigRowCell::SecondaryValue) => right_label,
        (ConfigGroupLayout::Pair { .. }, ConfigRowCell::Action) => "Action",
        (ConfigGroupLayout::Pair { .. }, ConfigRowCell::State) => "State",
        (ConfigGroupLayout::Pair { left_label, .. }, _) => left_label,
    }
}

pub(crate) fn group_has_action_column(group: &ConfigGroupView, view: PluginConfigView) -> bool {
    group
        .rows
        .iter()
        .filter(|row| row_visible(row, view))
        .any(|row| row.action_display.is_some())
}

pub(crate) fn section_selected_row_cell(
    section: &ConfigSectionView,
    view: PluginConfigView,
    index: usize,
    preferred: ConfigRowCell,
) -> ConfigRowCell {
    let Some(row) = section_row_at(section, view, index) else {
        return ConfigRowCell::Value;
    };
    let layout = section_group_for_row(section, view, index)
        .map(|group| group.layout)
        .unwrap_or(ConfigGroupLayout::Standard);
    normalize_config_row_cell(row, layout, preferred)
}

pub fn drilldown_selected_row_cell(
    overlay: &PluginConfigDrilldownOverlay,
    view: PluginConfigView,
    preferred: ConfigRowCell,
) -> ConfigRowCell {
    let Some(row) = drilldown_row_at(overlay, view, overlay.selected_row) else {
        return ConfigRowCell::Value;
    };
    let layout = drilldown_group_for_row(overlay, view, overlay.selected_row)
        .map(|group| group.layout)
        .unwrap_or(ConfigGroupLayout::Standard);
    normalize_config_row_cell(row, layout, preferred)
}

pub(crate) fn drilldown_selected_row_cell_for_groups(
    groups: &[ConfigGroupView],
    view: PluginConfigView,
    index: usize,
    preferred: ConfigRowCell,
) -> ConfigRowCell {
    let Some(row) = drilldown_row_at_groups(groups, view, index) else {
        return ConfigRowCell::Value;
    };
    let layout = drilldown_group_for_row_in_groups(groups, view, index)
        .map(|group| group.layout)
        .unwrap_or(ConfigGroupLayout::Standard);
    normalize_config_row_cell(row, layout, preferred)
}

#[derive(Debug, Clone)]
/// Context of the selected config row.
pub struct SelectedConfigRowContext {
    pub plugin_id: String,
    pub row: ConfigRowView,
    pub layout: ConfigGroupLayout,
    pub cell: ConfigRowCell,
    pub group_title: String,
    pub group_paths: Vec<ConfigPath>,
}

pub fn selected_config_row_context(
    dialog: &PluginWorkbenchOverlay,
) -> Option<SelectedConfigRowContext> {
    if let Some(overlay) = dialog.current_drilldown() {
        let row = drilldown_row_at(overlay, dialog.config_view, overlay.selected_row)?.clone();
        let group = drilldown_group_for_row(overlay, dialog.config_view, overlay.selected_row)?;
        return Some(SelectedConfigRowContext {
            plugin_id: overlay.plugin_id.clone(),
            row,
            layout: group.layout,
            cell: drilldown_selected_row_cell(overlay, dialog.config_view, overlay.selected_cell),
            group_title: group.title.clone(),
            group_paths: group_row_paths(group),
        });
    }
    let plugin = dialog.selected_plugin()?;
    let section = dialog.selected_section()?;
    let row = section_row_at(section, dialog.config_view, dialog.selected_node)?.clone();
    let group = section_group_for_row(section, dialog.config_view, dialog.selected_node)?;
    Some(SelectedConfigRowContext {
        plugin_id: plugin.plugin_id.clone(),
        row,
        layout: group.layout,
        cell: section_selected_row_cell(
            section,
            dialog.config_view,
            dialog.selected_node,
            dialog.selected_cell,
        ),
        group_title: group.title.clone(),
        group_paths: group_row_paths(group),
    })
}

pub(crate) fn group_row_paths(group: &ConfigGroupView) -> Vec<ConfigPath> {
    let mut paths = Vec::new();
    for row in &group.rows {
        for path in row_paths(row) {
            paths.push(path.clone());
        }
    }
    paths
}

pub fn section_group_for_row(
    section: &ConfigSectionView,
    view: PluginConfigView,
    index: usize,
) -> Option<&ConfigGroupView> {
    let ConfigSectionBody::Form { groups, .. } = &section.body else {
        return None;
    };
    let mut visible_index = 0usize;
    for group in groups {
        for row in &group.rows {
            if !row_visible(row, view) {
                continue;
            }
            if visible_index == index {
                return Some(group);
            }
            visible_index += 1;
        }
    }
    None
}

pub fn drilldown_group_for_row(
    overlay: &PluginConfigDrilldownOverlay,
    view: PluginConfigView,
    index: usize,
) -> Option<&ConfigGroupView> {
    drilldown_group_for_row_in_groups(&overlay.groups, view, index)
}

pub(crate) fn drilldown_group_for_row_in_groups(
    groups: &[ConfigGroupView],
    view: PluginConfigView,
    index: usize,
) -> Option<&ConfigGroupView> {
    let mut visible_index = 0usize;
    for group in groups {
        for row in &group.rows {
            if !row_visible(row, view) {
                continue;
            }
            if visible_index == index {
                return Some(group);
            }
            visible_index += 1;
        }
    }
    None
}

pub fn build_drilldown_groups(
    plugin: &PluginWorkbenchPlugin,
    path: &ConfigPath,
    title: &str,
) -> Vec<ConfigGroupView> {
    let value = get_value_at_path(&plugin.draft_config, path).unwrap_or(&JsonValue::Null);
    match value {
        JsonValue::Object(_) => build_generic_object_groups(plugin, path, title),
        JsonValue::Array(items) => {
            let rows = items
                .iter()
                .enumerate()
                .map(|(index, _)| {
                    let mut item_path = path.clone();
                    item_path.push(PathSegment::Index(index));
                    build_row_for_path(plugin, item_path, format!("Item {index}").as_str(), None)
                })
                .collect::<Vec<_>>();
            vec![ConfigGroupView {
                title: title.to_owned(),
                layout: ConfigGroupLayout::Standard,
                rows,
            }]
        }
        _ => vec![ConfigGroupView {
            title: title.to_owned(),
            layout: ConfigGroupLayout::Standard,
            rows: vec![build_row_for_path(plugin, path.clone(), title, None)],
        }],
    }
}

pub fn drilldown_row_count(
    overlay: &PluginConfigDrilldownOverlay,
    view: PluginConfigView,
) -> usize {
    overlay
        .groups
        .iter()
        .map(|group| {
            group
                .rows
                .iter()
                .filter(|row| row_visible(row, view))
                .count()
        })
        .sum()
}

pub fn drilldown_row_at(
    overlay: &PluginConfigDrilldownOverlay,
    view: PluginConfigView,
    index: usize,
) -> Option<&ConfigRowView> {
    drilldown_row_at_groups(&overlay.groups, view, index)
}

pub(crate) fn drilldown_row_at_groups(
    groups: &[ConfigGroupView],
    view: PluginConfigView,
    index: usize,
) -> Option<&ConfigRowView> {
    let mut visible_index = 0usize;
    for group in groups {
        for row in &group.rows {
            if !row_visible(row, view) {
                continue;
            }
            if visible_index == index {
                return Some(row);
            }
            visible_index += 1;
        }
    }
    None
}

pub(crate) fn rebuild_drilldown_overlay(
    dialog: &PluginWorkbenchOverlay,
    previous: &PluginConfigDrilldownOverlay,
) -> Option<PluginConfigDrilldownOverlay> {
    let plugin = dialog
        .plugins
        .iter()
        .find(|plugin| plugin.plugin_id == previous.plugin_id)?;
    let groups = build_drilldown_groups(plugin, &previous.path, previous.title.as_str());
    let row_count = groups
        .iter()
        .map(|group| {
            group
                .rows
                .iter()
                .filter(|row| row_visible(row, dialog.config_view))
                .count()
        })
        .sum::<usize>();
    let selected_row = if row_count == 0 {
        0
    } else {
        previous.selected_row.min(row_count.saturating_sub(1))
    };
    let selected_cell = drilldown_selected_row_cell_for_groups(
        &groups,
        dialog.config_view,
        selected_row,
        previous.selected_cell,
    );
    Some(PluginConfigDrilldownOverlay {
        plugin_id: previous.plugin_id.clone(),
        path: previous.path.clone(),
        title: previous.title.clone(),
        groups,
        selected_row,
        selected_cell,
    })
}

pub fn rebuild_drilldown_stack(
    dialog: &PluginWorkbenchOverlay,
    previous_stack: &[PluginConfigDrilldownOverlay],
) -> Vec<PluginConfigDrilldownOverlay> {
    previous_stack
        .iter()
        .filter_map(|overlay| rebuild_drilldown_overlay(dialog, overlay))
        .collect()
}
use super::{
    ArrayItemActionInfo, BTreeSet, ConfigGroupLayout, ConfigGroupView, ConfigPath, ConfigRowCell,
    ConfigRowEditor, ConfigRowPrimaryAction, ConfigRowView, ConfigSectionBody, ConfigSectionView,
    DiagnosticSeverity, JsonMap, JsonValue, PathSegment, PluginConfigDrilldownOverlay,
    PluginConfigStatus, PluginConfigStatusKind, PluginConfigView, PluginWorkbenchOverlay,
    PluginWorkbenchPlugin, array_item_action_info, build_config_sections,
    build_generic_object_groups, build_row_for_path, can_append_array_item, diff_config_values,
    get_value_at_path, move_index, object_add_field_block_reason, path_key_info,
    plugin_all_diagnostics, plugin_semantic_diagnostics, remove_value_at_path, runtime_diagnostics,
    schema_declared_property_keys, schema_for_path, set_value_at_path, validate_config_value,
};
