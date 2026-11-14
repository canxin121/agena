//! Package/install regression for plugins with browser resources.

use std::collections::BTreeMap;
use std::fs;

use serde_json::json;

use crate::{
    HttpFetcher, InstallRequest, InstalledRecord, MarketplaceCache, MarketplaceClient,
    MarketplaceError, PackagePluginRequest, RegistryIndex, RegistrySpec, current_target_triple,
    package_plugin,
};

struct FixtureFetcher(BTreeMap<String, Vec<u8>>);

impl HttpFetcher for FixtureFetcher {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, MarketplaceError> {
        self.0
            .get(url)
            .cloned()
            .ok_or_else(|| MarketplaceError::Http(format!("unexpected fixture URL: {url}")))
    }
}

#[test]
fn archived_web_package_keeps_resources_and_explicit_root_through_install_and_uninstall() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("source");
    fs::create_dir_all(source.join("web/dist/chunks")).unwrap();
    fs::write(
        source.join("agena-plugin.toml"),
        r#"schema_version = 1
[plugin]
id = "fixture.web_package"
name = "Web package fixture"
description = "A package with browser assets and a nested executable"
version = "1.0.0"
kind = "stdio"
[release]
entrypoint = "bin/plugin"
include = ["web/dist"]
"#,
    )
    .unwrap();
    fs::write(source.join("plugin"), b"fixture executable").unwrap();
    let entry = b"import './chunks/model.js'; export function mount() {}";
    fs::write(source.join("web/dist/dashboard.js"), entry).unwrap();
    fs::write(
        source.join("web/dist/chunks/model.js"),
        b"export const value = 7;",
    )
    .unwrap();
    let packed = package_plugin(PackagePluginRequest {
        manifest_path: source.join("agena-plugin.toml"),
        artifact_path: source.join("plugin"),
        target: current_target_triple().to_owned(),
        output_dir: temporary.path().join("release"),
    })
    .unwrap();

    let registry_url = "https://fixtures.invalid/web-index.json";
    let artifact_url = "https://fixtures.invalid/web-package.tar.gz";
    let mut index = RegistryIndex::default();
    index.marketplace.name = "Web package fixtures".to_owned();
    index.plugins.push(
        serde_json::from_value(json!({
            "id": "fixture.web_package",
            "name": "Web package fixture",
            "versions": [{
                "version": "1.0.0", "kind": "stdio",
                "platform": current_target_triple(), "url": artifact_url,
                "sha256": packed.sha256,
                "archive": { "format": "tar_gz", "entrypoint": "bin/plugin" }
            }]
        }))
        .unwrap(),
    );
    let fetcher = FixtureFetcher(BTreeMap::from([
        (registry_url.to_owned(), serde_json::to_vec(&index).unwrap()),
        (
            artifact_url.to_owned(),
            fs::read(packed.archive_path).unwrap(),
        ),
    ]));
    let client = MarketplaceClient::from_parts(
        MarketplaceCache::new(temporary.path().join("cache")),
        fetcher,
        BTreeMap::new(),
    );
    let config_path = temporary.path().join("agena.json");
    client
        .install(InstallRequest {
            registry: RegistrySpec {
                id: "fixture-web".to_owned(),
                url: registry_url.to_owned(),
                require_signature: false,
                require_github_distribution: false,
            },
            plugin_id: "fixture.web_package".to_owned(),
            version: None,
            config_path: config_path.clone(),
            force: false,
            dry_run: false,
            allow_unverified: false,
            refresh_index: true,
        })
        .unwrap();

    let root = client.cache().plugin_dir("fixture.web_package", "1.0.0");
    let config: serde_json::Value =
        serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    let installed = &config["plugins"]["list"]["fixture.web_package"];
    assert_eq!(installed["package_root"], root.to_str().unwrap());
    assert_eq!(
        installed["package"]["command"],
        root.join("bin/plugin").to_str().unwrap()
    );
    assert_eq!(fs::read(root.join("web/dist/dashboard.js")).unwrap(), entry);
    assert_eq!(
        fs::read(root.join("web/dist/chunks/model.js")).unwrap(),
        b"export const value = 7;"
    );
    let records = client.list_installed().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].package_root.as_deref(), Some(root.as_path()));

    // Existing installed.json records remain readable after the new metadata.
    let mut legacy = serde_json::to_value(&records[0]).unwrap();
    legacy.as_object_mut().unwrap().remove("package_root");
    assert!(
        serde_json::from_value::<InstalledRecord>(legacy)
            .unwrap()
            .package_root
            .is_none()
    );

    client.uninstall("fixture.web_package").unwrap();
    assert!(!root.exists());
    assert!(client.list_installed().unwrap().is_empty());
    let config: serde_json::Value =
        serde_json::from_slice(&fs::read(config_path).unwrap()).unwrap();
    assert!(
        config["plugins"]["list"]
            .get("fixture.web_package")
            .is_none()
    );
}
