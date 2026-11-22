//! Stable integration authority, separate from plugin presentation and schemas.
use std::collections::BTreeMap;

use agena_assistant::{Error, INTEGRATION_API_VERSION, INTEGRATION_SERVICE, Result};
use agena_plugin_host::{LoadedPlugin, config::ConfiguredPlugin};
use agena_plugin_sdk::PluginManifest;
use serde_json::{Value, json};

const PREFIX: &str = "integration-provider:v2:";

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

#[cfg(test)]
mod tests {
    use super::*;
    use agena_plugin_host::config::{DurationSpec, HttpAuth, PluginPackage};
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
        assert_eq!(
            fingerprint("static", &manifest, &configured).unwrap(),
            stored
        );
    }

    #[test]
    fn settings_transport_code_and_credentials_changes_still_require_review() {
        let manifest = telegram();
        let configured = configuration();
        let current = fingerprint("static", &manifest, &configured).unwrap();
        let mut changed = configured.clone();
        changed.settings["api_base"] = json!("https://another-endpoint.invalid");
        assert_ne!(fingerprint("static", &manifest, &changed).unwrap(), current);
        changed = configured.clone();
        changed.settings["bot_token_env"] = json!("OTHER_TELEGRAM_TOKEN");
        assert_ne!(fingerprint("static", &manifest, &changed).unwrap(), current);
        changed = configured.clone();
        changed.package_root = Some("/another-package".into());
        assert_ne!(fingerprint("static", &manifest, &changed).unwrap(), current);
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
        assert_ne!(fingerprint("http", &manifest, &changed).unwrap(), http);
        changed.package = PluginPackage::Wasm {
            path: "/plugin.wasm".into(),
            sha256: Some("first".into()),
        };
        let wasm = fingerprint("wasm", &manifest, &changed).unwrap();
        changed.package = PluginPackage::Wasm {
            path: "/plugin.wasm".into(),
            sha256: Some("second".into()),
        };
        assert_ne!(fingerprint("wasm", &manifest, &changed).unwrap(), wasm);
    }

    #[test]
    fn provider_and_protocol_operations_are_part_of_the_fingerprint() {
        let configured = configuration();
        let mut manifest = telegram();
        let stored = fingerprint("static", &manifest, &configured).unwrap();
        manifest.name = "other".into();
        assert_ne!(
            fingerprint("static", &manifest, &configured).unwrap(),
            stored
        );
        manifest = telegram();
        let export = manifest
            .services
            .exports
            .iter_mut()
            .find(|export| export.id == INTEGRATION_SERVICE)
            .unwrap();
        export.methods.push(PluginServiceMethod::bounded_json(
            "new_authority_operation",
            4096,
            4,
        ));
        assert_ne!(
            fingerprint("static", &manifest, &configured).unwrap(),
            stored
        );
        manifest
            .services
            .exports
            .iter_mut()
            .find(|export| export.id == INTEGRATION_SERVICE)
            .unwrap()
            .api_version += 1;
        assert!(fingerprint("static", &manifest, &configured).is_err());
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
