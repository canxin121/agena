use agena_storage::WorkspaceListQuery as StorageWorkspaceListQuery;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use path_clean::PathClean;
use uuid::Uuid;

impl ApplicationService {
    pub async fn list_workspaces(
        &self,
        query: WorkspaceListQuery,
    ) -> ApplicationResult<PaginatedResponse<WorkspaceResource>> {
        let limit = normalize_limit(query.pagination.limit());
        let cursor = query
            .pagination
            .cursor()
            .map(decode_cursor::<WorkspaceCursor>)
            .transpose()?;
        let rows = self
            .workspace_repository
            .list(StorageWorkspaceListQuery {
                search: non_empty(query.pagination.search()).map(ToString::to_string),
                before_updated_at_ms: cursor.map(|value| value.updated_at_ms),
                before_id: cursor.map(|value| value.id),
                limit: limit + 1,
            })
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?;
        let (slice, has_more) = trim_page(rows, limit)?;
        let workspace_ids = slice.iter().map(|row| row.id).collect::<Vec<_>>();
        let session_counts = if query.include_session_count {
            self.workspace_session_counts(&workspace_ids).await?
        } else {
            HashMap::new()
        };
        let items = slice
            .iter()
            .map(|row| workspace_record_resource(row, session_counts.get(&row.id).copied()))
            .collect::<ApplicationResult<Vec<_>>>()?;
        let next_cursor = slice.last().map(|row| WorkspaceCursor {
            updated_at_ms: row.updated_at_ms,
            id: row.id,
        });

        build_page(items, has_more, next_cursor, PageOrder::Desc, limit)
    }

    pub async fn get_workspace(
        &self,
        workspace_id: i64,
    ) -> ApplicationResult<Option<WorkspaceResource>> {
        let row = self
            .workspace_repository
            .get(workspace_id)
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?;
        let Some(row) = row else {
            return Ok(None);
        };
        let counts = self.workspace_session_counts(&[row.id]).await?;
        Ok(Some(workspace_record_resource(
            &row,
            counts.get(&row.id).copied(),
        )?))
    }

    pub async fn list_workspace_files(
        &self,
        workspace_id: i64,
        query: WorkspaceFileTreeQuery,
    ) -> ApplicationResult<WorkspaceFileTreeResource> {
        let root = PathBuf::from(
            self.workspace_repository
                .path_by_id(workspace_id)
                .await
                .map_err(|error| ApplicationError::internal(error.to_string()))?
                .ok_or_else(|| {
                    ApplicationError::not_found_with_diagnostic(
                        "The workspace was not found.",
                        format!("workspace not found: {workspace_id}"),
                    )
                })?,
        );
        let root = root
            .canonicalize()
            .map_err(|error| workspace_fs_error(root.as_path(), error))?;
        if !root.is_dir() {
            return Err(ApplicationError::bad_request_with_diagnostic(
                "The workspace root is not a directory.",
                format!("workspace root is not a directory: {}", root.display()),
            ));
        }

        let relative_path = clean_workspace_relative_path(query.path.as_deref())?;
        let target = root.join(&relative_path).clean();
        let target = target
            .canonicalize()
            .map_err(|error| workspace_fs_error(target.as_path(), error))?;
        if !target.starts_with(&root) {
            return Err(ApplicationError::bad_request(
                "workspace file path escapes workspace root",
            ));
        }
        if !target.is_dir() {
            return Err(ApplicationError::bad_request_with_diagnostic(
                "The selected workspace path is not a directory.",
                format!(
                    "workspace path is not a directory: {}",
                    workspace_relative_path(&relative_path)
                ),
            ));
        }

        let depth = query.depth.unwrap_or(2).min(8);
        let mut remaining = query.limit.unwrap_or(500).clamp(1, 2_000);
        let entries =
            read_workspace_entries(root.as_path(), target.as_path(), depth, &mut remaining)?;

        Ok(WorkspaceFileTreeResource {
            workspace_id,
            root: root.display().to_string(),
            path: workspace_relative_path(&relative_path),
            entries,
        })
    }

