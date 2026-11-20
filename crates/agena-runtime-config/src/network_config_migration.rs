//! Safe upgrades of historical complete network policies to field overlays.
//!
//! Legacy layers are resolved before replacement, so partial old overrides keep
//! their effective allow and deny lists. Ordinary saves only accept the flat form.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use agena_domain::NetworkPermissionConfig;
use serde_json::Value;

use crate::ConfigError;

pub struct NetworkConfigMigration {
    pub global: NetworkPermissionConfig,
    pub effective: NetworkPermissionConfig,
    pub migrated_files: Vec<PathBuf>,
}

pub fn is_legacy_network(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|value| value.contains_key("egress") || value.contains_key("ingress"))
}

/// Project old flat blocks as version-2 fields without writing the file. Invalid
/// policies remain available for repair through the raw settings editor.
pub fn normalize_network_settings_document(document: &mut Value) {
    fn permission(value: &mut Value) {
        let Some(network) = value.get_mut("network") else {
            return;
        };
        if !network.is_object() || is_legacy_network(network) || network.get("version").is_some() {
            return;
        }
        if let Ok(parsed) = serde_json::from_value::<NetworkPermissionConfig>(network.clone()) {
            if let Ok(canonical) = serde_json::to_value(parsed) {
                *network = canonical;
            }
        }
    }
    if let Some(value) = document.get_mut("permission") {
        permission(value);
    }
    if let Some(profiles) = document.get_mut("profiles").and_then(Value::as_object_mut) {
        for profile in profiles.values_mut() {
            if let Some(value) = profile.get_mut("permission") {
                permission(value);
            }
        }
    }
    if let Some(access) = document.get_mut("access_control") {
        if let Some(profiles) = access.get_mut("profiles").and_then(Value::as_object_mut) {
            for profile in profiles.values_mut() {
                if let Some(value) = profile.get_mut("permission") {
                    permission(value);
                }
            }
        }
        if let Some(users) = access.get_mut("users").and_then(Value::as_array_mut) {
            for user in users {
                if let Some(value) = user.get_mut("overrides") {
                    permission(value);
                }
            }
        }
    }
}

#[derive(Clone)]
struct LayerNetwork {
    historical: NetworkPermissionConfig,
    current: NetworkPermissionConfig,
}

/// Leaf PUT/PATCH creates a v2 overlay when no network block exists yet.
pub(crate) fn initialize_network_field_object(value: &mut Value, segments: &[String]) {
    let path: Vec<_> = segments.iter().map(String::as_str).collect();
    let network = matches!(
        path.as_slice(),
        ["permission", "network"]
            | ["profiles", _, "permission", "network"]
            | ["access_control", "profiles", _, "permission", "network"]
            | ["access_control", "users", _, "overrides", "network"]
    );
    if network {
        if let Some(object) = value.as_object_mut().filter(|object| object.is_empty()) {
            object.insert("version".to_owned(), Value::from(2));
        }
    }
}

struct PendingFile {
    path: PathBuf,
    original: Vec<u8>,
    replacement: Vec<u8>,
}

fn read_layer(
    path: &Path,
    inherited: &LayerNetwork,
    pending: &mut Vec<PendingFile>,
) -> Result<LayerNetwork, ConfigError> {
    if path.as_os_str().is_empty() {
        return Ok(inherited.clone());
    }
    let original = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(inherited.clone()),
        Err(source) => {
            return Err(ConfigError::ReadFile {
                path: path.into(),
                source,
            });
        }
    };
    let mut document: Value =
        serde_json::from_slice(&original).map_err(|source| ConfigError::ParseFile {
            path: path.into(),
            source,
        })?;
    let before = document.clone();
    let mut result = inherited.clone();
    if let Some(network) = document.pointer("/permission/network") {
        let legacy = is_legacy_network(network);
        let effective = if legacy {
            let overlay =
                NetworkPermissionConfig::from_legacy_value(network.clone()).map_err(|reason| {
                    ConfigError::Validation(format!("{}: {reason}", path.display()))
                })?;
            let mut effective = inherited.historical.clone();
            effective.merge_legacy_from(overlay);
            effective.resolved()
        } else {
            let overlay = serde_json::from_value::<NetworkPermissionConfig>(network.clone())
                .map_err(|source| ConfigError::ParseFile {
                    path: path.into(),
                    source,
                })?;
            let mut effective = inherited.current.clone();
            effective.merge_from(overlay);
            effective.resolved()
        };
        effective
            .validate_for_platform(std::env::consts::OS)
            .map_err(|reason| ConfigError::Validation(format!("{}: {reason}", path.display())))?;
        if legacy {
            document["permission"]["network"] = serde_json::to_value(&effective)?;
        }
        result = LayerNetwork {
            historical: effective.clone(),
            current: effective,
        };
    }
    normalize_network_settings_document(&mut document);
    if document != before {
        let mut replacement = serde_json::to_vec_pretty(&document)?;
        replacement.push(b'\n');
        pending.push(PendingFile {
            path: path.into(),
            original,
            replacement,
        });
    }
    Ok(result)
}

