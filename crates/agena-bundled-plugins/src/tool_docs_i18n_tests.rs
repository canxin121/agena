use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use agena_plugin_host::sdk::Plugin;

const LOCALES: [&str; 10] = [
    "zh-CN", "zh-TW", "ja-JP", "ko-KR", "fr-FR", "de-DE", "es-ES", "hi-IN", "ar-SA", "pt-BR",
];

#[test]
fn bundled_plugin_and_tool_documentation_are_localized() {
    let mut manifests = vec![
        crate::tool::new_content_plugin().manifest(),
        crate::tool::new_commands_plugin().manifest(),
        crate::tool::new_lsp_plugin().manifest(),
        crate::tool::new_cron_plugin().manifest(),
        crate::tool::new_code_plugin().manifest(),
        crate::tool::new_fs_plugin().manifest(),
        crate::tool::new_settings_plugin().manifest(),
        crate::tool::new_shell_plugin().manifest(),
        crate::tool::new_monitor_plugin().manifest(),
        crate::tool::new_tool_api_plugin().manifest(),
        crate::tool::new_session_plugin().manifest(),
        crate::tool::new_interaction_plugin().manifest(),
        crate::tool::new_terminal_plugin().manifest(),
        crate::tool::new_plan_plugin().manifest(),
        crate::tool::new_tasks_plugin().manifest(),
        crate::tool::new_report_plugin().manifest(),
        crate::tool::new_notebook_plugin().manifest(),
        crate::tool::new_web_plugin().manifest(),
        crate::tool::new_memory_plugin().manifest(),
        crate::tool::new_mcp_plugin(Arc::new(agena_mcp_client::McpConnectionManager::new(
            "translation-test",
            env!("CARGO_PKG_VERSION"),
        )))
        .manifest(),
    ];
    manifests.extend([
        crate::tool::new_chatgpt_plugin().manifest(),
        crate::tool::new_claude_plugin().manifest(),
        crate::tool::new_gemini_plugin().manifest(),
    ]);
    for manifest in &mut manifests {
        // This is the same transform attached to each production static
        // registration, applied to the manifest returned by the plugin type.
        crate::plugin_tool_docs::localize_bundled_plugin_manifest(manifest);
    }

    let mut untranslated = BTreeMap::<String, BTreeSet<String>>::new();
    for manifest in &manifests {
        for locale in LOCALES {
            if let Some(base) = manifest.summary.as_deref()
                && manifest.summary_for_locale(locale) == Some(base)
            {
                untranslated
                    .entry(format!("{} summary", manifest.name))
                    .or_default()
                    .insert(locale.to_owned());
            }
            if let Some(base) = manifest.help.as_deref()
                && manifest.help_for_locale(locale) == Some(base)
            {
                untranslated
                    .entry(format!("{} help", manifest.name))
                    .or_default()
                    .insert(locale.to_owned());
            }
            for tool in &manifest.tools {
                if let Some(base) = tool.docs.summary.as_deref()
                    && tool.docs.summary_for_locale(locale) == Some(base)
                {
                    untranslated
                        .entry(format!("{}.{} summary", manifest.name, tool.name))
                        .or_default()
                        .insert(locale.to_owned());
                }
                if let Some(base) = tool.docs.help.as_deref()
                    && tool.docs.help_for_locale(locale) == Some(base)
                {
                    untranslated
                        .entry(format!("{}.{} help", manifest.name, tool.name))
                        .or_default()
                        .insert(locale.to_owned());
                }
                if let Some(base) = tool.docs.before_help.as_deref()
                    && tool.docs.before_help_for_locale(locale) == Some(base)
                {
                    untranslated
                        .entry(format!("{}.{} before_help", manifest.name, tool.name))
                        .or_default()
                        .insert(locale.to_owned());
                }
                if let Some(base) = tool.docs.after_help.as_deref()
                    && tool.docs.after_help_for_locale(locale) == Some(base)
                {
                    untranslated
                        .entry(format!("{}.{} after_help", manifest.name, tool.name))
                        .or_default()
                        .insert(locale.to_owned());
                }
            }
        }
    }
    let untranslated = untranslated
        .into_iter()
        .map(|(field, locales)| {
            format!(
                "{field}: {}",
                locales.into_iter().collect::<Vec<_>>().join(", ")
            )
        })
        .collect::<Vec<_>>();
    assert!(
        untranslated.is_empty(),
        "bundled plugin documentation is missing native translations:\n{}",
        untranslated.join("\n")
    );
}
