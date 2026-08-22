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
    use super::validate_memory_name;

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
}
