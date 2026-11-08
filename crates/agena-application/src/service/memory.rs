use std::collections::HashSet;

use agena_storage::{MemoryRepository, MemoryRevision, MemoryWriteOptions, normalize_memory_name};

impl ApplicationService {
    pub fn memory_directory(&self) -> std::path::PathBuf {
        self.memory_repository.directory()
    }

    pub fn memory_index_path(&self) -> ApplicationResult<std::path::PathBuf> {
        recover_operator_memory(self.memory_repository.as_ref())?;
        self.memory_repository
            .ensure_index()
            .map_err(memory_api_error)
    }

    pub fn memory_entry_path(&self, name: &str) -> ApplicationResult<std::path::PathBuf> {
        let name = normalize_memory_name(name).map_err(memory_api_error)?;
        recover_operator_memory(self.memory_repository.as_ref())?;
        self.memory_repository
            .get(name)
            .map(|record| record.path)
            .map_err(memory_api_error)
    }

    pub fn list_memories(&self) -> ApplicationResult<Vec<MemoryResource>> {
        recover_operator_memory(self.memory_repository.as_ref())?;
        let entries = self.memory_repository.list().map_err(memory_api_error)?;
        Ok(entries.into_iter().map(memory_resource).collect())
    }

    pub fn get_memory(&self, name: &str) -> ApplicationResult<MemoryResource> {
        let name = normalize_memory_name(name).map_err(memory_api_error)?;
        recover_operator_memory(self.memory_repository.as_ref())?;
        self.memory_repository
            .get(name)
            .map(memory_resource)
            .map_err(memory_api_error)
    }

    pub fn save_memory(
        &self,
        name: &str,
        request: MemoryWriteRequest,
    ) -> ApplicationResult<MemoryResource> {
        save_memory_record(self.memory_repository.as_ref(), name, request)
    }

    pub fn delete_memory(
        &self,
        name: &str,
        expected_sha256: &str,
    ) -> ApplicationResult<MemoryResource> {
        delete_memory_record(self.memory_repository.as_ref(), name, expected_sha256)
    }

    pub fn delete_memories(&self, records: &[MemoryRevision]) -> ApplicationResult<Vec<String>> {
        delete_memory_records(self.memory_repository.as_ref(), records)
    }
}

/// The authenticated operator surface owns the complete memory store. Model
/// tool and background callers use per-path authorization on MemoryStore.
fn recover_operator_memory(repository: &dyn MemoryRepository) -> ApplicationResult<()> {
    repository
        .recover_pending(&|_| true)
        .map(|_| ())
        .map_err(memory_api_error)
}

fn save_memory_record(
    repository: &dyn MemoryRepository,
    name: &str,
    request: MemoryWriteRequest,
) -> ApplicationResult<MemoryResource> {
    let name = normalize_memory_name(name)
        .map_err(memory_api_error)?
        .to_string();
    if request.body.trim().is_empty() {
        return Err(ApplicationError::bad_request("memory body is required"));
    }
    let entry = NewMemory {
        name,
        description: request.description.trim().to_string(),
        kind: request.kind,
        body: request.body,
    };
    recover_operator_memory(repository)?;
    repository
        .save_with_options(
            entry,
            request.expected_sha256.as_deref().map(str::trim),
            MemoryWriteOptions::default(),
        )
        .map(memory_resource)
        .map_err(memory_api_error)
}

fn delete_memory_record(
    repository: &dyn MemoryRepository,
    name: &str,
    expected: &str,
) -> ApplicationResult<MemoryResource> {
    let name = normalize_memory_name(name).map_err(memory_api_error)?;
    recover_operator_memory(repository)?;
    let existing = repository.get(name).map_err(memory_api_error)?;
    repository
        .forget_checked(name, expected.trim())
        .map_err(memory_api_error)?;
    Ok(memory_resource(existing))
}

fn delete_memory_records(
    repository: &dyn MemoryRepository,
    records: &[MemoryRevision],
) -> ApplicationResult<Vec<String>> {
    if records.is_empty() || records.len() > 1000 {
        return Err(ApplicationError::bad_request(
            "select between 1 and 1000 memory entries",
        ));
    }
    let mut seen = HashSet::new();
    let mut selected = Vec::with_capacity(records.len());
    for record in records {
        let name = normalize_memory_name(&record.name)
            .map_err(memory_api_error)?
            .to_owned();
        if !seen.insert(name.clone()) {
            return Err(ApplicationError::bad_request("duplicate memory deletion"));
        }
        if record.sha256.trim().is_empty() {
            return Err(ApplicationError::bad_request(
                "every deletion requires its observed sha256",
            ));
        }
        selected.push(MemoryRevision {
            name,
            sha256: record.sha256.trim().to_owned(),
        });
    }
    recover_operator_memory(repository)?;
    // Validate the caller's complete revision set and delete atomically.
    repository
        .forget_many_checked(&selected)
        .map_err(memory_api_error)?;
    Ok(selected.into_iter().map(|record| record.name).collect())
}

