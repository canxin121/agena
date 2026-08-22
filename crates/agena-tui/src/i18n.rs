//! Internationalization strings and helpers.

use std::borrow::Cow;
use std::collections::HashMap;

use fluent_templates::{Loader, static_loader};
use sys_locale::get_locale;
use unic_langid::{LanguageIdentifier, langid};

pub use fluent_templates::fluent_bundle::FluentValue;

static_loader! {
    static LOCALES = {
        locales: "./locales",
        fallback_language: "en-US",
    };
}

const FALLBACK_LOCALE: LanguageIdentifier = langid!("en-US");

pub type FluentArgs = HashMap<Cow<'static, str>, FluentValue<'static>>;

#[derive(Debug, Clone)]
/// Internationalization helper.
pub struct I18n {
    locale: LanguageIdentifier,
}

impl I18n {
    pub fn resolve(cli_locale: Option<&str>, config_locale: Option<&str>) -> Self {
        let locale = cli_locale
            .and_then(parse_supported_locale)
            .or_else(|| config_locale.and_then(parse_supported_locale))
            .or_else(|| get_locale().as_deref().and_then(parse_supported_locale))
            .unwrap_or_else(|| FALLBACK_LOCALE.clone());

        Self { locale }
    }

    pub fn english() -> Self {
        Self {
            locale: FALLBACK_LOCALE.clone(),
        }
    }

    pub fn text(&self, key: &str) -> String {
        LOCALES.lookup(&self.locale, key)
    }

    pub fn text_args(&self, key: &str, args: &FluentArgs) -> String {
        LOCALES.lookup_with_args(&self.locale, key, args)
    }

    pub fn locale_tag(&self) -> String {
        self.locale.to_string()
    }
}

pub const SUPPORTED_LOCALES: &[(&str, &str)] = &[
    ("en-US", "English (United States)"),
    ("zh-CN", "简体中文"),
    ("zh-TW", "繁體中文"),
    ("ja-JP", "日本語"),
    ("ko-KR", "한국어"),
    ("fr-FR", "Français"),
    ("de-DE", "Deutsch"),
    ("es-ES", "Español"),
];

impl Default for I18n {
    fn default() -> Self {
        Self::resolve(None, None)
    }
}

fn parse_supported_locale(raw: &str) -> Option<LanguageIdentifier> {
    let normalized = normalize_locale_tag(raw);
    let parsed = normalized.parse::<LanguageIdentifier>().ok()?;
    if is_supported_locale(&parsed) {
        return Some(parsed);
    }

    match parsed.language.as_str() {
        "en" => Some(langid!("en-US")),
        "zh" => {
            let region = parsed.region.map(|value| value.to_string());
            let script = parsed.script.map(|value| value.to_string());
            match (script.as_deref(), region.as_deref()) {
                (Some("Hant"), _) => Some(langid!("zh-TW")),
                (_, Some("TW" | "HK" | "MO")) => Some(langid!("zh-TW")),
                _ => Some(langid!("zh-CN")),
            }
        }
        "ja" => Some(langid!("ja-JP")),
        "ko" => Some(langid!("ko-KR")),
        "fr" => Some(langid!("fr-FR")),
        "de" => Some(langid!("de-DE")),
        "es" => Some(langid!("es-ES")),
        _ => None,
    }
}

fn normalize_locale_tag(raw: &str) -> String {
    let trimmed = raw.trim();
    let without_encoding = trimmed.split('.').next().unwrap_or(trimmed);
    let without_modifier = without_encoding
        .split('@')
        .next()
        .unwrap_or(without_encoding);
    without_modifier.replace('_', "-")
}

fn is_supported_locale(locale: &LanguageIdentifier) -> bool {
    matches!(
        locale,
        value if *value == langid!("en-US")
            || *value == langid!("zh-CN")
            || *value == langid!("zh-TW")
            || *value == langid!("ja-JP")
            || *value == langid!("ko-KR")
            || *value == langid!("fr-FR")
            || *value == langid!("de-DE")
            || *value == langid!("es-ES")
    )
}

