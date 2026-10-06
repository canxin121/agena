use agena_storage::WorkspaceListQuery as StorageWorkspaceListQuery;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use path_clean::PathClean;
use uuid::Uuid;

static WORKSPACE_SCANS: agena_async::BlockingPool = agena_async::BlockingPool::new(4);
static WORKSPACE_TRANSFERS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);
static WORKSPACE_IDENTITIES: agena_async::BlockingPool = agena_async::BlockingPool::new(8);

impl ApplicationService {
    /// Metadata-only catalog check; avoids session counts/tree projections.
    pub async fn workspace_revision_rows(&self) -> ApplicationResult<Vec<(i64, i64, String)>> {
        Ok(self
            .workspace_repository
            .list(StorageWorkspaceListQuery {
                limit: i64::MAX as u64,
                ..Default::default()
            })
            .await
            .map_err(|error| ApplicationError::internal_error(&error))?
            .into_iter()
            .map(|row| (row.id, row.updated_at_ms, row.path))
            .collect())
    }

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
        if cursor.is_some() && query.offset > 0 {
            return Err(ApplicationError::bad_request(
                "use either cursor or offset pagination",
            ));
        }
        let rows = self
            .workspace_repository
            .list(StorageWorkspaceListQuery {
                search: non_empty(query.pagination.search()).map(ToString::to_string),
                before_updated_at_ms: cursor.map(|value| value.updated_at_ms),
                before_id: cursor.map(|value| value.id),
                limit: limit + 1,
                offset: query.offset,
            })
            .await
            .map_err(|error| ApplicationError::internal_error(&error))?;
        let (slice, has_more) = trim_page(rows, limit)?;
        let workspace_ids = slice.iter().map(|row| row.id).collect::<Vec<_>>();
        let session_counts = if query.include_session_count {
            self.workspace_session_counts(&workspace_ids).await?
        } else {
            HashMap::new()
        };
        let stats = if query.include_session_count {
            self.session_store
                .workspace_session_stats(&workspace_ids)
                .await
                .map_err(|error| ApplicationError::internal_error(&error))?
        } else {
            HashMap::new()
        };
        let items = slice
            .iter()
            .map(|row| {
                let mut resource =
                    workspace_record_resource(row, session_counts.get(&row.id).copied())?;
                resource.session_stats =
                    stats
                        .get(&row.id)
                        .map(|stats| agena_api::resource::WorkspaceSessionStats {
                            total: stats.total,
                            roots: stats.roots,
                            pinned: stats.pinned,
                            running: stats.running,
                            attention: stats.attention,
                        });
                Ok(resource)
            })
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
            .map_err(|error| ApplicationError::internal_error(&error))?;
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
                .map_err(|error| ApplicationError::internal_error(&error))?
                .ok_or_else(|| {
                    ApplicationError::not_found_with_diagnostic(
                        "The workspace was not found.",
                        format!("workspace not found: {workspace_id}"),
                    )
                })?,
        );
        WORKSPACE_SCANS
            .run(move || list_workspace_files_sync(workspace_id, root, query))
            .await
            .map_err(|error| ApplicationError::internal_error(&error))?
    }

    pub async fn read_workspace_file(
        &self,
        workspace_id: i64,
        query: WorkspaceFileDownloadQuery,
    ) -> ApplicationResult<(String, Vec<u8>)> {
        let root_path = PathBuf::from(
            self.workspace_repository
                .path_by_id(workspace_id)
                .await
                .map_err(|error| ApplicationError::internal_error(&error))?
                .ok_or_else(|| {
                    ApplicationError::not_found_with_diagnostic(
                        "The workspace was not found.",
                        format!("workspace not found: {workspace_id}"),
                    )
                })?,
        );
        WORKSPACE_TRANSFERS
            .run(move || read_workspace_file_sync(root_path, query))
            .await
            .map_err(|error| ApplicationError::internal_error(&error))?
    }

    /// Read a runtime-managed image artifact for a session. These files live
    /// outside the workspace, so the ordinary workspace download endpoint
    /// must not be widened to expose them. The session-to-workspace relation
    /// and the managed artifact root are both resolved on the server.
    pub async fn read_session_media_file(
        &self,
        session_id: i64,
        query: WorkspaceFileDownloadQuery,
    ) -> ApplicationResult<(String, Vec<u8>)> {
        let session = self.ensure_session_model(session_id).await?;
        let workspace_path = self
            .workspace_repository
            .path_by_id(session.workspace_id)
            .await
            .map_err(|error| ApplicationError::internal_error(&error))?
            .ok_or_else(|| {
                ApplicationError::not_found_with_diagnostic(
                    "The workspace was not found.",
                    format!("workspace not found: {}", session.workspace_id),
                )
            })?;
        let managed_root = agena_runtime_tools::project_state_dir(Path::new(&workspace_path))
            .join("generated_images")
            .join(session_id.to_string());
        WORKSPACE_TRANSFERS
            .run(move || read_session_media_file_sync(managed_root, query))
            .await
            .map_err(|error| ApplicationError::internal_error(&error))?
    }

    pub async fn upload_workspace_file(
        &self,
        workspace_id: i64,
        request: WorkspaceFileUploadRequest,
    ) -> ApplicationResult<WorkspaceFileUploadResource> {
        let root_path = PathBuf::from(
            self.workspace_repository
                .path_by_id(workspace_id)
                .await
                .map_err(|error| ApplicationError::internal_error(&error))?
                .ok_or_else(|| {
                    ApplicationError::not_found_with_diagnostic(
                        "The workspace was not found.",
                        format!("workspace not found: {workspace_id}"),
                    )
                })?,
        );
        WORKSPACE_TRANSFERS
            .run(move || upload_workspace_file_sync(workspace_id, root_path, request))
            .await
            .map_err(|error| ApplicationError::internal_error(&error))?
    }

    pub async fn create_workspace(
        &self,
        request: WorkspacePathRequest,
    ) -> ApplicationResult<WorkspaceResource> {
        let path = canonical_workspace_identity_async(request.path).await?;
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
            .map_err(|error| ApplicationError::internal_error(&error))?;

        workspace_record_resource(&created, Some(0))
    }

    pub async fn resolve_workspace(
        &self,
        request: WorkspaceResolveRequest,
    ) -> ApplicationResult<WorkspaceResource> {
        let path = canonical_workspace_identity_async(request.workspace.path).await?;
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
            .map_err(|error| ApplicationError::internal_error(&error))?
        else {
            return Err(ApplicationError::not_found_with_diagnostic(
                "The workspace was not found.",
                format!("workspace not found: {workspace_id}"),
            ));
        };

        let path = canonical_workspace_identity_async(request.path).await?;
        if path == existing.path {
            return self
                .get_workspace(workspace_id)
                .await?
                .ok_or_else(|| ApplicationError::not_found("The workspace was not found."));
        }
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
            .map_err(|error| ApplicationError::internal_error(&error))?
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
            .map_err(|error| ApplicationError::internal_error(&error))?
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
            .map_err(|error| ApplicationError::internal_error(&error))?;
        workspace_record_resource(&existing, counts.get(&workspace_id).copied())
    }
}

