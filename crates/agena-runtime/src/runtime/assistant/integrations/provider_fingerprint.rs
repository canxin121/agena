//! Stable integration authority, separate from plugin presentation and schemas.
//!
//! The legacy Telegram manifest is an exact host-localized export from commit
//! 885d63f244193ca6ebe1fd39e5fea94c426ff887. It is used only with the CURRENT
//! configuration and the same static provider/protocol, proving that a legacy
//! grant did not change transport, credentials configuration or settings.
use std::{collections::BTreeMap, sync::LazyLock};

use agena_assistant::{Error, INTEGRATION_API_VERSION, INTEGRATION_SERVICE, Result};
use agena_plugin_host::{
    LoadedPlugin,
    config::{ConfiguredPlugin, PluginPackage},
};
use agena_plugin_sdk::PluginManifest;
use serde_json::{Value, json};

const PREFIX: &str = "integration-provider:v2:";
static LEGACY_TELEGRAM: LazyLock<std::result::Result<Value, String>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("legacy/telegram-885d63f24.json"))
        .map_err(|error| error.to_string())
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderFingerprintMatch {
    Current,
    CompatibleLegacy,
    Mismatch,
}

impl ProviderFingerprintMatch {
    pub(crate) fn is_compatible(self) -> bool {
        self != Self::Mismatch
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderIdentityCheck {
    pub(crate) fingerprint: String,
    pub(crate) kind: ProviderFingerprintMatch,
}

fn protocol(manifest: &PluginManifest) -> Result<Value> {
    let export = manifest
        .services
        .exports
        .iter()
        .find(|export| {
            export.id == INTEGRATION_SERVICE && export.api_version == INTEGRATION_API_VERSION
        })
        .ok_or(Error::Unavailable(
            "integration provider does not export the supported integration API",
        ))?;
    let mut methods: Vec<_> = export
        .methods
        .iter()
        .map(|method| method.id.as_str())
        .collect();
    methods.sort_unstable();
    if !methods.contains(&"describe") || !methods.contains(&"poll") {
        return Err(Error::Unavailable(
            "integration provider does not export describe and poll",
        ));
    }
    Ok(json!({"service": export.id, "api_version": export.api_version, "methods": methods}))
}

fn canonical(value: Value) -> Value {
    match value {
        Value::Object(object) => {
            let ordered: BTreeMap<_, _> = object
                .into_iter()
                .map(|(key, value)| (key, canonical(value)))
                .collect();
            Value::Object(ordered.into_iter().collect())
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonical).collect()),
        value => value,
    }
}

fn fingerprint(
    kind: &str,
    manifest: &PluginManifest,
    configured: &ConfiguredPlugin,
) -> Result<String> {
    let boundary = canonical(json!({
        "version": 2,
        "provider": {"namespace": manifest.namespace, "name": manifest.name},
        "transport": kind,
        "package": configured.package,
        "package_root": configured.package_root,
        "settings": configured.settings,
        "integration": protocol(manifest)?,
    }));
    Ok(format!("{PREFIX}{}", super::digest(&boundary)?))
}

pub(super) fn identity(plugin: &LoadedPlugin) -> Result<String> {
    fingerprint(plugin.kind, &plugin.manifest, &plugin.configured_plugin)
}

pub(super) fn check(plugin: &LoadedPlugin, stored: &str) -> Result<ProviderIdentityCheck> {
    check_parts(
        plugin.kind,
        &plugin.manifest,
        &plugin.configured_plugin,
        stored,
    )
}

fn check_parts(
    kind: &str,
    manifest: &PluginManifest,
    configured: &ConfiguredPlugin,
    stored: &str,
) -> Result<ProviderIdentityCheck> {
    let fingerprint = fingerprint(kind, manifest, configured)?;
    let result = |kind| ProviderIdentityCheck {
        fingerprint: fingerprint.clone(),
        kind,
    };
    if stored == fingerprint {
        return Ok(result(ProviderFingerprintMatch::Current));
    }
    if stored.len() != 64 || !stored.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(result(ProviderFingerprintMatch::Mismatch));
    }
    // Preserve exact legacy serialization. This path proves equality to the
    // active plugin rather than treating an arbitrary old digest as trusted.
    if stored == super::digest(&json!({"manifest":manifest,"configuration":configured}))? {
        return Ok(result(ProviderFingerprintMatch::CompatibleLegacy));
    }
    if kind == "static"
        && matches!(configured.package, PluginPackage::Static {})
        && manifest.namespace == "agena"
        && manifest.name == "telegram"
    {
        let legacy = LEGACY_TELEGRAM.as_ref().map_err(|error| {
            Error::Storage(format!("trusted integration manifest is invalid: {error}"))
        })?;
        let legacy_manifest: PluginManifest =
            serde_json::from_value(legacy.clone()).map_err(|error| {
                Error::Storage(format!("trusted integration contract is invalid: {error}"))
            })?;
        // The supported integration API and its operation set must match the
        // trusted manifest even when the plugin key remains unchanged.
        if protocol(manifest)? == protocol(&legacy_manifest)?
            && stored == super::digest(&json!({"manifest":legacy,"configuration":configured}))?
        {
            return Ok(result(ProviderFingerprintMatch::CompatibleLegacy));
        }
    }
    Ok(result(ProviderFingerprintMatch::Mismatch))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_plugin_host::config::{DurationSpec, HttpAuth};
    use agena_plugin_sdk::{Plugin, PluginServiceMethod};

