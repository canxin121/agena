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
