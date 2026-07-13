use std::borrow::Cow;

use crate::app::{
    ChoiceCustomValue, ChoiceItem, ChoiceOverlayMeta, FileAttachOverlayMeta, JsonValue, Modifier,
    PathBrowserItem, PathBrowserOverlayMeta, PickerItem, PickerValue, SessionModelChoiceItem,
    SessionSearchItem, Style, TimelineItem, TypedPathValue, format_setting_value_inline, ui_text,
};
use agena_tui_components::{SearchPickerCustomValue, SearchPickerItem};

impl SearchPickerItem for ChoiceItem {
    fn search_picker_key(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.value)
    }

    fn search_picker_label(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.label)
    }

    fn search_picker_detail(&self) -> Option<Cow<'_, str>> {
        (!self.detail.trim().is_empty()).then_some(Cow::Borrowed(self.detail.as_str()))
    }

    fn search_picker_fill_value(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.value)
    }

    fn search_picker_search_text(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.search_text)
    }

    fn search_picker_prefix(&self) -> Option<Cow<'_, str>> {
        self.current.then_some(Cow::Borrowed("✓ "))
    }

    fn search_picker_prefix_style(&self) -> Style {
        Style::default().fg(agena_tui_components::theme::accent_color())
    }
}

impl SearchPickerCustomValue<ChoiceOverlayMeta> for ChoiceCustomValue {
    fn search_picker_from_input(input: &str, _: &ChoiceOverlayMeta) -> Option<Self> {
        let raw = input.trim().to_string();
        (!raw.is_empty()).then_some(Self { raw })
    }

    fn search_picker_label(&self, meta: &ChoiceOverlayMeta) -> Cow<'_, str> {
        Cow::Owned(ui_text::t(&meta.i18n, "search-picker-custom-value-label"))
    }

    fn search_picker_detail(&self, meta: &ChoiceOverlayMeta) -> Option<Cow<'_, str>> {
        Some(Cow::Owned(meta.i18n.text_args(
            "search-picker-custom-value-detail",
            &crate::fl_args!(
                "value" => format_setting_value_inline(&JsonValue::String(self.raw.clone()))
            ),
        )))
    }

    fn search_picker_input_text(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.raw)
    }
}

impl SearchPickerCustomValue<FileAttachOverlayMeta> for TypedPathValue {
    fn search_picker_from_input(input: &str, _: &FileAttachOverlayMeta) -> Option<Self> {
        let raw = input.trim().to_string();
        (!raw.is_empty()).then_some(Self { raw })
    }

    fn search_picker_label(&self, meta: &FileAttachOverlayMeta) -> Cow<'_, str> {
        Cow::Owned(ui_text::t(&meta.i18n, "search-picker-custom-path-label"))
    }

    fn search_picker_detail(&self, _: &FileAttachOverlayMeta) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(&self.raw))
    }

    fn search_picker_input_text(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.raw)
    }
}

impl SearchPickerCustomValue<PathBrowserOverlayMeta> for TypedPathValue {
    fn search_picker_from_input(input: &str, _: &PathBrowserOverlayMeta) -> Option<Self> {
        let raw = input.trim().to_string();
        (!raw.is_empty()).then_some(Self { raw })
    }

    fn search_picker_label(&self, meta: &PathBrowserOverlayMeta) -> Cow<'_, str> {
        Cow::Owned(ui_text::t(&meta.i18n, "search-picker-custom-path-label"))
    }

    fn search_picker_detail(&self, _: &PathBrowserOverlayMeta) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(&self.raw))
    }

    fn search_picker_input_text(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.raw)
    }
}

impl SearchPickerItem for PathBrowserItem {
    fn search_picker_key(&self) -> Cow<'_, str> {
        Cow::Owned(self.path.display().to_string())
    }

    fn search_picker_label(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.label)
    }

    fn search_picker_detail(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(&self.detail))
    }

    fn search_picker_fill_value(&self) -> Cow<'_, str> {
        Cow::Owned(self.path.display().to_string())
    }

    fn search_picker_label_style(&self) -> Style {
        if self.is_dir {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        }
    }
}

impl SearchPickerItem for SessionSearchItem {
    fn search_picker_key(&self) -> Cow<'_, str> {
        Cow::Owned(self.session.id.to_string())
    }

    fn search_picker_label(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.label)
    }

    fn search_picker_detail(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(&self.detail))
    }

    fn search_picker_fill_value(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.session.title)
    }
}

impl SearchPickerItem for PickerItem {
    fn search_picker_key(&self) -> Cow<'_, str> {
        match &self.value {
            PickerValue::Command(spec) => Cow::Owned(format!("command:{}", spec.name)),
            PickerValue::PluginCommand(entry) => Cow::Owned(format!(
                "plugin-command:{}:{}",
                entry.plugin_id, entry.command.id
            )),
            PickerValue::ProviderCreate => Cow::Borrowed("action:create-provider"),
            PickerValue::Provider(provider) => {
                Cow::Owned(format!("provider:{}", provider.provider_id))
            }
            PickerValue::AgentCreate => Cow::Borrowed("action:create-agent"),
            PickerValue::Agent(agent) => Cow::Owned(format!("agent:{}", agent.name)),
            PickerValue::Session(id) => Cow::Owned(format!("session:{id}")),
            PickerValue::Message(message) => Cow::Owned(format!("message:{}", message.id)),
            PickerValue::PermissionRuleCreate => Cow::Borrowed("action:create-permission-rule"),
            PickerValue::PermissionRule(rule) => Cow::Owned(format!("permission-rule:{}", rule.id)),
            PickerValue::Inspector => Cow::Borrowed("action:inspector"),
        }
    }

    fn search_picker_label(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.label)
    }

    fn search_picker_detail(&self) -> Option<Cow<'_, str>> {
        (!self.detail.trim().is_empty()).then_some(Cow::Borrowed(self.detail.as_str()))
    }

    fn search_picker_search_text(&self) -> Cow<'_, str> {
        let aliases = match &self.value {
            PickerValue::Command(spec) => spec.aliases.join(" "),
            PickerValue::Session(id) => format!("#{id}"),
            PickerValue::Message(message) => format!("#{}", message.id),
            _ => String::new(),
        };
        Cow::Owned(format!("{} {} {aliases}", self.label, self.detail))
    }

    fn search_picker_always_visible(&self) -> bool {
        matches!(
            &self.value,
            PickerValue::ProviderCreate
                | PickerValue::AgentCreate
                | PickerValue::PermissionRuleCreate
        )
    }
}

impl SearchPickerItem for SessionModelChoiceItem {
    fn search_picker_key(&self) -> Cow<'_, str> {
        Cow::Owned(format!(
            "{}/{}/{}",
            self.model.provider_id,
            self.model
                .adapter_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            self.model.model_id
        ))
    }

    fn search_picker_label(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.label)
    }

    fn search_picker_detail(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(&self.detail))
    }

    fn search_picker_search_text(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.search_text)
    }
}

impl SearchPickerItem for TimelineItem {
    fn search_picker_key(&self) -> Cow<'_, str> {
        self.linked_message_id
            .map(|id| Cow::Owned(format!("message:{id}")))
            .unwrap_or_else(|| Cow::Owned(format!("event:{}", self.summary)))
    }

    fn search_picker_label(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.summary)
    }

    fn search_picker_detail(&self) -> Option<Cow<'_, str>> {
        None
    }

    fn search_picker_search_text(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.search_text)
    }
}