    fn telegram() -> PluginManifest {
        agena_bundled_plugins::tool::new_telegram_plugin().manifest()
    }

    fn configuration() -> ConfiguredPlugin {
        ConfiguredPlugin::static_settings(json!({
            "account": "test_bot",
            "bot_token_env": "TEST_TELEGRAM_TOKEN",
            "api_base": "https://api.telegram.org",
            "long_poll_seconds": 0,
            "allow_loopback_http": false
        }))
    }

    fn legacy_fingerprint(configured: &ConfiguredPlugin) -> String {
        super::super::digest(&json!({
            "manifest": LEGACY_TELEGRAM.as_ref().unwrap(),
            "configuration": configured
        }))
        .unwrap()
    }

    #[test]
    fn presentation_and_wire_schema_updates_do_not_invalidate_current_authority() {
        let mut manifest = telegram();
        let mut configured = configuration();
        let stored = fingerprint("static", &manifest, &configured).unwrap();
        manifest.version = "999.0.0".into();
        manifest.summary = Some("Updated translated summary".into());
        manifest.help = Some("Updated setup instructions".into());
        manifest.commands.clear();
        manifest.tools.clear();
        manifest.translations.clear();
        for export in &mut manifest.services.exports {
            for method in &mut export.methods {
                *method = PluginServiceMethod::bounded_json(&method.id, 65536, 12);
            }
            export.methods.reverse();
        }
        configured.timeouts.init = Some(DurationSpec(std::time::Duration::from_millis(12345)));
        configured
            .activation
            .after
            .push(agena_plugin_sdk::PluginKey::new("example", "optional").unwrap());
        let check = check_parts("static", &manifest, &configured, &stored).unwrap();
        assert_eq!(check.kind, ProviderFingerprintMatch::Current);
        assert_eq!(check.fingerprint, stored);
    }

    #[test]
    fn current_legacy_and_trusted_previous_telegram_manifest_are_compatible() {
        let manifest = telegram();
        let configured = configuration();
        let legacy_current =
            super::super::digest(&json!({"manifest":manifest,"configuration":configured})).unwrap();
        for stored in [legacy_current, legacy_fingerprint(&configured)] {
            let check = check_parts("static", &manifest, &configured, &stored).unwrap();
            assert_eq!(check.kind, ProviderFingerprintMatch::CompatibleLegacy);
            assert!(check.fingerprint.starts_with(PREFIX));
        }
        let stored = fingerprint("static", &manifest, &configured).unwrap();
        assert_eq!(
            check_parts("static", &manifest, &configured, &stored)
                .unwrap()
                .kind,
            ProviderFingerprintMatch::Current
        );
    }

