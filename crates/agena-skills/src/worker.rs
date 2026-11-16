//! Explicit, credential-free operations for a native skills worker.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    DeclaredSkill, Result, SkillError, SkillImport, SkillManager, SkillRoot, SkillScope,
    SkillSource,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillManagerSnapshot {
    pub workspace: PathBuf,
    pub global_root: PathBuf,
    pub roots: Vec<SkillRoot>,
    pub declared: Vec<DeclaredSkill>,
    pub disabled_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum SkillWorkerOperation {
    List,
    ResolvePath {
        id: String,
    },
    Get {
        id: String,
    },
    Create {
        scope: SkillScope,
        document: String,
    },
    Update {
        id: String,
        document: String,
        expected_revision: String,
    },
    Remove {
        id: String,
        expected_revision: String,
    },
    SetEnabled {
        id: String,
        enabled: bool,
        expected_revision: String,
    },
    ReadResource {
        id: String,
        path: String,
    },
    WriteResource {
        id: String,
        path: String,
        content: String,
        expected_revision: Option<String>,
    },
    RemoveResource {
        id: String,
        path: String,
        expected_revision: String,
    },
    PreviewInstall {
        package: SkillImport,
        scope: SkillScope,
    },
    Install {
        package: SkillImport,
        scope: SkillScope,
        selected: Vec<String>,
        replace: bool,
        source: SkillSource,
    },
}

impl SkillWorkerOperation {
    pub const fn is_mutation(&self) -> bool {
        matches!(
            self,
            Self::Create { .. }
                | Self::Update { .. }
                | Self::Remove { .. }
                | Self::SetEnabled { .. }
                | Self::WriteResource { .. }
                | Self::RemoveResource { .. }
                | Self::Install { .. }
        )
    }

    pub fn execute(self, manager: SkillManager) -> Result<Value> {
        fn value(value: impl Serialize) -> Result<Value> {
            serde_json::to_value(value).map_err(|error| SkillError::invalid(error.to_string()))
        }
        match self {
            Self::List => value(manager.list()?),
            Self::ResolvePath { id } => value(manager.resolve_path(&id)?),
            Self::Get { id } => value(manager.get(&id)?),
            Self::Create { scope, document } => value(manager.create(scope, &document)?),
            Self::Update {
                id,
                document,
                expected_revision,
            } => value(manager.update(&id, &document, Some(&expected_revision))?),
            Self::Remove {
                id,
                expected_revision,
            } => value(manager.remove(&id, Some(&expected_revision))?),
            Self::SetEnabled {
                id,
                enabled,
                expected_revision,
            } => value(manager.set_enabled(&id, enabled, Some(&expected_revision))?),
            Self::ReadResource { id, path } => value(manager.read_resource(&id, &path)?),
            Self::WriteResource {
                id,
                path,
                content,
                expected_revision,
            } => value(manager.write_resource(
                &id,
                &path,
                &content,
                Some(expected_revision.as_deref().unwrap_or("")),
            )?),
            Self::RemoveResource {
                id,
                path,
                expected_revision,
            } => value(manager.remove_resource(&id, &path, Some(&expected_revision))?),
            Self::PreviewInstall { package, scope } => {
                value(manager.preview_install(&package, scope)?)
            }
            Self::Install {
                package,
                scope,
                selected,
                replace,
                source,
            } => value(manager.install(&package, scope, &selected, replace, Some(source))?),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillWorkerRequest {
    pub manager: SkillManagerSnapshot,
    pub operation: SkillWorkerOperation,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SkillErrorKind, SkillImportFile};

    #[test]
    fn snapshot_keeps_resolved_roots_and_declared_skills() {
        let manager = SkillManager::new("/project".into(), "/instance/skills".into())
            .with_additional_roots(vec!["custom/skills".into()])
            .with_declared_skills(vec![DeclaredSkill {
                plugin_id: "example.plugin".into(),
                version: "1".into(),
                name: "sample".into(),
                description: "Sample".into(),
                instructions: "Use the sample.".into(),
                aliases: Vec::new(),
            }]);
        let roots = manager.roots();
        let encoded = serde_json::to_vec(&manager.worker_snapshot()).unwrap();
        let restored =
            SkillManager::from_worker_snapshot(serde_json::from_slice(&encoded).unwrap());
        assert_eq!(restored.roots(), roots);
        assert_eq!(
            restored.worker_snapshot().declared,
            manager.worker_snapshot().declared
        );
    }

    #[test]
    fn worker_keeps_revision_checks_and_enabled_state() {
        let directory = tempfile::tempdir().unwrap();
        let workspace = directory.path().join("project");
        std::fs::create_dir(&workspace).unwrap();
        let manager = SkillManager::new(workspace, directory.path().join("global"))
            .with_instance_only_roots();
        let package = SkillImport::from_files(vec![SkillImportFile {
            path: "SKILL.md".into(),
            content: b"---\nname: sample\ndescription: Sample skill\n---\nOriginal\n".to_vec(),
            executable: false,
        }])
        .unwrap();
        let installed = manager
            .install(&package, SkillScope::Global, &[], false, None)
            .unwrap();
        let skill = &installed.installed[0];
        let request = SkillWorkerRequest {
            manager: manager.worker_snapshot(),
            operation: SkillWorkerOperation::SetEnabled {
                id: skill.id.clone(),
                enabled: false,
                expected_revision: skill.revision.clone(),
            },
        };
        let request: SkillWorkerRequest =
            serde_json::from_slice(&serde_json::to_vec(&request).unwrap()).unwrap();
        let output = request
            .operation
            .execute(SkillManager::from_worker_snapshot(request.manager))
            .unwrap();
        assert_eq!(output["enabled"], false);
        let error = SkillWorkerOperation::Update {
            id: skill.id.clone(),
            document: "---\nname: sample\ndescription: Sample skill\n---\nNew\n".into(),
            expected_revision: skill.revision.clone(),
        }
        .execute(manager)
        .unwrap_err();
        assert_eq!(error.kind, SkillErrorKind::Conflict);
    }
    #[test]
    fn import_worker_uses_base64_and_revalidates_package_paths() {
        use crate::source::{SkillSourceOperation, SkillSourceResult};
        let document = b"---\nname: imported\ndescription: Imported skill\n---\nText\n";
        let mut archive = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(document.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive
            .append_data(&mut header, "imported/SKILL.md", &document[..])
            .unwrap();
        let operation = SkillSourceOperation::ImportArchive {
            filename: "skills.tar".into(),
            bytes: archive.into_inner().unwrap(),
            subpath: None,
        };
        let wire = serde_json::to_value(&operation).unwrap();
        assert!(wire["bytes"].is_string());
        let operation: SkillSourceOperation = serde_json::from_value(wire).unwrap();
        let SkillSourceResult::Package(package) = operation.execute().unwrap() else {
            panic!("expected imported package")
        };
        let encoded = serde_json::to_value(&package).unwrap();
        assert!(encoded[0]["content"].is_string());
        let restored: SkillImport = serde_json::from_value(encoded).unwrap();
        assert_eq!(restored.revision(), package.revision());
        assert!(
            serde_json::from_value::<SkillImport>(serde_json::json!([
                {"path": "../private.txt", "content": "", "executable": false}
            ]))
            .is_err()
        );
    }

    #[test]
    fn source_worker_rejects_unbounded_file_reads_before_io() {
        let error = crate::source::SkillSourceOperation::ReadFile {
            path: "absent".into(),
            limit: crate::source::MAX_SOURCE_BYTES + 1,
        }
        .execute()
        .unwrap_err();
        assert!(error.message.contains("64 MiB"));
    }
}