#[macro_export]
macro_rules! fl_args {
    ($($key:literal => $value:expr),* $(,)?) => {{
        let mut args = $crate::i18n::FluentArgs::new();
        $(
            args.insert(
                std::borrow::Cow::Borrowed($key),
                $crate::i18n::FluentValue::from(($value).to_string()),
            );
        )*
        args
    }};
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{I18n, SUPPORTED_LOCALES};

    const RESOURCES: &[(&str, &str)] = &[
        ("en-US", include_str!("../locales/en-US/main.ftl")),
        ("zh-CN", include_str!("../locales/zh-CN/main.ftl")),
        ("zh-TW", include_str!("../locales/zh-TW/main.ftl")),
        ("ja-JP", include_str!("../locales/ja-JP/main.ftl")),
        ("ko-KR", include_str!("../locales/ko-KR/main.ftl")),
        ("fr-FR", include_str!("../locales/fr-FR/main.ftl")),
        ("de-DE", include_str!("../locales/de-DE/main.ftl")),
        ("es-ES", include_str!("../locales/es-ES/main.ftl")),
    ];

    const SETTINGS_CORE_KEYS: &[&str] = &[
        "overlay-settings-title",
        "overlay-settings-section-plugins-label",
        "overlay-settings-section-providers-label",
        "overlay-settings-section-model-catalog-label",
        "overlay-settings-section-permissions-label",
        "overlay-settings-section-ui-label",
        "overlay-settings-section-runtime-session-label",
        "settings-field-permission-approval-model-label",
        "settings-field-ui-locale-label",
        "settings-field-tui-color-scheme-label",
        "settings-field-tui-theme-label",
        "settings-field-tui-graphics-label",
        "settings-field-tracing-filter-label",
        "settings-field-tracing-database-label",
        "settings-field-tracing-adapter-label",
        "settings-plugin-workbench-label",
        "settings-mcp-server-label",
        "settings-mcp-auth-label",
        "settings-mcp-anonymous-access-label",
        "settings-mcp-client-registration-label",
        "settings-mcp-public-url-label",
        "settings-mcp-oauth-issuer-label",
        "settings-mcp-oauth-password-label",
        "settings-field-session-compaction-auto-label",
        "settings-field-session-compaction-reserved-tokens-label",
        "settings-client-versions-section-label",
        "permission-studio-nav-filesystem",
        "permission-studio-nav-default-zones",
        "permission-studio-nav-path-rules",
        "permission-studio-nav-network",
        "permission-studio-nav-network-zones",
        "permission-studio-nav-domain-rules",
        "permission-studio-nav-tool-access",
        "permission-studio-nav-name-rules",
        "permission-studio-nav-command-rules",
        "overlay-provider-studio-providers",
        "overlay-provider-studio-adapters",
        "overlay-provider-studio-models",
        "settings-mcp-public-url-updated",
        "settings-mcp-server-enabled-flash",
        "permission-studio-command-pattern-title",
        "permission-studio-rename-unsupported",
        "settings-tool-api-list-description",
        "settings-tool-api-call-description",
    ];

    fn message_keys(resource: &str) -> BTreeSet<&str> {
        resource
            .lines()
            .filter_map(|line| {
                let (key, _) = line.split_once('=')?;
                let key = key.trim();
                (!key.is_empty()
                    && key
                        .bytes()
                        .next()
                        .is_some_and(|byte| byte.is_ascii_alphabetic()))
                .then_some(key)
            })
            .collect()
    }

    fn message_values(resource: &str) -> std::collections::BTreeMap<&str, &str> {
        resource
            .lines()
            .filter_map(|line| {
                let (key, value) = line.split_once('=')?;
                let key = key.trim();
                (!key.is_empty()
                    && key
                        .bytes()
                        .next()
                        .is_some_and(|byte| byte.is_ascii_alphabetic()))
                .then_some((key, value.trim()))
            })
            .collect()
    }

    fn settings_keys(resource: &str) -> BTreeSet<&str> {
        message_keys(resource)
            .into_iter()
            .filter(|key| {
                key.contains("settings")
                    || key.starts_with("overlay-provider")
                    || key.starts_with("permission-studio")
                    || key.starts_with("plugin-workbench")
                    || key.starts_with("model-catalog")
                    || key.starts_with("provider-studio")
            })
            .collect()
    }

    #[test]
    fn every_supported_locale_has_the_settings_core_catalog() {
        let supported = SUPPORTED_LOCALES
            .iter()
            .map(|(locale, _)| *locale)
            .collect::<BTreeSet<_>>();
        let available = RESOURCES
            .iter()
            .map(|(locale, _)| *locale)
            .collect::<BTreeSet<_>>();
        assert_eq!(available, supported);

        for (locale, resource) in RESOURCES {
            let keys = message_keys(resource);
            for key in SETTINGS_CORE_KEYS {
                assert!(
                    keys.contains(key),
                    "{locale} is missing core Settings translation `{key}`"
                );
            }
            assert!(
                !resource.contains("= §"),
                "{locale} contains a damaged translation"
            );
            assert!(
                !resource.contains("<ph") && !resource.contains("< ph"),
                "{locale} contains an unrestored translation placeholder"
            );
        }
    }

    #[test]
    fn every_supported_locale_covers_the_full_settings_catalog() {
        let english = settings_keys(RESOURCES[0].1);
        for (locale, resource) in RESOURCES.iter().skip(1) {
            let localized = settings_keys(resource);
            let missing = english.difference(&localized).copied().collect::<Vec<_>>();
            assert!(
                missing.is_empty(),
                "{locale} misses Settings keys: {missing:?}"
            );
        }
    }

    #[test]
    fn non_english_locales_translate_the_primary_settings_navigation() {
        let english = message_values(RESOURCES[0].1);
        let primary_keys = [
            "overlay-settings-title",
            "overlay-settings-section-plugins-label",
            "overlay-settings-section-providers-label",
            "overlay-settings-section-permissions-label",
            "overlay-settings-section-ui-label",
            "overlay-settings-section-runtime-session-label",
            "settings-field-permission-approval-model-label",
            "settings-field-ui-locale-label",
            "settings-field-tui-color-scheme-label",
            "settings-plugin-workbench-label",
            "permission-studio-nav-filesystem",
            "permission-studio-nav-network",
            "permission-studio-nav-tool-access",
            "overlay-provider-studio-providers",
            "overlay-provider-studio-adapters",
            "overlay-provider-studio-models",
        ];

        for (locale, resource) in RESOURCES.iter().skip(1) {
            let localized = message_values(resource);
            for key in primary_keys {
                assert_ne!(
                    localized.get(key),
                    english.get(key),
                    "{locale} leaves primary Settings label `{key}` in English"
                );
            }
        }
    }

    #[test]
    fn core_settings_labels_resolve_without_falling_back_to_message_ids() {
        for (locale, _) in RESOURCES {
            let i18n = I18n::resolve(Some(locale), None);
            for key in SETTINGS_CORE_KEYS {
                let value = i18n.text(key);
                assert_ne!(value, *key, "{locale} returned the message id for `{key}`");
                assert!(
                    !value.trim().is_empty(),
                    "{locale} returned an empty value for `{key}`"
                );
            }
        }
    }
}