    #[test]
    fn settings_transport_code_and_credentials_changes_still_require_review() {
        let manifest = telegram();
        let configured = configuration();
        let current = fingerprint("static", &manifest, &configured).unwrap();
        let legacy = legacy_fingerprint(&configured);
        let mut changed = configured.clone();
        changed.settings["api_base"] = json!("https://another-endpoint.invalid");
        for stored in [&current, &legacy] {
            assert_eq!(
                check_parts("static", &manifest, &changed, stored)
                    .unwrap()
                    .kind,
                ProviderFingerprintMatch::Mismatch
            );
        }
        changed = configured.clone();
        changed.settings["bot_token_env"] = json!("OTHER_TELEGRAM_TOKEN");
        assert_eq!(
            check_parts("static", &manifest, &changed, &legacy)
                .unwrap()
                .kind,
            ProviderFingerprintMatch::Mismatch
        );
        changed = configured.clone();
        changed.package_root = Some("/another-package".into());
        assert_eq!(
            check_parts("static", &manifest, &changed, &current)
                .unwrap()
                .kind,
            ProviderFingerprintMatch::Mismatch
        );
        changed.package = PluginPackage::Http {
            url: "https://plugin.invalid".parse().unwrap(),
            auth: HttpAuth::Bearer {
                token: Some("fixture-token-a".into()),
                token_env: None,
            },
        };
        let http = fingerprint("http", &manifest, &changed).unwrap();
        let PluginPackage::Http { auth, .. } = &mut changed.package else {
            unreachable!()
        };
        *auth = HttpAuth::Bearer {
            token: Some("fixture-token-b".into()),
            token_env: None,
        };
        assert_eq!(
            check_parts("http", &manifest, &changed, &http)
                .unwrap()
                .kind,
            ProviderFingerprintMatch::Mismatch
        );
        assert_eq!(
            check_parts("http", &manifest, &changed, &legacy)
                .unwrap()
                .kind,
            ProviderFingerprintMatch::Mismatch
        );
        changed.package = PluginPackage::Wasm {
            path: "/plugin.wasm".into(),
            sha256: Some("first".into()),
        };
        let wasm = fingerprint("wasm", &manifest, &changed).unwrap();
        changed.package = PluginPackage::Wasm {
            path: "/plugin.wasm".into(),
            sha256: Some("second".into()),
        };
        assert_eq!(
            check_parts("wasm", &manifest, &changed, &wasm)
                .unwrap()
                .kind,
            ProviderFingerprintMatch::Mismatch
        );
    }

    #[test]
    fn legacy_compatibility_is_scoped_to_the_exact_static_provider_and_protocol() {
        let configured = configuration();
        let stored = legacy_fingerprint(&configured);
        let mut manifest = telegram();
        manifest.name = "other".into();
        assert_eq!(
            check_parts("static", &manifest, &configured, &stored)
                .unwrap()
                .kind,
            ProviderFingerprintMatch::Mismatch
        );
        manifest = telegram();
        manifest
            .services
            .exports
            .iter_mut()
            .find(|export| export.id == INTEGRATION_SERVICE)
            .unwrap()
            .methods
            .push(PluginServiceMethod::bounded_json(
                "new_authority_operation",
                4096,
                4,
            ));
        assert_eq!(
            check_parts("static", &manifest, &configured, &stored)
                .unwrap()
                .kind,
            ProviderFingerprintMatch::Mismatch
        );
        let mut manifest = telegram();
        manifest
            .services
            .exports
            .iter_mut()
            .find(|export| export.id == INTEGRATION_SERVICE)
            .unwrap()
            .api_version += 1;
        assert!(check_parts("static", &manifest, &configured, &stored).is_err());
        assert_eq!(
            check_parts("static", &telegram(), &configured, &"0".repeat(64))
                .unwrap()
                .kind,
            ProviderFingerprintMatch::Mismatch
        );
    }

    #[test]
    fn configuration_object_key_order_is_not_an_authority_change() {
        let manifest = telegram();
        let mut first = configuration();
        first.settings = serde_json::from_str(r#"{"nested":{"b":2,"a":1},"array":[2,1]}"#).unwrap();
        let mut second = first.clone();
        second.settings =
            serde_json::from_str(r#"{"array":[2,1],"nested":{"a":1,"b":2}}"#).unwrap();
        assert_eq!(
            fingerprint("static", &manifest, &first).unwrap(),
            fingerprint("static", &manifest, &second).unwrap()
        );
        second.settings["array"] = json!([1, 2]);
        assert_ne!(
            fingerprint("static", &manifest, &first).unwrap(),
            fingerprint("static", &manifest, &second).unwrap()
        );
    }
}