    pub async fn read_workspace_file(
        &self,
        workspace_id: i64,
        query: WorkspaceFileDownloadQuery,
    ) -> ApplicationResult<(String, Vec<u8>)> {
        const MAX_DOWNLOAD_BYTES: u64 = 100 * 1024 * 1024;

        let root_path = PathBuf::from(
            self.workspace_repository
                .path_by_id(workspace_id)
                .await
                .map_err(|error| ApplicationError::internal(error.to_string()))?
                .ok_or_else(|| {
                    ApplicationError::not_found_with_diagnostic(
                        "The workspace was not found.",
                        format!("workspace not found: {workspace_id}"),
                    )
                })?,
        );
        let root = root_path
            .canonicalize()
            .map_err(|error| workspace_fs_error(root_path.as_path(), error))?;
        if !root.is_dir() {
            return Err(ApplicationError::bad_request_with_diagnostic(
                "The workspace root is not a directory.",
                format!("workspace root is not a directory: {}", root.display()),
            ));
        }

        let relative_path = clean_workspace_relative_path(Some(query.path.as_str()))?;
        if relative_path.as_os_str().is_empty() {
            return Err(ApplicationError::bad_request(
                "workspace file path cannot be empty",
            ));
        }
        let unresolved_target = root.join(&relative_path).clean();
        let target = unresolved_target
            .canonicalize()
            .map_err(|error| workspace_fs_error(unresolved_target.as_path(), error))?;
        if !target.starts_with(&root) {
            return Err(ApplicationError::bad_request(
                "workspace file path escapes workspace root",
            ));
        }
        let metadata = fs::metadata(target.as_path())
            .map_err(|error| workspace_fs_error(target.as_path(), error))?;
        if !metadata.is_file() {
            return Err(ApplicationError::bad_request_with_diagnostic(
                "The selected workspace path is not a file.",
                format!(
                    "workspace path is not a file: {}",
                    workspace_relative_path(&relative_path)
                ),
            ));
        }
        if metadata.len() > MAX_DOWNLOAD_BYTES {
            return Err(ApplicationError::bad_request_with_diagnostic(
                "The workspace file exceeds the 100 MiB download limit.",
                format!(
                    "workspace file exceeds download limit: {}",
                    workspace_relative_path(&relative_path)
                ),
            ));
        }

        let bytes = fs::read(target.as_path())
            .map_err(|error| workspace_fs_error(target.as_path(), error))?;
        let filename = target
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("workspace-file")
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                    character
                } else {
                    '_'
                }
            })
            .collect();

        Ok((filename, bytes))
    }

    pub async fn upload_workspace_file(
        &self,
        workspace_id: i64,
        request: WorkspaceFileUploadRequest,
    ) -> ApplicationResult<WorkspaceFileUploadResource> {
        const MAX_UPLOAD_BYTES: u64 = 50 * 1024 * 1024;

        let root_path = PathBuf::from(
            self.workspace_repository
                .path_by_id(workspace_id)
                .await
                .map_err(|error| ApplicationError::internal(error.to_string()))?
                .ok_or_else(|| {
                    ApplicationError::not_found_with_diagnostic(
                        "The workspace was not found.",
                        format!("workspace not found: {workspace_id}"),
                    )
                })?,
        );
        let root = root_path
            .canonicalize()
            .map_err(|error| workspace_fs_error(root_path.as_path(), error))?;
        if !root.is_dir() {
            return Err(ApplicationError::bad_request_with_diagnostic(
                "The workspace root is not a directory.",
                format!("workspace root is not a directory: {}", root.display()),
            ));
        }

        let filename = sanitize_upload_filename(request.filename.as_str());
        if filename.is_empty() {
            return Err(ApplicationError::bad_request(
                "The uploaded file needs a non-empty filename.",
            ));
        }

        let decoded = BASE64_STANDARD
            .decode(request.data_base64.trim().as_bytes())
            .map_err(|_| {
                ApplicationError::bad_request("The uploaded file data is not valid base64.")
            })?;
        if decoded.is_empty() {
            return Err(ApplicationError::bad_request(
                "The uploaded file is empty.",
            ));
        }
        if decoded.len() as u64 > MAX_UPLOAD_BYTES {
            return Err(ApplicationError::bad_request_with_diagnostic(
                "The uploaded file exceeds the 50 MiB upload limit.",
                format!("uploaded file is {} bytes", decoded.len()),
            ));
        }

        let upload_dir = root.join(".agena").join("uploads");
        fs::create_dir_all(upload_dir.as_path())
            .map_err(|error| workspace_fs_error(upload_dir.as_path(), error))?;
        let stored_name = format!("{}-{}", Uuid::new_v4().simple(), filename);
        let target = upload_dir.join(&stored_name);
        fs::write(target.as_path(), decoded.as_slice())
            .map_err(|error| workspace_fs_error(target.as_path(), error))?;

        Ok(WorkspaceFileUploadResource {
            workspace_id,
            path: format!(".agena/uploads/{stored_name}"),
            name: filename,
            mime: non_empty(request.mime.as_deref()).map(ToOwned::to_owned),
            size_bytes: decoded.len() as u64,
        })
    }

    pub async fn create_workspace(
        &self,
        request: WorkspacePathRequest,
    ) -> ApplicationResult<WorkspaceResource> {
        let path = normalize_workspace_path(request.path.as_str()).map_err(|error| {
            ApplicationError::bad_request_with_diagnostic("The workspace path is invalid.", error)
        })?;
        if self.workspace_id_by_path(path.as_str()).await?.is_some() {
            return Err(ApplicationError::conflict_with_diagnostic(
                "A workspace already uses this path.",
                format!("workspace path already exists: {path}"),
            ));
        }

        let created = self
            .workspace_repository
            .create(path)
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?;

        workspace_record_resource(&created, Some(0))
    }

    pub async fn resolve_workspace(
        &self,
        request: WorkspaceResolveRequest,
    ) -> ApplicationResult<WorkspaceResource> {
        let path = normalize_workspace_path(request.workspace.path.as_str()).map_err(|error| {
            ApplicationError::bad_request_with_diagnostic("The workspace path is invalid.", error)
        })?;
        if let Some(workspace_id) = self.workspace_id_by_path(path.as_str()).await? {
            return self.get_workspace(workspace_id).await?.ok_or_else(|| {
                ApplicationError::internal(format!(
                    "workspace {workspace_id} disappeared while resolving path {path}"
                ))
            });
        }

        if !request.create_if_missing {
            return Err(ApplicationError::not_found_with_diagnostic(
                "The workspace was not found.",
                format!("workspace not found for path: {path}"),
            ));
        }

        match self
            .create_workspace(WorkspacePathRequest { path: path.clone() })
            .await
        {
            Ok(workspace) => Ok(workspace),
            Err(error) => {
                if let Some(workspace_id) = self.workspace_id_by_path(path.as_str()).await? {
                    return self.get_workspace(workspace_id).await?.ok_or_else(|| {
                        ApplicationError::internal(format!(
                            "workspace {workspace_id} disappeared while resolving path {path}"
                        ))
                    });
                }
                Err(error)
            }
        }
    }

    pub async fn replace_workspace(
        &self,
        workspace_id: i64,
        request: WorkspacePathRequest,
    ) -> ApplicationResult<WorkspaceResource> {
        let Some(existing) = self
            .workspace_repository
            .get(workspace_id)
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?
        else {
            return Err(ApplicationError::not_found_with_diagnostic(
                "The workspace was not found.",
                format!("workspace not found: {workspace_id}"),
            ));
        };

        let path = normalize_workspace_path(request.path.as_str()).map_err(|error| {
            ApplicationError::bad_request_with_diagnostic("The workspace path is invalid.", error)
        })?;
        if path != existing.path
            && let Some(existing_id) = self.workspace_id_by_path(path.as_str()).await?
            && existing_id != workspace_id
        {
            return Err(ApplicationError::conflict_with_diagnostic(
                "A workspace already uses this path.",
                format!("workspace path already exists: {path}"),
            ));
        }

        let Some(updated) = self
            .workspace_repository
            .update_path(workspace_id, path)
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?
        else {
            return Err(ApplicationError::not_found_with_diagnostic(
                "The workspace was not found.",
                format!("workspace not found: {workspace_id}"),
            ));
        };
        let counts = self.workspace_session_counts(&[updated.id]).await?;
        workspace_record_resource(&updated, counts.get(&updated.id).copied())
    }

    pub async fn delete_workspace(
        &self,
        workspace_id: i64,
    ) -> ApplicationResult<WorkspaceResource> {
        let Some(existing) = self
            .workspace_repository
            .get(workspace_id)
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?
        else {
            return Err(ApplicationError::not_found_with_diagnostic(
                "The workspace was not found.",
                format!("workspace not found: {workspace_id}"),
            ));
        };

        let counts = self.workspace_session_counts(&[workspace_id]).await?;
        self.workspace_repository
            .delete(workspace_id)
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?;
        workspace_record_resource(&existing, counts.get(&workspace_id).copied())
    }
}

