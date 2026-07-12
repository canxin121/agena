impl ApiService {
    pub fn list_memories(
        &self,
        runtime: &agena::runtime::AgenaRuntime,
    ) -> ApiResult<Vec<MemoryResource>> {
        let store = MemoryStore::for_workspace(runtime.workspace_root());
        let entries = store.list().map_err(memory_api_error)?;
        Ok(entries.into_iter().map(memory_resource).collect())
    }

    pub fn get_memory(
        &self,
        runtime: &agena::runtime::AgenaRuntime,
        name: &str,
    ) -> ApiResult<MemoryResource> {
        let name = validate_memory_name(name)?;
        let store = MemoryStore::for_workspace(runtime.workspace_root());
        store
            .get(name)
            .map(memory_resource)
            .map_err(memory_api_error)
    }

    pub fn save_memory(
        &self,
        runtime: &agena::runtime::AgenaRuntime,
        name: &str,
        request: MemoryWriteRequest,
    ) -> ApiResult<MemoryResource> {
        let name = validate_memory_name(name)?.to_string();
        if request.body.trim().is_empty() {
            return Err(ApiError::bad_request("memory body is required"));
        }
        let store = MemoryStore::for_workspace(runtime.workspace_root());
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
        store
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

    pub fn delete_memory(
        &self,
        runtime: &agena::runtime::AgenaRuntime,
        name: &str,
    ) -> ApiResult<MemoryResource> {
        let name = validate_memory_name(name)?;
        let store = MemoryStore::for_workspace(runtime.workspace_root());
        let existing = store.get(name).map_err(memory_api_error)?;
        store.forget(name).map_err(memory_api_error)?;
        Ok(memory_resource(existing))
    }
}

fn validate_memory_name(name: &str) -> ApiResult<&str> {
    let normalized = name.trim().trim_end_matches(".md");
    if normalized.is_empty()
        || normalized == "MEMORY"
        || normalized.len() > 128
        || !normalized.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        return Err(ApiError::bad_request(
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

fn memory_api_error(error: MemoryError) -> ApiError {
    match error {
        MemoryError::NotFound(name) => ApiError::not_found(format!("memory not found: {name}")),
        MemoryError::Malformed { .. } | MemoryError::Yaml(_) => {
            ApiError::bad_request(error.to_string())
        }
        MemoryError::Io(_) => ApiError::internal(error.to_string()),
    }
}

use super::{
    ApiError, ApiResult, ApiService, MemoryError, MemoryRecord, MemoryResource, MemoryStore,
    MemoryWriteRequest, NewMemory,
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