fn memory_resource(record: MemoryRecord) -> MemoryResource {
    MemoryResource {
        name: record.frontmatter.name,
        file_name: record.file_name,
        path: record.path.display().to_string(),
        description: record.frontmatter.description,
        body: record.body,
        sha256: record.sha256,
        metadata: record.metadata,
    }
}

fn memory_api_error(error: MemoryError) -> ApplicationError {
    match error {
        MemoryError::NotFound(name) => ApplicationError::not_found_with_diagnostic(
            "The memory entry was not found.",
            format!("memory not found: {name}"),
        ),
        MemoryError::Malformed { .. } | MemoryError::Yaml(_) => {
            ApplicationError::bad_request_with_diagnostic("The memory file is invalid.", error)
        }
        MemoryError::Io(ref source)
            if source.kind() == std::io::ErrorKind::InvalidInput
                && source.to_string().starts_with("memory revision conflict:") =>
        {
            ApplicationError::conflict_with_diagnostic(
                "The memory changed. Reload the latest version before editing or deleting.",
                error,
            )
        }
        MemoryError::Io(ref source) if source.kind() == std::io::ErrorKind::InvalidInput => {
            ApplicationError::bad_request_with_diagnostic("The memory operation is invalid.", error)
        }
        MemoryError::Io(_) => ApplicationError::internal_error(&error),
    }
}

use super::{
    ApplicationError, ApplicationResult, ApplicationService, MemoryError, MemoryRecord,
    MemoryResource, MemoryWriteRequest, NewMemory,
};

#[cfg(test)]
mod tests {
    use super::*;
    use agena_failure::FailureCategory;
    use agena_storage::MemoryStore;
    use serde_json::json;

    fn request(body: &str, expected: Option<&str>) -> MemoryWriteRequest {
        serde_json::from_value(json!({
            "description": "Project decision",
            "kind": "fact",
            "body": body,
            "expected_sha256": expected,
        }))
        .unwrap()
    }