fn workspace_record_resource(
    row: &agena_storage::WorkspaceRecord,
    session_count: Option<u64>,
) -> ApplicationResult<WorkspaceResource> {
    Ok(WorkspaceResource {
        id: row.id,
        path: row.path.clone(),
        created_at: timestamp_millis_to_utc(row.created_at_ms)?,
        updated_at: timestamp_millis_to_utc(row.updated_at_ms)?,
        session_count,
    })
}

fn clean_workspace_relative_path(value: Option<&str>) -> ApplicationResult<PathBuf> {
    let mut cleaned = PathBuf::new();
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(cleaned);
    };
    let path = Path::new(value);
    if path.is_absolute() {
        return Err(ApplicationError::bad_request(
            "workspace file path must be relative",
        ));
    }
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => cleaned.push(part),
            std::path::Component::CurDir => {}
            _ => {
                return Err(ApplicationError::bad_request(
                    "workspace file path cannot contain parent or root components",
                ));
            }
        }
    }
    Ok(cleaned)
}

fn read_workspace_entries(
    root: &Path,
    dir: &Path,
    depth: usize,
    remaining: &mut usize,
) -> ApplicationResult<Vec<WorkspaceFileNode>> {
    if *remaining == 0 {
        return Ok(Vec::new());
    }

    let mut entries = fs::read_dir(dir)
        .map_err(|error| workspace_fs_error(dir, error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| workspace_fs_error(dir, error))?;
    entries.sort_by_key(|entry| entry.file_name());

    let mut nodes = Vec::new();
    for entry in entries {
        if *remaining == 0 {
            break;
        }

        let path = entry.path();
        let metadata = fs::symlink_metadata(path.as_path())
            .map_err(|error| workspace_fs_error(path.as_path(), error))?;
        let file_type = metadata.file_type();
        let kind = if file_type.is_dir() {
            WorkspaceFileKind::Directory
        } else if file_type.is_file() {
            WorkspaceFileKind::File
        } else if file_type.is_symlink() {
            WorkspaceFileKind::Symlink
        } else {
            WorkspaceFileKind::Other
        };
        *remaining -= 1;
        let children = if kind == WorkspaceFileKind::Directory && depth > 0 {
            read_workspace_entries(root, path.as_path(), depth - 1, remaining)?
        } else {
            Vec::new()
        };
        nodes.push(WorkspaceFileNode {
            name: entry.file_name().to_string_lossy().to_string(),
            path: path
                .strip_prefix(root)
                .map(workspace_relative_path)
                .unwrap_or_else(|_| path.display().to_string()),
            kind,
            size: (kind == WorkspaceFileKind::File).then_some(metadata.len()),
            children,
        });
    }
    nodes.sort_by(|left, right| {
        let left_dir = left.kind == WorkspaceFileKind::Directory;
        let right_dir = right.kind == WorkspaceFileKind::Directory;
        right_dir
            .cmp(&left_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    Ok(nodes)
}

fn workspace_relative_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            std::path::Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn sanitize_upload_filename(filename: &str) -> String {
    let trimmed = filename.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return String::new();
    }
    // Keep only the final path component so a client-provided path can never
    // smuggle directory traversal into the managed uploads directory.
    let base = trimmed
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(trimmed);
    base.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_' | ' ') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim()
        .to_owned()
}

fn workspace_fs_error(path: &Path, error: io::Error) -> ApplicationError {
    match error.kind() {
        io::ErrorKind::NotFound => ApplicationError::not_found_with_diagnostic(
            "The workspace file was not found.",
            format!("workspace file path not found: {}", path.display()),
        ),
        io::ErrorKind::PermissionDenied => ApplicationError::bad_request_with_diagnostic(
            "The workspace file cannot be read because access was denied.",
            format!("workspace file path cannot be read: {}", path.display()),
        ),
        _ => ApplicationError::internal(format!(
            "workspace file path error for {}: {}",
            path.display(),
            error
        )),
    }
}

fn normalize_workspace_path(workspace_path: &str) -> Result<String, String> {
    let raw = workspace_path.trim();
    if raw.is_empty() {
        return Err("workspace path cannot be empty".to_string());
    }

    let cleaned = Path::new(raw).clean();
    let mut normalized = cleaned.to_string_lossy().replace('\\', "/");
    while normalized.ends_with('/') && normalized.len() > 1 && !is_windows_drive_root(&normalized) {
        normalized.pop();
    }
    if cfg!(windows) {
        normalized.make_ascii_lowercase();
    }
    Ok(normalized)
}

fn is_windows_drive_root(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() == 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use std::path::PathBuf;

    use super::{clean_workspace_relative_path, sanitize_upload_filename};

    #[test]
    fn workspace_file_paths_reject_escape_and_absolute_components() {
        assert_eq!(
            clean_workspace_relative_path(Some("src/./main.rs")).unwrap(),
            PathBuf::from("src/main.rs")
        );
        assert!(clean_workspace_relative_path(Some("../secret")).is_err());
        assert!(clean_workspace_relative_path(Some("src/../../secret")).is_err());
        assert!(clean_workspace_relative_path(Some("/etc/passwd")).is_err());
    }

    #[test]
    fn upload_filenames_strip_directories_and_keep_safe_characters() {
        assert_eq!(sanitize_upload_filename("report.pdf"), "report.pdf");
        assert_eq!(sanitize_upload_filename("docs/../secret.txt"), "secret.txt");
        assert_eq!(sanitize_upload_filename("C:\\Users\\alice\\notes.md"), "notes.md");
        assert_eq!(sanitize_upload_filename("a b+c!.png"), "a b_c_.png");
        assert_eq!(sanitize_upload_filename("  "), "");
        assert_eq!(sanitize_upload_filename("."), "");
        assert_eq!(sanitize_upload_filename(".."), "");
    }
}
use super::{
    ApplicationError, ApplicationResult, ApplicationService, HashMap, PageOrder, PaginatedResponse,
    Path, PathBuf, WorkspaceCursor, WorkspaceFileDownloadQuery, WorkspaceFileKind,
    WorkspaceFileNode, WorkspaceFileTreeQuery, WorkspaceFileTreeResource, WorkspaceFileUploadRequest,
    WorkspaceFileUploadResource, WorkspaceListQuery, WorkspacePathRequest, WorkspaceResolveRequest,
    WorkspaceResource, build_page, decode_cursor, fs, io, non_empty, normalize_limit,
    timestamp_millis_to_utc, trim_page,
};
