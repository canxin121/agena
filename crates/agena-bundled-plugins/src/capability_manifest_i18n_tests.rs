//! Bundled documentation copy must cover every locale the UIs ship.

use super::bundled_plugins;

/// Locales every bundled surface ships copy for. Adding a locale to the Web or
/// terminal catalogs means adding it here and to the manifests below.
const UI_LOCALES: [&str; 10] = [
    "ar-SA", "de-DE", "es-ES", "fr-FR", "hi-IN", "ja-JP", "ko-KR", "pt-BR", "zh-CN", "zh-TW",
];

fn non_empty(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty())
}

/// Plugin and tool introductions are user-visible copy: settings, the tool
/// list, and part headers show them. English-only manifests silently fall back
/// to English in every other locale, so keep the manifests complete instead.
#[test]
fn bundled_plugin_and_tool_copy_covers_every_ui_locale() {
    for (plugin, _) in bundled_plugins() {
        let manifest = plugin.manifest();
        let id = format!("{}.{}", manifest.namespace, manifest.name);
        if non_empty(manifest.summary.as_deref()) {
            for locale in UI_LOCALES {
                let translation = manifest
                    .translations
                    .get(locale)
                    .unwrap_or_else(|| panic!("{id} has no `{locale}` plugin translation"));
                assert!(
                    non_empty(translation.summary.as_deref()),
                    "{id} has no `{locale}` plugin summary"
                );
            }
        }
        for tool in &manifest.tools {
            if !non_empty(tool.docs.summary.as_deref()) {
                continue;
            }
            for locale in UI_LOCALES {
                assert!(
                    non_empty(tool.docs.summary_for_locale(locale)),
                    "{id} tool `{}` has no `{locale}` summary",
                    tool.name
                );
            }
        }
    }
}