/// Upgrade recognized old policies with a private backup of each original file.
/// Unversioned flat blocks are complete policies, including an offline {}.
pub fn migrate_network_config_files(
    global_path: &Path,
    workspace_path: &Path,
) -> Result<NetworkConfigMigration, ConfigError> {
    crate::with_config_file_write_lock(|| {
        let mut pending = Vec::new();
        // Legacy directional overrides inherited an offline root. Keep that
        // historical policy even if new, unconfigured projects allow outbound.
        let global = read_layer(
            global_path,
            &LayerNetwork {
                historical: NetworkPermissionConfig::offline(),
                current: NetworkPermissionConfig::allow_all(),
            },
            &mut pending,
        )?;
        let effective = if workspace_path == global_path {
            global.clone()
        } else {
            read_layer(workspace_path, &global, &mut pending)?
        };
        // Validate every candidate before creating any backup or replacing a file.
        for file in &pending {
            let current = fs::read(&file.path).map_err(|source| ConfigError::ReadFile {
                path: file.path.clone(),
                source,
            })?;
            if current != file.original {
                return Err(ConfigError::Validation(format!(
                    "{} changed during network migration; retry",
                    file.path.display()
                )));
            }
        }
        let mut backups = Vec::new();
        for file in &pending {
            let backup = (|| -> std::io::Result<PathBuf> {
                let parent = file.path.parent().unwrap_or_else(|| Path::new("."));
                let name = file.path.file_name().unwrap_or_default().to_string_lossy();
                let mut backup = tempfile::Builder::new()
                    .prefix(&format!("{name}.network-v2-"))
                    .suffix(".bak")
                    .tempfile_in(parent)?;
                backup.write_all(&file.original)?;
                backup.as_file().sync_all()?;
                let (_, path) = backup.keep().map_err(|error| error.error)?;
                agena_storage_layout::secure_file(&path)?;
                Ok(path)
            })()
            .map_err(|source| ConfigError::WriteFile {
                path: file.path.clone(),
                source,
            })?;
            backups.push(backup);
        }
        for (index, file) in pending.iter().enumerate() {
            if let Err(source) = crate::write_config_file_atomically(&file.path, &file.replacement)
            {
                // Earlier layers can be safely restored from the captured bytes.
                // Backups also remain available if the filesystem cannot restore.
                for previous in &pending[..index] {
                    let _ = crate::write_config_file_atomically(&previous.path, &previous.original);
                }
                return Err(ConfigError::WriteFile {
                    path: file.path.clone(),
                    source,
                });
            }
        }
        Ok(NetworkConfigMigration {
            global: global.historical,
            effective: effective.historical,
            migrated_files: pending.into_iter().map(|file| file.path).collect(),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn partial_legacy_layers_keep_effective_lists_and_backups() {
        let root = tempfile::tempdir().unwrap();
        let global = root.path().join("global.json");
        let workspace = root.path().join("workspace.json");
        let global_text = json!({"permission":{"network":{"egress":{
            "default":"allow", "deny":[{"to":[{"cidr":"192.0.2.0/24"}]}]
        }}}})
        .to_string();
        let workspace_text = json!({"permission":{"network":{"egress":{"default":"deny",
            "allow":[{"to":[{"cidr":"198.51.100.10/32"}],"ports":[{"protocol":"tcp","port":443}]}]
        }}}})
        .to_string();
        fs::write(&global, &global_text).unwrap();
        fs::write(&workspace, &workspace_text).unwrap();
        let result = migrate_network_config_files(&global, &workspace).unwrap();
        assert_eq!(result.migrated_files.len(), 2);
        let value = serde_json::to_value(result.effective).unwrap();
        assert_eq!(value["default"], "deny");
        assert_eq!(value["allow"].as_array().unwrap().len(), 1);
        assert_eq!(value["deny"].as_array().unwrap().len(), 1);
        let backups: Vec<_> = fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|value| value == "bak"))
            .collect();
        assert_eq!(backups.len(), 2);
        let contents: Vec<_> = backups
            .iter()
            .map(|path| fs::read_to_string(path).unwrap())
            .collect();
        assert!(contents.contains(&global_text));
        assert!(contents.contains(&workspace_text));
        assert!(
            migrate_network_config_files(&global, &workspace)
                .unwrap()
                .migrated_files
                .is_empty()
        );
    }

    #[test]
    fn legacy_partial_policy_keeps_the_historical_offline_root() {
        for in_workspace in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let global = root.path().join("global.json");
            let workspace = root.path().join("workspace.json");
            let authored = if in_workspace { &workspace } else { &global };
            fs::write(authored, r#"{"permission":{"network":{"egress":{}}}}"#).unwrap();
            let result = migrate_network_config_files(&global, &workspace).unwrap();
            assert_eq!(result.effective, NetworkPermissionConfig::offline());
            assert_eq!(result.migrated_files, vec![authored.clone()]);
            let saved: Value = serde_json::from_slice(&fs::read(authored).unwrap()).unwrap();
            assert_eq!(
                saved["permission"]["network"],
                json!({"version":2,"default":"deny","allow":[],"deny":[],"inbound":"deny","hostLoopback":"deny"})
            );
            let resolved = serde_json::from_value::<crate::RawConfig>(saved)
                .unwrap()
                .resolve()
                .unwrap();
            assert_eq!(
                resolved.permission.network.unwrap(),
                NetworkPermissionConfig::offline()
            );
        }
    }

    #[test]
    fn unversioned_empty_object_migrates_to_an_explicit_offline_policy() {
        let root = tempfile::tempdir().unwrap();
        let global = root.path().join("global.json");
        let workspace = root.path().join("workspace.json");
        fs::write(&global, r#"{"permission":{"network":{"default":"allow"}}}"#).unwrap();
        let before = r#"{"permission":{"network":{}}}"#;
        fs::write(&workspace, before).unwrap();
        let result = migrate_network_config_files(&global, &workspace).unwrap();
        assert_eq!(result.effective, NetworkPermissionConfig::offline());
        assert_eq!(result.migrated_files.len(), 2);
        let saved: Value = serde_json::from_slice(&fs::read(&workspace).unwrap()).unwrap();
        assert_eq!(
            saved["permission"]["network"],
            json!({"version":2,"default":"deny","allow":[],"deny":[],"inbound":"deny","hostLoopback":"deny"})
        );
        assert!(
            migrate_network_config_files(&global, &workspace)
                .unwrap()
                .migrated_files
                .is_empty()
        );
    }

    #[test]
    fn version_two_partial_policy_inherits_current_defaults_without_rewriting() {
        let root = tempfile::tempdir().unwrap();
        let global = root.path().join("global.json");
        let workspace = root.path().join("workspace.json");
        let original = r#"{"permission":{"network":{"version":2,"hostLoopback":"deny"}}}"#;
        fs::write(&workspace, original).unwrap();
        let result = migrate_network_config_files(&global, &workspace).unwrap();
        assert_eq!(result.effective, NetworkPermissionConfig::allow_all());
        assert!(result.migrated_files.is_empty());
        assert_eq!(fs::read_to_string(workspace).unwrap(), original);
    }

    #[test]
    fn invalid_legacy_child_preserves_both_files() {
        let root = tempfile::tempdir().unwrap();
        let global = root.path().join("global.json");
        let workspace = root.path().join("workspace.json");
        let before = r#"{"permission":{"network":{"egress":{"default":"allow"}}}}"#;
        let invalid = r#"{"permission":{"network":{"egress":{"allow":[{"ports":[]}]}}}}"#;
        fs::write(&global, before).unwrap();
        fs::write(&workspace, invalid).unwrap();
        assert!(migrate_network_config_files(&global, &workspace).is_err());
        assert_eq!(fs::read_to_string(global).unwrap(), before);
        assert_eq!(fs::read_to_string(workspace).unwrap(), invalid);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    }
}
