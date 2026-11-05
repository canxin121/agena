//! Read-only Hook inspector: one authored function per row, regardless of how
//! the plugin's subscription bitset and transport callbacks are aggregated.

use std::collections::BTreeMap;

use agena_plugin_sdk::{
    HookHandlerDefinition, host_api::HostHookRegistration, inspection::PluginRunState,
};
use agena_tui::i18n::I18n;

use super::super::{
    Line, Modifier, PluginWorkbenchOverlay, PluginWorkbenchPlugin, Span, Style, Text, clean,
};

pub(crate) fn plugin_hooks_text(
    dialog: &PluginWorkbenchOverlay,
    plugin: &PluginWorkbenchPlugin,
) -> Text<'static> {
    let Some(inspect) = plugin.inspect.as_ref() else {
        return Text::from(dialog.i18n.text("plugin-workbench-hooks-empty"));
    };
    let Some(manifest) = inspect.manifest.as_ref() else {
        return Text::from(dialog.i18n.text("plugin-workbench-hooks-empty"));
    };
    hook_documentation_text(
        &dialog.i18n,
        &manifest.hook_handlers,
        &inspect.hooks,
        plugin.status.state == PluginRunState::Running,
    )
}

fn hook_documentation_text(
    i18n: &I18n,
    handlers: &[HookHandlerDefinition],
    registrations: &[HostHookRegistration],
    running: bool,
) -> Text<'static> {
    if handlers.is_empty() {
        return Text::from(i18n.text("plugin-workbench-hooks-empty"));
    }
    let locale = i18n.locale_tag();
    let mut grouped: BTreeMap<&str, Vec<&HookHandlerDefinition>> = BTreeMap::new();
    for handler in handlers {
        grouped.entry(&handler.event).or_default().push(handler);
    }
    let mut lines = vec![Line::from(i18n.text_args(
        "plugin-workbench-hooks-summary",
        &agena_tui::fl_args![
            "count" => handlers.len(),
            "events" => grouped.len(),
        ],
    ))];
    lines.push(Line::from(""));

    for (event, mut event_handlers) in grouped {
        event_handlers.sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| left.id.cmp(&right.id))
        });
        lines.push(Line::from(Span::styled(
            format!("{event}  ·  {}", event_handlers.len()),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for handler in event_handlers {
            let registered = registrations
                .iter()
                .flat_map(|item| &item.hooks)
                .find(|item| item.name == handler.event && item.handler_id == handler.id);
            let status = match registered {
                None => i18n.text("plugin-workbench-hook-declared"),
                Some(registration) if !registration.enabled => {
                    i18n.text("plugin-workbench-hook-disabled")
                }
                Some(_) if !running => i18n.text("plugin-workbench-hook-inactive"),
                Some(_) => i18n.text("plugin-workbench-hook-active"),
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {}", clean(handler.summary_for_locale(&locale))),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  ·  {status}"),
                    agena_tui_components::theme::muted_style(),
                ),
            ]));
            lines.push(Line::from(Span::styled(
                format!("    {}", clean(handler.help_for_locale(&locale))),
                Style::default(),
            )));
            let mut metadata = vec![handler.id.clone()];
            if handler.priority != 0 {
                metadata.push(format!("P{}", handler.priority));
            }
            metadata.push(if handler.is_async { "async" } else { "sync" }.to_owned());
            for (label, values) in [
                ("tool", &handler.filters.tools),
                ("command", &handler.filters.commands),
                ("plugin", &handler.filters.plugins),
                ("tag", &handler.filters.tags),
            ] {
                if !values.is_empty() {
                    metadata.push(format!("{label}: {}", values.join(", ")));
                }
            }
            lines.push(Line::from(Span::styled(
                format!("    {}", clean(metadata.join("  ·  "))),
                agena_tui_components::theme::muted_style(),
            )));
            lines.push(Line::from(""));
        }
    }
    Text::from(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_plugin_sdk::{
        HookHandlerFilters, HookHandlerTranslation, PluginKey, host_api::HostHookDescriptor,
    };

    fn handler(id: &str, priority: i32) -> HookHandlerDefinition {
        HookHandlerDefinition {
            id: id.to_owned(),
            event: "tool.before".to_owned(),
            summary: format!("Guard {id}"),
            help: format!("Checks {id} before execution."),
            translations: BTreeMap::from([(
                "zh-CN".into(),
                HookHandlerTranslation {
                    summary: format!("检查 {id}"),
                    help: format!("在执行前检查 {id}。"),
                },
            )]),
            priority,
            filters: HookHandlerFilters {
                tools: vec!["bash".into()],
                ..Default::default()
            },
            is_async: true,
        }
    }

    fn registration(id: &str) -> HostHookRegistration {
        HostHookRegistration {
            plugin_id: "test.probe".parse::<PluginKey>().unwrap(),
            trust_level: "verified".into(),
            trust_status: "trusted".into(),
            provenance: vec![],
            source: "static".into(),
            source_path: None,
            manifest_hash: None,
            hooks: vec![HostHookDescriptor {
                name: "tool.before".into(),
                handler_id: id.into(),
                summary: format!("Guard {id}"),
                help: format!("Checks {id} before execution."),
                translations: BTreeMap::new(),
                priority: 100,
                filters: Default::default(),
                is_async: true,
                enabled: true,
                trust_level: "verified".into(),
                trust_status: "trusted".into(),
                source: "static".into(),
                source_path: None,
                current_hash: None,
            }],
        }
    }

    fn as_plain_text(text: Text<'static>) -> String {
        text.lines
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.into_owned())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn two_handlers_under_one_event_have_separate_translated_descriptions_and_states() {
        let i18n = I18n::resolve(Some("zh-CN"), None);
        let doc = as_plain_text(hook_documentation_text(
            &i18n,
            &[handler("protect_files", 80), handler("check_shell", 100)],
            &[registration("check_shell")],
            true,
        ));
        assert!(doc.contains("检查 check_shell"), "{doc}");
        assert!(doc.contains("检查 protect_files"), "{doc}");
        assert!(doc.contains("在执行前检查 check_shell。"), "{doc}");
        assert!(doc.contains("tool: bash"), "{doc}");
        assert!(doc.contains("tool.before  ·  2"), "{doc}");
        assert!(doc.find("check_shell").unwrap() < doc.find("protect_files").unwrap());
        assert!(doc.contains(&i18n.text("plugin-workbench-hook-active")));
        assert!(doc.contains(&i18n.text("plugin-workbench-hook-declared")));
    }

    #[test]
    fn stopped_plugin_never_presents_its_handlers_as_active() {
        let english = I18n::english();
        let doc = as_plain_text(hook_documentation_text(
            &english,
            &[handler("guard", 0)],
            &[registration("guard")],
            false,
        ));
        assert!(doc.contains("Guard guard"));
        assert!(doc.contains(&english.text("plugin-workbench-hook-inactive")));
        assert!(!doc.contains(&english.text("plugin-workbench-hook-active")));
    }

    #[test]
    fn disabled_handlers_are_distinguished_from_stopped_plugins() {
        let english = I18n::english();
        let mut disabled = registration("guard");
        disabled.hooks[0].enabled = false;
        let doc = as_plain_text(hook_documentation_text(
            &english,
            &[handler("guard", 0)],
            &[disabled],
            true,
        ));
        assert!(doc.contains(&english.text("plugin-workbench-hook-disabled")));
        assert!(!doc.contains(&english.text("plugin-workbench-hook-active")));
    }

    #[test]
    fn empty_manifest_has_a_localized_empty_state() {
        let i18n = I18n::resolve(Some("zh-CN"), None);
        let doc = as_plain_text(hook_documentation_text(&i18n, &[], &[], false));
        assert_eq!(doc, i18n.text("plugin-workbench-hooks-empty"));
    }

    #[test]
    fn hooks_tab_is_addressable_and_localized_in_every_supported_language() {
        use crate::PluginDetailTab;
        assert_eq!(
            PluginDetailTab::from_id("hooks"),
            Some(PluginDetailTab::Hooks)
        );
        assert!(PluginDetailTab::ALL.contains(&PluginDetailTab::Hooks));
        for &(locale, _) in agena_tui::i18n::SUPPORTED_LOCALES {
            let i18n = I18n::resolve(Some(locale), None);
            let title = PluginDetailTab::Hooks.label(&i18n);
            assert!(!title.is_empty(), "missing Hooks tab for {locale}");
            assert!(
                i18n.try_text("plugin-workbench-hook-disabled").is_some(),
                "missing disabled Hook status for {locale}"
            );
            assert!(
                !title.starts_with("Unknown localization key"),
                "{locale}: {title}"
            );
            let body = as_plain_text(hook_documentation_text(
                &i18n,
                &[handler("guard", 0)],
                &[],
                false,
            ));
            assert!(
                !body.contains("Unknown localization key"),
                "{locale}: {body}"
            );
        }
    }
}
