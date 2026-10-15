use std::collections::HashSet;

impl ApplicationService {
    pub fn memory_directory(&self) -> std::path::PathBuf {
        self.memory_repository.directory()
    }

    pub fn memory_index_path(&self) -> ApplicationResult<std::path::PathBuf> {
        self.memory_repository
            .ensure_index()
            .map_err(memory_api_error)
    }

    pub fn memory_entry_path(&self, name: &str) -> ApplicationResult<std::path::PathBuf> {
        let name = validate_memory_name(name)?;
        self.memory_repository
            .get(name)
            .map(|record| record.path)
            .map_err(memory_api_error)
    }

    pub fn forget_memory(&self, name: &str) -> ApplicationResult<()> {
        let name = validate_memory_name(name)?;
        self.memory_repository
            .forget(name)
            .map_err(memory_api_error)
    }

    pub fn list_memories(&self) -> ApplicationResult<Vec<MemoryResource>> {
        let entries = self.memory_repository.list().map_err(memory_api_error)?;
        Ok(entries.into_iter().map(memory_resource).collect())
    }

    pub fn get_memory(&self, name: &str) -> ApplicationResult<MemoryResource> {
        let name = validate_memory_name(name)?;
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
        self.memory_repository
            .save(NewMemory {
                name: name.clone(),
                description,
                memory_type: request.memory_type,
                body: request.body,
                index_line: Some(format!("- [{name}]({name}.md) — {index_label}")),
            })
            .map(memory_resource)
            .map_err(memory_api_error)
    }

    pub fn delete_memory(&self, name: &str) -> ApplicationResult<MemoryResource> {
        let name = validate_memory_name(name)?;
        let existing = self.memory_repository.get(name).map_err(memory_api_error)?;
        self.memory_repository
            .forget(name)
            .map_err(memory_api_error)?;
        Ok(memory_resource(existing))
    }

    pub fn delete_memories(&self, names: &[String]) -> ApplicationResult<Vec<String>> {
        if names.is_empty() || names.len() > 1_000 {
            return Err(ApplicationError::bad_request(
                "select between 1 and 1000 memory entries",
            ));
        }
        let mut seen = HashSet::new();
        let mut normalized = Vec::with_capacity(names.len());
        for name in names {
            let name = validate_memory_name(name)?.to_string();
            if seen.insert(name.clone()) {
                normalized.push(name);
            }
        }

        // Read every record before deleting any of them, so a stale or invalid
        // name cannot leave the requested batch only partly applied.
        let mut records = Vec::with_capacity(normalized.len());
        for name in &normalized {
            records.push(self.memory_repository.get(name).map_err(memory_api_error)?);
        }

        let mut deleted = Vec::with_capacity(normalized.len());
        for (name, record) in normalized.iter().zip(records) {
            self.memory_repository
                .forget(name)
                .map_err(memory_api_error)?;
            let display_name = record.frontmatter.name.trim();
            deleted.push(if display_name.is_empty() {
                record.file_name.trim_end_matches(".md").to_string()
            } else {
                display_name.to_string()
            });
        }
        Ok(deleted)
    }
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
        MemoryError::Io(_) => ApplicationError::internal_error(&error),
    }
}

use super::{
    ApplicationError, ApplicationResult, ApplicationService, MemoryError, MemoryRecord,
    MemoryResource, MemoryWriteRequest, NewMemory,
};

#[cfg(test)]
mod tests {
    use std::{path::Path, sync::Arc};

    use agena_storage::{MemoryStore, store::SessionStore};
    use agena_storage_sqlite::{SeaPermissionRuleRepository, SeaWorkspaceRepository};
    use sea_orm::Database;

    use super::{ApplicationService, MemoryWriteRequest, validate_memory_name};

    async fn service(workspace: &Path) -> ApplicationService {
        let database = Arc::new(
            Database::connect("sqlite::memory:")
                .await
                .expect("open in-memory SQLite database"),
        );
        agena_storage_sqlite::initialize_schema(database.as_ref())
            .await
            .expect("initialize test schema");
        let sessions: Arc<dyn SessionStore> =
            agena_storage_sqlite::session_store(Arc::clone(&database), 16);
        ApplicationService::new(
            workspace.display().to_string(),
            Arc::new(MemoryStore::for_workspace(workspace)),
            Arc::new(SeaWorkspaceRepository::new(Arc::clone(&database))),
            Arc::new(SeaPermissionRuleRepository::new(database)),
            sessions,
        )
    }

    #[test]
    fn validates_memory_names_without_allowing_path_traversal() {
        assert_eq!(
            validate_memory_name("project-decisions.md").unwrap(),
            "project-decisions"
        );
        assert!(validate_memory_name("../secret").is_err());
        assert!(validate_memory_name("nested/secret").is_err());
        assert!(validate_memory_name("MEMORY.md").is_err());
    }

    #[tokio::test]
    async fn batch_memory_delete_validates_the_whole_batch_and_deduplicates_names() {
        let workspace = tempfile::tempdir().unwrap();
        let service = service(workspace.path()).await;
        for name in ["first", "second"] {
            service
                .save_memory(
                    name,
                    MemoryWriteRequest {
                        description: String::new(),
                        memory_type: None,
                        body: format!("Memory body for {name}."),
                    },
                )
                .unwrap();
        }

        let invalid_batch = vec!["first".to_owned(), "missing".to_owned()];
        assert!(service.delete_memories(&invalid_batch).is_err());
        assert!(service.get_memory("first").is_ok());

        let deleted = service
            .delete_memories(&[
                "first".to_owned(),
                "first.md".to_owned(),
                "second".to_owned(),
            ])
            .unwrap();
        assert_eq!(deleted, vec!["first", "second"]);
        assert!(service.get_memory("first").is_err());
        assert!(service.get_memory("second").is_err());
    }
}
