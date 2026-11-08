use std::collections::{BTreeMap, HashSet};

use agena_storage::{MemoryRepository, MemoryRevision};

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
        let name = validate_memory_name(name)?;
        recover_operator_memory(self.memory_repository.as_ref())?;
        self.memory_repository
            .get(name)
            .map(|record| record.path)
            .map_err(memory_api_error)
    }

    pub fn forget_memory(&self, name: &str) -> ApplicationResult<()> {
        let name = validate_memory_name(name)?;
        recover_operator_memory(self.memory_repository.as_ref())?;
        self.memory_repository
            .forget(name)
            .map_err(memory_api_error)
    }

    pub fn list_memories(&self) -> ApplicationResult<Vec<MemoryResource>> {
        recover_operator_memory(self.memory_repository.as_ref())?;
        let entries = self.memory_repository.list().map_err(memory_api_error)?;
        Ok(entries.into_iter().map(memory_resource).collect())
    }

    pub fn get_memory(&self, name: &str) -> ApplicationResult<MemoryResource> {
        let name = validate_memory_name(name)?;
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

    pub fn delete_memory(&self, name: &str) -> ApplicationResult<MemoryResource> {
        self.delete_memory_checked(name, None)
    }

    pub fn delete_memory_checked(
        &self,
        name: &str,
        expected: Option<&str>,
    ) -> ApplicationResult<MemoryResource> {
        delete_memory_record(self.memory_repository.as_ref(), name, expected)
    }

    pub fn delete_memories(&self, names: &[String]) -> ApplicationResult<Vec<String>> {
        self.delete_memories_checked(names, &BTreeMap::new())
    }

    pub fn delete_memories_checked(
        &self,
        names: &[String],
        expected: &BTreeMap<String, String>,
    ) -> ApplicationResult<Vec<String>> {
        delete_memory_records(self.memory_repository.as_ref(), names, expected)
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
    let name = validate_memory_name(name)?.to_string();
    if request.body.trim().is_empty() {
        return Err(ApplicationError::bad_request("memory body is required"));
    }
    let description = request.description.trim().to_string();
    let label = if description.is_empty() {
        request
            .body
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("memory")
            .trim()
    } else {
        description.as_str()
    };
    let index_label = label.replace(['\r', '\n'], " ");
    let entry = NewMemory {
        name: name.clone(),
        description,
        memory_type: request.memory_type,
        body: request.body,
        index_line: Some(format!("- [{name}]({name}.md) — {index_label}")),
    };
    recover_operator_memory(repository)?;
    let result = match request.expected_sha256.as_deref() {
        Some(expected) => repository.save_checked(entry, Some(expected.trim())),
        // Older clients retain their existing write contract.
        None => repository.save(entry),
    };
    result.map(memory_resource).map_err(memory_api_error)
}

fn delete_memory_record(
    repository: &dyn MemoryRepository,
    name: &str,
    expected: Option<&str>,
) -> ApplicationResult<MemoryResource> {
    let name = validate_memory_name(name)?;
    recover_operator_memory(repository)?;
    let existing = repository.get(name).map_err(memory_api_error)?;
    // Even a legacy call protects the record observed by this operation from
    // another writer arriving between the read and the delete.
    repository
        .forget_checked(name, Some(expected.unwrap_or(&existing.sha256).trim()))
        .map_err(memory_api_error)?;
    Ok(memory_resource(existing))
}

fn delete_memory_records(
    repository: &dyn MemoryRepository,
    names: &[String],
    expected: &BTreeMap<String, String>,
) -> ApplicationResult<Vec<String>> {
    if names.is_empty() || names.len() > 1000 || expected.len() > 1000 {
        return Err(ApplicationError::bad_request(
            "select between 1 and 1000 memory entries",
        ));
    }
    let mut seen = HashSet::new();
    let mut normalized = Vec::with_capacity(names.len());
    for name in names {
        let name = validate_memory_name(name)?.to_owned();
        if seen.insert(name.clone()) {
            normalized.push(name);
        }
    }
    let mut revisions = BTreeMap::new();
    for (name, sha256) in expected {
        let name = validate_memory_name(name)?.to_owned();
        if !seen.contains(&name) || revisions.insert(name, sha256.trim()).is_some() {
            return Err(ApplicationError::bad_request(
                "memory revisions must match the selected entries",
            ));
        }
    }
    recover_operator_memory(repository)?;
    let mut selected = Vec::with_capacity(normalized.len());
    let mut deleted = Vec::with_capacity(normalized.len());
    for name in normalized {
        let record = repository.get(&name).map_err(memory_api_error)?;
        let expected_sha = revisions.get(&name).copied().unwrap_or(&record.sha256);
        if !expected_sha.eq_ignore_ascii_case(&record.sha256) {
            return Err(ApplicationError::conflict(
                "The memory changed. Reload the latest version before editing or deleting.",
            ));
        }
        selected.push(MemoryRevision {
            name: name.clone(),
            sha256: record.sha256,
        });
        let display_name = record.frontmatter.name.trim();
        deleted.push(if display_name.is_empty() {
            name
        } else {
            display_name.to_owned()
        });
    }
    // Recheck every revision and delete the entire selection in one transaction.
    repository
        .forget_many_checked(&selected)
        .map_err(memory_api_error)?;
    Ok(deleted)
}

fn validate_memory_name(name: &str) -> ApplicationResult<&str> {
    let normalized = name.trim().trim_end_matches(".md");
    if normalized.is_empty()
        || normalized == "MEMORY"
        || normalized.len() > 128
        || !normalized.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        return Err(ApplicationError::bad_request(
            "memory names must use 1-128 letters, numbers, dots, dashes, or underscores",
        ));
    }
    Ok(normalized)
}

