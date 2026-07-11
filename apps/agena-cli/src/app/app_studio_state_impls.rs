impl SectionedListSection for PermissionStudioSection {
    type Item = PermissionStudioItem;

    fn items(&self) -> &[Self::Item] {
        self.items.as_slice()
    }
}

impl SettingsSourceRow {
    pub(in crate::app) fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
        }
    }
}

impl SettingsStudioItem {
    pub(in crate::app) fn new(
        label: impl Into<String>,
        value: impl Into<String>,
        detail: impl Into<String>,
        action: SettingsPickerAction,
    ) -> Self {
        let value = value.into();
        let current_value = (!value.trim().is_empty()).then(|| value.clone());
        Self::from_parts(
            label,
            value,
            detail,
            None,
            current_value,
            None,
            Vec::new(),
            action,
        )
    }

    pub(in crate::app) fn from_parts(
        label: impl Into<String>,
        value: impl Into<String>,
        detail: impl Into<String>,
        path: Option<String>,
        current_value: Option<String>,
        effective_value: Option<String>,
        source_rows: Vec<SettingsSourceRow>,
        action: SettingsPickerAction,
    ) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            detail: detail.into(),
            path,
            current_value,
            effective_value,
            source_rows,
            action,
        }
    }
}

impl SectionedListSection for SettingsStudioSection {
    type Item = SettingsStudioItem;

    fn items(&self) -> &[Self::Item] {
        self.items.as_slice()
    }
}
use crate::app::{
    PermissionStudioItem, PermissionStudioSection, SettingsPickerAction, SettingsSourceRow,
    SettingsStudioItem, SettingsStudioSection,
};
use agena_tui_components::SectionedListSection;
