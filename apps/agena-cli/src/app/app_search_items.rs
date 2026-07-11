impl SearchListItem for ChoiceItem {
    fn search_list_label(&self) -> String {
        self.label.clone()
    }

    fn search_list_detail(&self) -> Option<String> {
        (!self.detail.trim().is_empty()).then_some(self.detail.clone())
    }

    fn search_list_fill_value(&self) -> String {
        self.value.clone()
    }

    fn search_list_matches_query(&self, query: &str) -> bool {
        query.trim().is_empty()
            || self
                .search_text
                .contains(query.trim().to_ascii_lowercase().as_str())
    }
}

impl SearchListCustomValue<ChoiceOverlayMeta> for ChoiceCustomValue {
    fn search_list_from_input(input: &str, _: &ChoiceOverlayMeta) -> Option<Self> {
        let raw = input.trim().to_string();
        (!raw.is_empty()).then_some(Self { raw })
    }

    fn search_list_label(&self, meta: &ChoiceOverlayMeta) -> String {
        ui_text::t(&meta.i18n, "search-list-custom-value-label")
    }

    fn search_list_detail(&self, meta: &ChoiceOverlayMeta) -> Option<String> {
        Some(meta.i18n.text_args(
            "search-list-custom-value-detail",
            &crate::fl_args!(
                "value" => format_setting_value_inline(&JsonValue::String(self.raw.clone()))
            ),
        ))
    }

    fn search_list_input_text(&self) -> String {
        self.raw.clone()
    }
}

impl SearchListCustomValue<FileAttachOverlayMeta> for TypedPathValue {
    fn search_list_from_input(input: &str, _: &FileAttachOverlayMeta) -> Option<Self> {
        let raw = input.trim().to_string();
        (!raw.is_empty()).then_some(Self { raw })
    }

    fn search_list_label(&self, meta: &FileAttachOverlayMeta) -> String {
        ui_text::t(&meta.i18n, "search-list-custom-path-label")
    }

    fn search_list_detail(&self, _: &FileAttachOverlayMeta) -> Option<String> {
        Some(self.raw.clone())
    }

    fn search_list_input_text(&self) -> String {
        self.raw.clone()
    }
}

impl SearchListCustomValue<PathBrowserOverlayMeta> for TypedPathValue {
    fn search_list_from_input(input: &str, _: &PathBrowserOverlayMeta) -> Option<Self> {
        let raw = input.trim().to_string();
        (!raw.is_empty()).then_some(Self { raw })
    }

    fn search_list_label(&self, meta: &PathBrowserOverlayMeta) -> String {
        ui_text::t(&meta.i18n, "search-list-custom-path-label")
    }

    fn search_list_detail(&self, _: &PathBrowserOverlayMeta) -> Option<String> {
        Some(self.raw.clone())
    }

    fn search_list_input_text(&self) -> String {
        self.raw.clone()
    }
}

impl SearchListItem for PathBrowserItem {
    fn search_list_label(&self) -> String {
        self.label.clone()
    }

    fn search_list_detail(&self) -> Option<String> {
        Some(self.detail.clone())
    }

    fn search_list_fill_value(&self) -> String {
        self.path.display().to_string()
    }

    fn search_list_matches_query(&self, query: &str) -> bool {
        query.trim().is_empty()
            || self
                .label
                .to_ascii_lowercase()
                .contains(query.trim().to_ascii_lowercase().as_str())
            || self
                .detail
                .to_ascii_lowercase()
                .contains(query.trim().to_ascii_lowercase().as_str())
    }

    fn search_list_label_style(&self) -> Style {
        if self.is_dir {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        }
    }
}

impl SearchListItem for SessionSearchItem {
    fn search_list_label(&self) -> String {
        self.label.clone()
    }

    fn search_list_detail(&self) -> Option<String> {
        Some(self.detail.clone())
    }

    fn search_list_fill_value(&self) -> String {
        self.session.title.clone()
    }

    fn search_list_matches_query(&self, query: &str) -> bool {
        session_matches_query(&self.session, query.trim())
    }
}

impl SearchListItem for PickerItem {
    fn search_list_label(&self) -> String {
        self.label.clone()
    }

    fn search_list_detail(&self) -> Option<String> {
        (!self.detail.trim().is_empty()).then_some(self.detail.clone())
    }

    fn search_list_fill_value(&self) -> String {
        self.label.clone()
    }

    fn search_list_matches_query(&self, query: &str) -> bool {
        let raw_query = query.trim();
        if raw_query.is_empty() {
            return true;
        }
        let query = raw_query.to_ascii_lowercase();
        let query = query.as_str();
        match &self.value {
            PickerValue::Command(spec) => {
                commands::command_matches_query(spec, raw_query)
                    || self.detail.to_ascii_lowercase().contains(query)
            }
            PickerValue::ProviderCreate | PickerValue::AgentCreate => true,
            PickerValue::RuntimeTool(_) => {
                self.label.to_ascii_lowercase().contains(query)
                    || self.detail.to_ascii_lowercase().contains(query)
            }
            PickerValue::Session(session_id) => {
                self.label.to_ascii_lowercase().contains(query)
                    || self.detail.to_ascii_lowercase().contains(query)
                    || format!("#{session_id}").contains(query)
            }
            PickerValue::Message(message_id) => {
                self.label.to_ascii_lowercase().contains(query)
                    || self.detail.to_ascii_lowercase().contains(query)
                    || format!("#{message_id}").contains(query)
            }
            _ => {
                self.label.to_ascii_lowercase().contains(query)
                    || self.detail.to_ascii_lowercase().contains(query)
            }
        }
    }
}

impl SearchListItem for SessionModelChoiceItem {
    fn search_list_label(&self) -> String {
        self.label.clone()
    }

    fn search_list_detail(&self) -> Option<String> {
        Some(self.detail.clone())
    }

    fn search_list_fill_value(&self) -> String {
        self.label.clone()
    }

    fn search_list_matches_query(&self, query: &str) -> bool {
        query.trim().is_empty()
            || self
                .search_text
                .contains(query.trim().to_ascii_lowercase().as_str())
    }
}
use crate::app::{
    ChoiceCustomValue, ChoiceItem, ChoiceOverlayMeta, FileAttachOverlayMeta, JsonValue, Modifier,
    PathBrowserItem, PathBrowserOverlayMeta, PickerItem, PickerValue, SessionModelChoiceItem,
    SessionSearchItem, Style, TypedPathValue, commands, format_setting_value_inline,
    session_matches_query, ui_text,
};
use agena_tui_components::{SearchListCustomValue, SearchListItem};
