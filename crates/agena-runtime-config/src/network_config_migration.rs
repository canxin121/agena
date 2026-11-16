//! One-time conversion of authored directional network settings to flat policies.
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

struct PendingFile {
    path: PathBuf,
    original: Vec<u8>,
    replacement: Vec<u8>,
}

fn read_layer(
    path: &Path,
    inherited: &NetworkPermissionConfig,
    pending: &mut Vec<PendingFile>,
) -> Result<NetworkPermissionConfig, ConfigError> {
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
    let Some(network) = document.pointer("/permission/network") else {
        return Ok(inherited.clone());
    };
    let legacy = is_legacy_network(network);
    let effective = if legacy {
        let overlay = NetworkPermissionConfig::from_legacy_value(network.clone())
            .map_err(|reason| ConfigError::Validation(format!("{}: {reason}", path.display())))?;
        let mut effective = inherited.clone();
        effective.merge_legacy_from(overlay);
        effective.resolved()
    } else {
        serde_json::from_value(network.clone()).map_err(|source| ConfigError::ParseFile {
            path: path.into(),
            source,
        })?
    };
    effective
        .validate_for_platform(std::env::consts::OS)
        .map_err(|reason| ConfigError::Validation(format!("{}: {reason}", path.display())))?;
    if legacy {
        document["permission"]["network"] = serde_json::to_value(&effective)?;
        let mut replacement = serde_json::to_vec_pretty(&document)?;
        replacement.push(b'\n');
        pending.push(PendingFile {
            path: path.into(),
            original,
            replacement,
        });
    }
    Ok(effective)
}

/// Upgrade only recognized directional policies, with a private backup of each
/// original file. New flat objects, including {}, always use whole-block semantics.
pub fn migrate_network_config_files(
    global_path: &Path,
    workspace_path: &Path,
) -> Result<NetworkConfigMigration, ConfigError> {
    crate::with_config_file_write_lock(|| {
        let mut pending = Vec::new();
        let global = read_layer(
            global_path,
            &NetworkPermissionConfig::offline(),
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
                    .prefix(&format!("{name}.network-v1-"))
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
            global,
            effective,
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
    fn new_empty_object_is_explicit_offline_and_is_not_migrated() {
        let root = tempfile::tempdir().unwrap();
        let global = root.path().join("global.json");
        let workspace = root.path().join("workspace.json");
        fs::write(&global, r#"{"permission":{"network":{"default":"allow"}}}"#).unwrap();
        let before = r#"{"permission":{"network":{}}}"#;
        fs::write(&workspace, before).unwrap();
        let result = migrate_network_config_files(&global, &workspace).unwrap();
        assert_eq!(result.effective, NetworkPermissionConfig::offline());
        assert!(result.migrated_files.is_empty());
        assert_eq!(fs::read_to_string(workspace).unwrap(), before);
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