fn list_workspace_files_sync(
    workspace_id: i64,
    root: PathBuf,
    query: WorkspaceFileTreeQuery,
) -> ApplicationResult<WorkspaceFileTreeResource> {
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

    // Interactive clients may request a bounded whole-workspace snapshot
    // so their synchronous file pickers never inspect the client host's
    // filesystem. Defaults stay small for ordinary REST callers.
    let depth = query.depth.unwrap_or(2).min(64);
    let mut remaining = query.limit.unwrap_or(500).clamp(1, 50_000);
    let discovery_root = root.clone();
    let entries = if query.respect_ignores {
        read_ignored_workspace_entries(&discovery_root, &target, depth, &mut remaining)?
    } else {
        read_workspace_entries(&discovery_root, &target, depth, &mut remaining, &target)?
    };

    Ok(WorkspaceFileTreeResource {
        workspace_id,
        root: root.display().to_string(),
        path: workspace_relative_path(&relative_path),
        entries,
    })
}

fn read_workspace_file_sync(
    root_path: PathBuf,
    query: WorkspaceFileDownloadQuery,
) -> ApplicationResult<(String, Vec<u8>)> {
    const MAX_DOWNLOAD_BYTES: u64 = 100 * 1024 * 1024;

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

    let bytes = read_bounded_workspace_file(&target, MAX_DOWNLOAD_BYTES)?;
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

fn read_session_media_file_sync(
    managed_root: PathBuf,
    query: WorkspaceFileDownloadQuery,
) -> ApplicationResult<(String, Vec<u8>)> {
    const MAX_SESSION_MEDIA_BYTES: u64 = 20 * 1024 * 1024;

    let managed_root = managed_root
        .canonicalize()
        .map_err(|error| workspace_fs_error(managed_root.as_path(), error))?;
    let requested = PathBuf::from(query.path.trim());
    if requested.as_os_str().is_empty() {
        return Err(ApplicationError::bad_request(
            "session media path is required",
        ));
    }
    let target = if requested.is_absolute() {
        requested
    } else {
        managed_root.join(requested)
    };
    let target = target
        .canonicalize()
        .map_err(|error| workspace_fs_error(target.as_path(), error))?;
    if !target.starts_with(&managed_root) {
        return Err(ApplicationError::bad_request(
            "session media path escapes the managed artifact directory",
        ));
    }
    let metadata = fs::metadata(target.as_path())
        .map_err(|error| workspace_fs_error(target.as_path(), error))?;
    if !metadata.is_file() {
        return Err(ApplicationError::bad_request(
            "the selected session media path is not a regular file",
        ));
    }
    if metadata.len() > MAX_SESSION_MEDIA_BYTES {
        return Err(ApplicationError::bad_request(
            "session media exceeds the 20 MiB presentation limit",
        ));
    }
    let filename = target
        .file_name()
        .and_then(|value| value.to_str())
        .map(sanitize_upload_filename)
        .unwrap_or_else(|| "session-media".to_owned());
    let bytes = read_bounded_workspace_file(&target, MAX_SESSION_MEDIA_BYTES)?;
    Ok((filename, bytes))
}

fn upload_workspace_file_sync(
    workspace_id: i64,
    root_path: PathBuf,
    request: WorkspaceFileUploadRequest,
) -> ApplicationResult<WorkspaceFileUploadResource> {
    const MAX_UPLOAD_BYTES: u64 = 50 * 1024 * 1024;

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

    // Check the encoded budget before allocating the decoded attachment.
    let max_encoded = 4 * MAX_UPLOAD_BYTES.div_ceil(3);
    if request.data_base64.trim().len() as u64 > max_encoded {
        return Err(ApplicationError::bad_request(
            "The uploaded file exceeds the 50 MiB upload limit.",
        ));
    }
    let decoded = BASE64_STANDARD
        .decode(request.data_base64.trim().as_bytes())
        .map_err(|_| {
            ApplicationError::bad_request("The uploaded file data is not valid base64.")
        })?;
    if decoded.is_empty() {
        return Err(ApplicationError::bad_request("The uploaded file is empty."));
    }
    if decoded.len() as u64 > MAX_UPLOAD_BYTES {
        return Err(ApplicationError::bad_request_with_diagnostic(
            "The uploaded file exceeds the 50 MiB upload limit.",
            format!("uploaded file is {} bytes", decoded.len()),
        ));
    }

    let upload_dir = root.join(".agena").join("uploads");
    for directory in [root.join(".agena"), upload_dir.clone()] {
        match fs::symlink_metadata(&directory) {
            Ok(metadata) if !metadata.is_dir() || metadata.file_type().is_symlink() => {
                return Err(ApplicationError::bad_request(
                    "attachment staging directory must be a real workspace directory, not a symlink",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&directory)
                    .map_err(|error| workspace_fs_error(&directory, error))?;
            }
            Err(error) => return Err(workspace_fs_error(&directory, error)),
        }
        if directory
            .canonicalize()
            .map_err(|error| workspace_fs_error(&directory, error))?
            != directory
        {
            return Err(ApplicationError::bad_request(
                "attachment staging directory changed during validation",
            ));
        }
    }
    let stored_name = format!("{}-{}", Uuid::new_v4().simple(), filename);
    let target = upload_dir.join(&stored_name);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(&target)
        .map_err(|error| workspace_fs_error(&target, error))?;
    use std::io::Write as _;
    if let Err(error) = file.write_all(&decoded).and_then(|()| file.sync_all()) {
        let _ = fs::remove_file(&target);
        return Err(workspace_fs_error(&target, error));
    }

    use sha2::Digest as _;
    Ok(WorkspaceFileUploadResource {
        sha256: Some(
            sha2::Sha256::digest(&decoded)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        ),
        workspace_id,
        path: format!(".agena/uploads/{stored_name}"),
        name: filename,
        mime: non_empty(request.mime.as_deref()).map(ToOwned::to_owned),
        size_bytes: decoded.len() as u64,
    })
}

fn read_bounded_workspace_file(path: &Path, limit: u64) -> ApplicationResult<Vec<u8>> {
    use std::io::Read as _;

    let file = fs::File::open(path).map_err(|error| workspace_fs_error(path, error))?;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| workspace_fs_error(path, error))?;
    // A file may grow after the metadata check. Bound the actual read too.
    if bytes.len() as u64 > limit {
        return Err(ApplicationError::bad_request_with_diagnostic(
            "The file exceeds the download limit.",
            format!(
                "file exceeds {limit}-byte download limit: {}",
                path.display()
            ),
        ));
    }
    Ok(bytes)
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
        session_stats: None,
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
    discovery_root: &Path,
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
        let children = if kind == WorkspaceFileKind::Directory
            && depth > 0
            && crate::filesystem_discovery::may_descend(discovery_root, path.as_path())
        {
            read_workspace_entries(root, path.as_path(), depth - 1, remaining, discovery_root)?
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

fn read_ignored_workspace_entries(
    root: &Path,
    dir: &Path,
    depth: usize,
    remaining: &mut usize,
) -> ApplicationResult<Vec<WorkspaceFileNode>> {
    let discovery_root = dir.to_path_buf();
    let mut builder = ignore::WalkBuilder::new(dir);
    builder
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .ignore(true)
        .follow_links(false)
        .parents(true)
        .require_git(false)
        .max_depth(Some(depth.saturating_add(1)))
        .filter_entry(move |entry| {
            !matches!(entry.file_name().to_str(), Some(".git" | ".hg" | ".svn"))
                && crate::filesystem_discovery::may_descend(&discovery_root, entry.path())
        });

    let mut nodes = std::collections::BTreeMap::<String, WorkspaceFileNode>::new();
    let mut children = std::collections::BTreeMap::<String, Vec<String>>::new();
    let mut failure = None;
    crate::filesystem_discovery::visit(&mut builder, |entry| {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                failure = Some(ApplicationError::internal(format!(
                    "workspace ignore-aware walk failed for {}: {error}",
                    dir.display()
                )));
                return ignore::WalkState::Quit;
            }
        };
        if entry.path() == dir {
            return ignore::WalkState::Continue;
        }
        if *remaining == 0 {
            return ignore::WalkState::Quit;
        }
        let metadata = match fs::symlink_metadata(entry.path()) {
            Ok(metadata) => metadata,
            Err(error) => {
                failure = Some(workspace_fs_error(entry.path(), error));
                return ignore::WalkState::Quit;
            }
        };
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
        let path = entry
            .path()
            .strip_prefix(root)
            .map(workspace_relative_path)
            .unwrap_or_else(|_| entry.path().display().to_string());
        let parent = Path::new(path.as_str())
            .parent()
            .map(workspace_relative_path)
            .unwrap_or_default();
        *remaining -= 1;
        children.entry(parent).or_default().push(path.clone());
        nodes.insert(
            path.clone(),
            WorkspaceFileNode {
                name: entry.file_name().to_string_lossy().to_string(),
                path,
                kind,
                size: (kind == WorkspaceFileKind::File).then_some(metadata.len()),
                children: Vec::new(),
            },
        );
        if *remaining == 0 {
            ignore::WalkState::Quit
        } else {
            ignore::WalkState::Continue
        }
    });
    if let Some(error) = failure {
        return Err(error);
    }

    fn assemble(
        parent: &str,
        nodes: &mut std::collections::BTreeMap<String, WorkspaceFileNode>,
        children: &std::collections::BTreeMap<String, Vec<String>>,
    ) -> Vec<WorkspaceFileNode> {
        let mut output = children
            .get(parent)
            .into_iter()
            .flatten()
            .filter_map(|path| {
                let mut node = nodes.remove(path)?;
                node.children = assemble(path, nodes, children);
                Some(node)
            })
            .collect::<Vec<_>>();
        output.sort_by(|left, right| {
            let left_dir = left.kind == WorkspaceFileKind::Directory;
            let right_dir = right.kind == WorkspaceFileKind::Directory;
            right_dir
                .cmp(&left_dir)
                .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
        });
        output
    }

    let base = dir
        .strip_prefix(root)
        .map(workspace_relative_path)
        .unwrap_or_default();
    Ok(assemble(base.as_str(), &mut nodes, &children))
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
    let base = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
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

/// Produce the persistent identity for an existing workspace path.
///
/// Lexical normalization alone treats filesystem aliases such as macOS
/// `/var` and `/private/var` (or an ordinary symlink) as different database
/// workspaces. Canonicalize paths that exist before lookup/create so every
/// client converges on one id. Nonexistent paths retain the stable lexical
/// behavior and can still be registered for later creation.
async fn canonical_workspace_identity_async(path: String) -> ApplicationResult<String> {
    WORKSPACE_IDENTITIES
        .run(move || canonical_workspace_identity(&path))
        .await
        .map_err(|error| ApplicationError::internal_error(&error))?
        .map_err(|error| {
            ApplicationError::bad_request_with_diagnostic("The workspace path is invalid.", error)
        })
}

fn canonical_workspace_identity(workspace_path: &str) -> Result<String, String> {
    let normalized = normalize_workspace_path(workspace_path)?;
    let Ok(canonical) = fs::canonicalize(Path::new(normalized.as_str())) else {
        return Ok(normalized);
    };
    normalize_workspace_path(canonical.to_string_lossy().as_ref())
}

fn is_windows_drive_root(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() == 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use std::path::PathBuf;

    use super::{
        canonical_workspace_identity, clean_workspace_relative_path, sanitize_upload_filename,
    };

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
        assert_eq!(
            sanitize_upload_filename("C:\\Users\\alice\\notes.md"),
            "notes.md"
        );
        assert_eq!(sanitize_upload_filename("a b+c!.png"), "a b_c_.png");
        assert_eq!(sanitize_upload_filename("  "), "");
        assert_eq!(sanitize_upload_filename("."), "");
        assert_eq!(sanitize_upload_filename(".."), "");
    }

    #[cfg(unix)]
    #[test]
    fn existing_workspace_aliases_share_one_canonical_identity() {
        use std::os::unix::fs::symlink;

        let fixture = tempfile::tempdir().expect("create canonical workspace fixture");
        let workspace = fixture.path().join("workspace");
        let alias = fixture.path().join("workspace-alias");
        std::fs::create_dir(&workspace).expect("create canonical workspace");
        symlink(&workspace, &alias).expect("create workspace alias");

        assert_eq!(
            canonical_workspace_identity(workspace.to_string_lossy().as_ref())
                .expect("canonicalize workspace"),
            canonical_workspace_identity(alias.to_string_lossy().as_ref())
                .expect("canonicalize workspace alias")
        );
    }
}
use super::{
    ApplicationError, ApplicationResult, ApplicationService, HashMap, PageOrder, PaginatedResponse,
    Path, PathBuf, WorkspaceCursor, WorkspaceFileDownloadQuery, WorkspaceFileKind,
    WorkspaceFileNode, WorkspaceFileTreeQuery, WorkspaceFileTreeResource,
    WorkspaceFileUploadRequest, WorkspaceFileUploadResource, WorkspaceListQuery,
    WorkspacePathRequest, WorkspaceResolveRequest, WorkspaceResource, build_page, decode_cursor,
    fs, io, non_empty, normalize_limit, timestamp_millis_to_utc, trim_page,
};