    #[test]
    fn memory_api_manages_storage_created_unicode_and_long_names() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::at(dir.path());
        for name in ["项目 偏好".to_owned(), "x".repeat(129)] {
            let initial = store
                .save_with_options(
                    NewMemory {
                        name: name.clone(),
                        description: "Created through the shared storage".into(),
                        kind: agena_domain::MemoryKind::Fact,
                        body: "Original".into(),
                    },
                    None,
                    MemoryWriteOptions::default(),
                )
                .unwrap();
            let updated = save_memory_record(
                &store,
                &format!(" {name}.md "),
                request("Corrected through management", Some(&initial.sha256)),
            )
            .unwrap();
            assert_eq!(updated.name, name);
            assert_eq!(updated.path, initial.path.display().to_string());
            assert_eq!(updated.body.trim_end(), "Corrected through management");
            assert_eq!(store.get(&name).unwrap().sha256, updated.sha256);
            assert_eq!(store.list().unwrap().len(), 1);

            let deleted =
                delete_memory_record(&store, &format!("{name}.md.md"), &updated.sha256).unwrap();
            assert_eq!(deleted.name, name);
            assert_eq!(deleted.sha256, updated.sha256);
            assert!(store.list().unwrap().is_empty());
        }
    }

    #[test]
    fn memory_api_rejects_unsafe_and_reserved_names_before_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::at(dir.path());
        let original = save_memory_record(&store, "safe", request("Preserve this", None)).unwrap();
        let state_bytes = std::fs::read(store.metadata_path()).unwrap();
        for name in [
            String::new(),
            ".".to_owned(),
            "..".to_owned(),
            "MEMORY".to_owned(),
            "memory.md".to_owned(),
            "../outside".to_owned(),
            "folder/note".to_owned(),
            r"folder\note".to_owned(),
            "foo\nbar".to_owned(),
            "x".repeat(256),
        ] {
            assert!(
                save_memory_record(&store, &name, request("Invalid target", None)).is_err(),
                "{name:?} must not be writable"
            );
            assert!(
                delete_memory_record(&store, &name, &original.sha256).is_err(),
                "{name:?} must not be deletable"
            );
            assert!(
                delete_memory_records(
                    &store,
                    &[
                        MemoryRevision {
                            name: "safe".into(),
                            sha256: original.sha256.clone(),
                        },
                        MemoryRevision {
                            name: name.clone(),
                            sha256: original.sha256.clone(),
                        },
                    ],
                )
                .is_err(),
                "{name:?} must reject the whole batch"
            );
        }
        assert_eq!(store.list().unwrap().len(), 1);
        assert_eq!(store.get("safe").unwrap().sha256, original.sha256);
        assert_eq!(std::fs::read(store.metadata_path()).unwrap(), state_bytes);
    }

    #[test]
    fn memory_api_bulk_rejects_normalized_aliases_before_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::at(dir.path());
        let original =
            save_memory_record(&store, "项目 决策", request("Canonical record", None)).unwrap();
        let other = save_memory_record(&store, "other", request("Preserve other", None)).unwrap();
        let state_bytes = std::fs::read(store.metadata_path()).unwrap();
        let error = delete_memory_records(
            &store,
            &[
                MemoryRevision {
                    name: "other".into(),
                    sha256: other.sha256.clone(),
                },
                MemoryRevision {
                    name: " 项目 决策.md ".into(),
                    sha256: original.sha256.clone(),
                },
                MemoryRevision {
                    name: "项目 决策.md.md".into(),
                    sha256: original.sha256.clone(),
                },
            ],
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("duplicate memory deletion"),
            "{error}"
        );
        assert_eq!(store.list().unwrap().len(), 2);
        assert_eq!(store.get("项目 决策").unwrap().sha256, original.sha256);
        assert_eq!(store.get("other").unwrap().sha256, other.sha256);
        assert_eq!(std::fs::read(store.metadata_path()).unwrap(), state_bytes);
    }

    #[test]
    fn memory_api_reports_metadata_and_rejects_stale_edits_and_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::at(dir.path());
        let first = save_memory_record(&store, "decision", request("A", None)).unwrap();
        let current =
            save_memory_record(&store, "decision", request("B", Some(&first.sha256))).unwrap();
        let stale = save_memory_record(
            &store,
            "decision",
            request("lost update", Some(&first.sha256)),
        )
        .unwrap_err();
        assert_eq!(stale.failure.category, FailureCategory::Conflict);
        assert!(delete_memory_record(&store, "decision", &first.sha256).is_err());
        assert_eq!(store.get("decision").unwrap().sha256, current.sha256);
        let wire = serde_json::to_value(&current).unwrap();
        assert_eq!(wire["sha256"], current.sha256);
        assert_eq!(wire["metadata"]["kind"], "fact");
        assert_eq!(wire["metadata"]["status"], "active");
        assert_eq!(wire["metadata"]["origin"], "explicit");
        assert!(wire["metadata"]["updated_at_ms"].as_i64().unwrap() > 0);
        assert!(wire.get("memory_type").is_none());
        delete_memory_record(&store, "decision", &current.sha256).unwrap();
        assert!(store.get("decision").is_err());
    }

    #[test]
    fn memory_api_create_only_cannot_overwrite_and_deleted_fields_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::at(dir.path());
        let first = save_memory_record(&store, "decision", request("First", None)).unwrap();
        let error = save_memory_record(
            &store,
            "decision",
            request("Overwrite without reading", None),
        )
        .unwrap_err();
        assert_eq!(error.failure.category, FailureCategory::Conflict);
        assert_eq!(store.get("decision").unwrap().sha256, first.sha256);
        assert!(delete_memory_record(&store, "decision", "").is_err());
        assert!(
            serde_json::from_value::<MemoryWriteRequest>(json!({"body":"Missing classification"}))
                .is_err()
        );
        assert!(
            serde_json::from_value::<MemoryWriteRequest>(
                json!({"body":"Old classification","memory_type":"user"})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<MemoryWriteRequest>(
                json!({"body":"Mixed schema","kind":"preference","memory_type":"user"})
            )
            .is_err()
        );
        let mut response = serde_json::to_value(&first).unwrap();
        response.as_object_mut().unwrap().remove("metadata");
        assert!(serde_json::from_value::<MemoryResource>(response).is_err());
    }

    #[test]
    fn memory_api_bulk_revision_conflict_does_not_delete_other_selected_records() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::at(dir.path());
        let a = save_memory_record(&store, "a", request("A", None)).unwrap();
        let b = save_memory_record(&store, "b", request("B", None)).unwrap();
        let current =
            save_memory_record(&store, "b", request("Updated B", Some(&b.sha256))).unwrap();
        let mut revisions = vec![
            MemoryRevision {
                name: "a".into(),
                sha256: a.sha256,
            },
            MemoryRevision {
                name: "b".into(),
                sha256: b.sha256,
            },
        ];
        let stale = delete_memory_records(&store, &revisions).unwrap_err();
        assert_eq!(stale.failure.category, FailureCategory::Conflict);
        assert_eq!(store.list().unwrap().len(), 2);
        revisions[1].sha256 = current.sha256;
        assert_eq!(
            delete_memory_records(&store, &revisions).unwrap(),
            vec!["a", "b"]
        );
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn memory_api_bulk_requires_each_revision_and_rejects_duplicates_before_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::at(dir.path());
        let a = save_memory_record(&store, "a", request("A", None)).unwrap();
        let b = save_memory_record(&store, "b", request("B", None)).unwrap();
        let revisions = vec![
            MemoryRevision {
                name: "a".into(),
                sha256: a.sha256.clone(),
            },
            MemoryRevision {
                name: "b".into(),
                sha256: String::new(),
            },
        ];
        assert!(delete_memory_records(&store, &revisions).is_err());
        assert!(
            delete_memory_records(&store, &[revisions[0].clone(), revisions[0].clone()]).is_err()
        );
        assert_eq!(store.get("a").unwrap().sha256, a.sha256);
        assert_eq!(store.get("b").unwrap().sha256, b.sha256);
    }
}