fn memory_resource(record: MemoryRecord) -> MemoryResource {
    let name = record.frontmatter.name.trim().to_string();
    MemoryResource {
        name: if name.is_empty() {
            record.file_name.trim_end_matches(".md").to_string()
        } else {
            name
        },
        file_name: record.file_name,
        path: record.path.display().to_string(),
        description: record.frontmatter.description,
        memory_type: record.frontmatter.r#type,
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

    fn request(body: &str, expected: Option<&str>) -> MemoryWriteRequest {
        serde_json::from_value(serde_json::json!({
            "description": "Project decision",
            "memory_type": "project",
            "body": body,
            "expected_sha256": expected,
        }))
        .unwrap()
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
        assert!(delete_memory_record(&store, "decision", Some(&first.sha256)).is_err());
        assert_eq!(store.get("decision").unwrap().sha256, current.sha256);
        let wire = serde_json::to_value(&current).unwrap();
        assert_eq!(wire["sha256"], current.sha256);
        assert_eq!(wire["metadata"]["status"], "active");
        assert_eq!(wire["metadata"]["origin"], "explicit");
        assert!(wire["metadata"]["updated_at_ms"].as_i64().unwrap() > 0);
        delete_memory_record(&store, "decision", Some(&current.sha256)).unwrap();
        assert!(store.get("decision").is_err());
    }

    #[test]
    fn memory_api_old_requests_remain_compatible() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::at(dir.path());
        let first: MemoryWriteRequest =
            serde_json::from_value(serde_json::json!({"body":"First"})).unwrap();
        let second: MemoryWriteRequest =
            serde_json::from_value(serde_json::json!({"body":"Second"})).unwrap();
        save_memory_record(&store, "legacy", first).unwrap();
        assert!(
            save_memory_record(&store, "legacy", second)
                .unwrap()
                .body
                .contains("Second")
        );
        delete_memory_record(&store, "legacy", None).unwrap();
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn memory_api_bulk_revision_conflict_does_not_delete_other_selected_records() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::at(dir.path());
        let a = save_memory_record(&store, "a", request("A", None)).unwrap();
        let b = save_memory_record(&store, "b", request("B", None)).unwrap();
        let current =
            save_memory_record(&store, "b", request("Updated B", Some(&b.sha256))).unwrap();
        let names = vec!["a".into(), "b".into()];
        let mut revisions =
            BTreeMap::from([("a".into(), a.sha256.clone()), ("b".into(), b.sha256)]);
        let stale = delete_memory_records(&store, &names, &revisions).unwrap_err();
        assert_eq!(stale.failure.category, FailureCategory::Conflict);
        assert_eq!(store.list().unwrap().len(), 2);
        revisions.insert("b".into(), current.sha256);
        assert_eq!(
            delete_memory_records(&store, &names, &revisions).unwrap(),
            names
        );
        assert!(store.list().unwrap().is_empty());
    }
}
