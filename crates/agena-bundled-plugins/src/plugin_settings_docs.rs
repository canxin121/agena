//! Presentation copy for the bounded settings trees shared by Web and TUI.

use agena_plugin_host::sdk::PluginManifest;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::LazyLock,
};

static CATALOGS: LazyLock<BTreeMap<String, BTreeMap<String, String>>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../locales/plugin-settings.json"))
        .expect("bundled settings translations must be valid JSON")
});

pub(crate) fn localize_settings_docs(manifest: &mut PluginManifest) {
    let Some(mut contract) = manifest.settings.clone() else {
        return;
    };
    let mut copy = BTreeSet::new();
    contract.map_presentation_text(|text| {
        if !text.trim().is_empty() {
            copy.insert(text.to_owned());
        }
        text.to_owned()
    });
    for (locale, catalog) in CATALOGS.iter() {
        let translation = manifest.translations.entry(locale.clone()).or_default();
        for source in &copy {
            if let Some(text) = catalog.get(source) {
                translation.settings.insert(source.clone(), text.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_settings_title_description_and_option_covers_all_ui_locales() {
        assert_eq!(CATALOGS.len(), 10);
        for (mut manifest, _) in crate::capability_manifest::bundled_plugin_manifests() {
            localize_settings_docs(&mut manifest);
            let Some(base) = manifest.settings.clone() else {
                continue;
            };
            let mut copy = BTreeSet::new();
            base.clone().map_presentation_text(|source| {
                if !source.trim().is_empty() {
                    copy.insert(source.to_owned());
                }
                source.to_owned()
            });
            for locale in CATALOGS.keys() {
                let translation = &manifest.translations[locale].settings;
                for source in &copy {
                    let localized = translation.get(source).unwrap_or_else(|| {
                        panic!(
                            "{} {locale} has no settings translation for {source:?}",
                            manifest.name
                        )
                    });
                    assert!(!localized.trim().is_empty());
                    if source.ends_with('.') {
                        assert_ne!(
                            source, localized,
                            "{} {locale} leaves a settings description in English",
                            manifest.name
                        );
                    }
                }
                let mut localized = manifest.clone();
                localized.localize_settings(locale);
                let mut localized_contract = localized.settings.unwrap();
                if let Ok(default) = base.default_value() {
                    localized_contract.validate_value(&default).unwrap();
                    assert_eq!(localized_contract.default_value().unwrap(), default);
                }
                let mut base_semantics = base.clone();
                base_semantics.map_presentation_text(|_| String::new());
                localized_contract.map_presentation_text(|_| String::new());
                assert_eq!(
                    base_semantics, localized_contract,
                    "localization must preserve {locale} {} settings semantics",
                    manifest.name
                );
            }
        }
    }
}
