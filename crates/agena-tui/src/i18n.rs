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

    /// Look a key up without the loader's `Unknown localization key: "…"`
    /// placeholder, returning `None` when no locale in the fallback chain
    /// carries it. Callers that own their own fallback text (a catalog's
    /// literal summary, a declaration's id) use this instead of comparing
    /// [`I18n::text`] against a sentinel string.
    pub fn try_text(&self, key: &str) -> Option<String> {
        LOCALES.try_lookup(&self.locale, key)
    }

    pub fn text_args(&self, key: &str, args: &FluentArgs) -> String {
        LOCALES.lookup_with_args(&self.locale, key, args)
    }

    pub fn locale_tag(&self) -> String {
        self.locale.to_string()
    }
}

/// The search-picker dialog keeps its own chrome keys; resolving them through
/// [`I18n`] keeps every `search-picker-*` message in the shared catalog.
impl agena_tui_components::SearchPickerText for I18n {
    fn text(&self, key: &str, args: &[(&str, String)]) -> String {
        if args.is_empty() {
            return I18n::text(self, key);
        }
        let mut fluent = FluentArgs::new();
        for (name, value) in args {
            fluent.insert(
                Cow::Owned((*name).to_owned()),
                FluentValue::from(value.clone()),
            );
        }
        I18n::text_args(self, key, &fluent)
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
