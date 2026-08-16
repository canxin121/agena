use std::{collections::HashSet, ffi::OsStr, path::PathBuf, sync::Arc};

use axum::{
    Json,
    body::{Body, Bytes},
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use tokio::process::Command;

use super::{
    ApiResult, AppError, MAX_FS_LIST_LIMIT, Path, ensure_within_base, has_parent_dir_component,
    home_dir_env, normalize_directory_path, normalize_for_workspace_compare,
    publish_fs_changed_event, resolve_path, to_api_path,
};

pub(crate) async fn validate_directory(candidate: &str) -> ApiResult<PathBuf> {
    let resolved = resolve_path(candidate);
    if resolved.as_os_str().is_empty() {
        return Err(AppError::bad_request("Directory parameter is required"));
    }

    let abs = if resolved.is_absolute() {
        resolved
    } else {
        // Treat relative paths as relative to cwd.
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(resolved)
    };

    let meta = tokio::fs::metadata(&abs)
        .await
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => "Directory not found".to_string(),
            std::io::ErrorKind::PermissionDenied => "Access to directory denied".to_string(),
            _ => "Failed to validate directory".to_string(),
        })
        .map_err(AppError::bad_request)?;
    if !meta.is_dir() {
        return Err(AppError::bad_request("Specified path is not a directory"));
    }
    Ok(abs)
}

#[derive(Debug, Deserialize)]
pub struct ProjectDirQuery {
    pub directory: Option<String>,
}

pub async fn resolve_project_directory(
    _state: &crate::AppState,
    headers: &HeaderMap,
    query_directory: Option<&str>,
) -> ApiResult<PathBuf> {
    let header_directory = headers
        .get("x-agena-directory")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty());

    let requested =
        header_directory.or(query_directory.map(|v| v.trim()).filter(|v| !v.is_empty()));

    if let Some(req) = requested {
        let resolved = validate_directory(req).await?;
        return Ok(resolved);
    }

    Err(AppError::bad_request("Directory parameter is required"))
}

async fn resolve_workspace_path_from_context(
    state: &crate::AppState,
    headers: &HeaderMap,
    query_directory: Option<&str>,
    target: &str,
) -> ApiResult<(PathBuf, PathBuf)> {
    let base = resolve_project_directory(state, headers, query_directory).await?;

    let target_trimmed = target.trim();
    if target_trimmed.is_empty() {
        return Err(AppError::bad_request("Path is required"));
    }

    let normalized = normalize_directory_path(target_trimmed);
    let mut candidate = PathBuf::from(normalized);
    if !candidate.is_absolute() {
        candidate = base.join(candidate);
    }

    // Normalize away '.' segments; reject any '..' segment.
    if has_parent_dir_component(&candidate) {
        return Err(AppError::bad_request(
            "Invalid path: path traversal not allowed",
        ));
    }

    // Important: avoid canonicalize here because the path may not exist yet.
    ensure_within_base(&base, &candidate)?;
    Ok((base, candidate))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FsHomeResponse {
    pub home: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SuccessPathResponse {
    pub success: bool,
    pub path: String,
}

pub async fn fs_home() -> ApiResult<Json<FsHomeResponse>> {
    let home = home_dir_env().unwrap_or_default();
    if home.trim().is_empty() {
        return Err(AppError::internal("Failed to resolve home directory"));
    }
    Ok(Json(FsHomeResponse { home }))
}

#[derive(Debug, Deserialize)]
pub struct MkdirBody {
    pub path: Option<String>,
}

pub async fn fs_mkdir(
    State(state): State<Arc<crate::AppState>>,
    headers: HeaderMap,
    Query(q): Query<ProjectDirQuery>,
    Json(body): Json<MkdirBody>,
) -> ApiResult<Json<SuccessPathResponse>> {
    let dir_path = body
        .path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .ok_or_else(|| AppError::bad_request("Path is required"))?;

    let (base, resolved) = resolve_workspace_path_from_context(
        state.as_ref(),
        &headers,
        q.directory.as_deref(),
        dir_path,
    )
    .await?;

    tokio::fs::create_dir_all(&resolved).await.map_err(|err| {
        if err.kind() == std::io::ErrorKind::PermissionDenied {
            AppError::forbidden("Access denied")
        } else {
            AppError::internal(err.to_string())
        }
    })?;

    publish_fs_changed_event(&base, "mkdir", [resolved.as_path()], None, None);

    Ok(Json(SuccessPathResponse {
        success: true,
        path: to_api_path(&resolved),
    }))
}

#[derive(Debug, Deserialize)]
pub struct ReadQuery {
    pub path: Option<String>,
}

const MAX_READ_BYTES: u64 = 50 * 1024 * 1024;
pub(crate) const MAX_UPLOAD_BYTES: usize = MAX_READ_BYTES as usize;
const DEFAULT_READ_CHUNK_LIMIT: usize = 256 * 1024;
const MAX_READ_CHUNK_LIMIT: usize = 2 * 1024 * 1024;

pub async fn fs_read(Query(q): Query<ReadQuery>) -> ApiResult<Response> {
    let file_path = q.path.unwrap_or_default();
    let file_path = file_path.trim();
    if file_path.is_empty() {
        return Err(AppError::bad_request("Path is required"));
    }

    let resolved = resolve_path(file_path);
    if has_parent_dir_component(&resolved) {
        return Err(AppError::bad_request(
            "Invalid path: path traversal not allowed",
        ));
    }

    let abs = if resolved.is_absolute() {
        resolved
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(resolved)
    };

    let meta = tokio::fs::metadata(&abs)
        .await
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => AppError::not_found("File not found"),
            std::io::ErrorKind::PermissionDenied => AppError::forbidden("Access to file denied"),
            _ => AppError::internal("Failed to read file"),
        })?;

    if !meta.is_file() {
        return Err(AppError::bad_request("Specified path is not a file"));
    }
    if meta.len() > MAX_READ_BYTES {
        return Err(AppError::payload_too_large("File too large"));
    }

    let content = tokio::fs::read_to_string(&abs)
        .await
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => AppError::not_found("File not found"),
            std::io::ErrorKind::PermissionDenied => AppError::forbidden("Access to file denied"),
            _ => AppError::internal(err.to_string()),
        })?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/plain")
        .body(Body::from(content))
        .unwrap())
}

#[derive(Debug, Deserialize)]
pub struct ReadChunkQuery {
    pub path: Option<String>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadChunkResponse {
    pub path: String,
    pub content: String,
    pub offset: usize,
    pub limit: usize,
    pub loaded_bytes: usize,
    pub total_bytes: usize,
    pub has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<usize>,
}

fn decode_utf8_chunk(bytes: &[u8]) -> ApiResult<(String, usize)> {
    if bytes.is_empty() {
        return Ok((String::new(), 0));
    }

    match std::str::from_utf8(bytes) {
        Ok(content) => Ok((content.to_string(), bytes.len())),
        Err(err) => {
            if err.error_len().is_some() {
                return Err(AppError::bad_request("Specified file is not UTF-8 text"));
            }

            let valid_up_to = err.valid_up_to();
            let valid = std::str::from_utf8(&bytes[..valid_up_to])
                .map_err(|_| AppError::bad_request("Specified file is not UTF-8 text"))?;
            Ok((valid.to_string(), valid_up_to))
        }
    }
}

pub async fn fs_read_chunk(Query(q): Query<ReadChunkQuery>) -> ApiResult<Json<ReadChunkResponse>> {
    let file_path = q.path.unwrap_or_default();
    let file_path = file_path.trim();
    if file_path.is_empty() {
        return Err(AppError::bad_request("Path is required"));
    }

    let resolved = resolve_path(file_path);
    if has_parent_dir_component(&resolved) {
        return Err(AppError::bad_request(
            "Invalid path: path traversal not allowed",
        ));
    }

    let abs = if resolved.is_absolute() {
        resolved
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(resolved)
    };

    let meta = tokio::fs::metadata(&abs)
        .await
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => AppError::not_found("File not found"),
            std::io::ErrorKind::PermissionDenied => AppError::forbidden("Access to file denied"),
            _ => AppError::internal("Failed to read file"),
        })?;

    if !meta.is_file() {
        return Err(AppError::bad_request("Specified path is not a file"));
    }
    if meta.len() > MAX_READ_BYTES {
        return Err(AppError::payload_too_large("File too large"));
    }

    let total_bytes_u64 = meta.len();
    let total_bytes = usize::try_from(total_bytes_u64).unwrap_or(usize::MAX);
    let offset = q.offset.unwrap_or(0);
    if (offset as u64) > total_bytes_u64 {
        return Err(AppError::bad_request("Offset is out of range"));
    }

    let limit = q
        .limit
        .unwrap_or(DEFAULT_READ_CHUNK_LIMIT)
        .min(MAX_READ_CHUNK_LIMIT);

    if limit == 0 {
        let has_more = offset < total_bytes;
        return Ok(Json(ReadChunkResponse {
            path: to_api_path(&abs),
            content: String::new(),
            offset,
            limit,
            loaded_bytes: offset,
            total_bytes,
            has_more,
            next_offset: if has_more { Some(offset) } else { None },
        }));
    }

    let mut file = tokio::fs::File::open(&abs)
        .await
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => AppError::not_found("File not found"),
            std::io::ErrorKind::PermissionDenied => AppError::forbidden("Access to file denied"),
            _ => AppError::internal(err.to_string()),
        })?;

    file.seek(SeekFrom::Start(offset as u64))
        .await
        .map_err(|err| AppError::internal(err.to_string()))?;

    let mut buffer = Vec::with_capacity(limit);
    file.take(limit as u64)
        .read_to_end(&mut buffer)
        .await
        .map_err(|err| AppError::internal(err.to_string()))?;

    let (content, consumed_bytes) = decode_utf8_chunk(&buffer)?;
    let loaded_bytes = offset.saturating_add(consumed_bytes);
    let has_more = (loaded_bytes as u64) < total_bytes_u64;

    Ok(Json(ReadChunkResponse {
        path: to_api_path(&abs),
        content,
        offset,
        limit,
        loaded_bytes,
        total_bytes,
        has_more,
        next_offset: if has_more { Some(loaded_bytes) } else { None },
    }))
}

fn mime_for_ext(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "bmp" => "image/bmp",
        "avif" => "image/avif",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "flac" => "audio/flac",
        "opus" => "audio/opus",
        "weba" => "audio/webm",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "ogv" => "video/ogg",
        "mov" => "video/quicktime",
        "m4v" => "video/x-m4v",
        "mkv" => "video/x-matroska",
        _ => "application/octet-stream",
    }
}

pub async fn fs_raw(Query(q): Query<ReadQuery>) -> ApiResult<Response> {
    let file_path = q.path.unwrap_or_default();
    let file_path = file_path.trim();
    if file_path.is_empty() {
        return Err(AppError::bad_request("Path is required"));
    }

    let resolved = resolve_path(file_path);
    if has_parent_dir_component(&resolved) {
        return Err(AppError::bad_request(
            "Invalid path: path traversal not allowed",
        ));
    }

    let abs = if resolved.is_absolute() {
        resolved
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(resolved)
    };

    let meta = tokio::fs::metadata(&abs)
        .await
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => AppError::not_found("File not found"),
            std::io::ErrorKind::PermissionDenied => AppError::forbidden("Access to file denied"),
            _ => AppError::internal("Failed to read file"),
        })?;

    if !meta.is_file() {
        return Err(AppError::bad_request("Specified path is not a file"));
    }
    if meta.len() > MAX_READ_BYTES {
        return Err(AppError::payload_too_large("File too large"));
    }

    let mime = mime_for_ext(&abs);
    let content = tokio::fs::read(&abs)
        .await
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => AppError::not_found("File not found"),
            std::io::ErrorKind::PermissionDenied => AppError::forbidden("Access to file denied"),
            _ => AppError::internal(err.to_string()),
        })?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("cache-control", "no-store")
        .header("content-type", mime)
        .header("content-disposition", content_disposition_inline(&abs))
        .body(Body::from(content))
        .unwrap())
}

#[derive(Debug, Deserialize)]
pub struct FsPathQuery {
    pub directory: Option<String>,
    pub path: Option<String>,
}

fn content_disposition_for(path: &Path, disposition_type: &str) -> String {
    let raw = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "download".to_string());

    // RFC 6266: provide both ASCII fallback and UTF-8 filename*.
    let mut ascii = String::with_capacity(raw.len());
    for ch in raw.chars() {
        let ok = ch.is_ascii() && !matches!(ch, '"' | '\\') && !ch.is_ascii_control();
        ascii.push(if ok { ch } else { '_' });
    }
    if ascii.trim().is_empty() {
        ascii = "download".to_string();
    }

    let encoded = urlencoding::encode(&raw);
    format!(
        "{}; filename=\"{}\"; filename*=UTF-8''{}",
        disposition_type, ascii, encoded
    )
}

fn content_disposition_attachment(path: &Path) -> String {
    content_disposition_for(path, "attachment")
}

fn content_disposition_inline(path: &Path) -> String {
    content_disposition_for(path, "inline")
}

pub async fn fs_download(
    State(state): State<Arc<crate::AppState>>,
    headers: HeaderMap,
    Query(q): Query<FsPathQuery>,
) -> ApiResult<Response> {
    let file_path = q
        .path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .ok_or_else(|| AppError::bad_request("Path is required"))?;

    let (_, abs) = resolve_workspace_path_from_context(
        state.as_ref(),
        &headers,
        q.directory.as_deref(),
        file_path,
    )
    .await?;

    let meta = tokio::fs::metadata(&abs)
        .await
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => AppError::not_found("File not found"),
            std::io::ErrorKind::PermissionDenied => AppError::forbidden("Access to file denied"),
            _ => AppError::internal("Failed to read file"),
        })?;

    if !meta.is_file() {
        return Err(AppError::bad_request("Specified path is not a file"));
    }
    if meta.len() > MAX_READ_BYTES {
        return Err(AppError::payload_too_large("File too large"));
    }

    let mime = mime_for_ext(&abs);
    let content = tokio::fs::read(&abs)
        .await
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => AppError::not_found("File not found"),
            std::io::ErrorKind::PermissionDenied => AppError::forbidden("Access to file denied"),
            _ => AppError::internal(err.to_string()),
        })?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("cache-control", "no-store")
        .header("content-type", mime)
        .header("content-disposition", content_disposition_attachment(&abs))
        .body(Body::from(content))
        .unwrap())
}

#[derive(Debug, Deserialize)]
pub struct UploadQuery {
    pub directory: Option<String>,
    pub path: Option<String>,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Serialize)]
pub struct UploadResponse {
    pub success: bool,
    pub path: String,
    pub bytes: usize,
}

pub async fn fs_upload(
    State(state): State<Arc<crate::AppState>>,
    headers: HeaderMap,
    Query(q): Query<UploadQuery>,
    payload: Bytes,
) -> ApiResult<Json<UploadResponse>> {
    let file_path = q
        .path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .ok_or_else(|| AppError::bad_request("Path is required"))?;

    let bytes_len = payload.len();
    if bytes_len > MAX_UPLOAD_BYTES {
        return Err(AppError::payload_too_large("File too large"));
    }

    let (base, resolved) = resolve_workspace_path_from_context(
        state.as_ref(),
        &headers,
        q.directory.as_deref(),
        file_path,
    )
    .await?;

    if !q.overwrite {
        match tokio::fs::symlink_metadata(&resolved).await {
            Ok(meta) => {
                if meta.is_dir() {
                    return Err(AppError::bad_request("Target path is a directory"));
                }
                return Err(AppError::bad_request("File already exists"));
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
                return Err(AppError::forbidden("Access denied"));
            }
            Err(err) => return Err(AppError::internal(err.to_string())),
        }
    }

    if let Some(parent) = resolved.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|err| {
            if err.kind() == std::io::ErrorKind::PermissionDenied {
                AppError::forbidden("Access denied")
            } else {
                AppError::internal(err.to_string())
            }
        })?;
    }

    tokio::fs::write(&resolved, payload.as_ref())
        .await
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::PermissionDenied => AppError::forbidden("Access denied"),
            std::io::ErrorKind::IsADirectory => AppError::bad_request("Target path is a directory"),
            _ => AppError::internal(err.to_string()),
        })?;

    publish_fs_changed_event(&base, "upload", [resolved.as_path()], None, None);

    Ok(Json(UploadResponse {
        success: true,
        path: to_api_path(&resolved),
        bytes: bytes_len,
    }))
}

#[derive(Debug, Deserialize)]
pub struct WriteBody {
    pub path: Option<String>,
    pub content: Option<String>,
}

pub async fn fs_write(
    State(state): State<Arc<crate::AppState>>,
    headers: HeaderMap,
    Query(q): Query<ProjectDirQuery>,
    Json(body): Json<WriteBody>,
) -> ApiResult<Json<SuccessPathResponse>> {
    let file_path = body
        .path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .ok_or_else(|| AppError::bad_request("Path is required"))?;
    let content = body
        .content
        .ok_or_else(|| AppError::bad_request("Content is required"))?;

    if content.len() as u64 > MAX_READ_BYTES {
        return Err(AppError::payload_too_large("Content too large"));
    }

    let (base, resolved) = resolve_workspace_path_from_context(
        state.as_ref(),
        &headers,
        q.directory.as_deref(),
        file_path,
    )
    .await?;

    if let Some(parent) = resolved.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|err| {
            if err.kind() == std::io::ErrorKind::PermissionDenied {
                AppError::forbidden("Access denied")
            } else {
                AppError::internal(err.to_string())
            }
        })?;
    }

    tokio::fs::write(&resolved, content).await.map_err(|err| {
        if err.kind() == std::io::ErrorKind::PermissionDenied {
            AppError::forbidden("Access denied")
        } else {
            AppError::internal(err.to_string())
        }
    })?;

    publish_fs_changed_event(&base, "write", [resolved.as_path()], None, None);

    Ok(Json(SuccessPathResponse {
        success: true,
        path: to_api_path(&resolved),
    }))
}

#[derive(Debug, Deserialize)]
pub struct DeleteBody {
    pub path: Option<String>,
}

pub async fn fs_delete(
    State(state): State<Arc<crate::AppState>>,
    headers: HeaderMap,
    Query(q): Query<ProjectDirQuery>,
    Json(body): Json<DeleteBody>,
) -> ApiResult<Json<SuccessPathResponse>> {
    let target_path = body
        .path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .ok_or_else(|| AppError::bad_request("Path is required"))?;

    let (base, resolved) = resolve_workspace_path_from_context(
        state.as_ref(),
        &headers,
        q.directory.as_deref(),
        target_path,
    )
    .await?;

    let meta = match tokio::fs::symlink_metadata(&resolved).await {
        Ok(m) => Some(m),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            return Err(AppError::forbidden("Access denied"));
        }
        Err(err) => return Err(AppError::internal(err.to_string())),
    };

    if meta.is_none() {
        // Match client "force: true" behavior.
        return Ok(Json(SuccessPathResponse {
            success: true,
            path: to_api_path(&resolved),
        }));
    }

    let meta = meta.unwrap();
    if meta.is_dir() {
        tokio::fs::remove_dir_all(&resolved).await
    } else {
        tokio::fs::remove_file(&resolved).await
    }
    .map_err(|err| {
        if err.kind() == std::io::ErrorKind::PermissionDenied {
            AppError::forbidden("Access denied")
        } else {
            AppError::internal(err.to_string())
        }
    })?;

    publish_fs_changed_event(&base, "delete", [resolved.as_path()], None, None);

    Ok(Json(SuccessPathResponse {
        success: true,
        path: to_api_path(&resolved),
    }))
}

#[derive(Debug, Deserialize)]
pub struct RenameBody {
    #[serde(rename = "oldPath")]
    pub old_path: Option<String>,
    #[serde(rename = "newPath")]
    pub new_path: Option<String>,
}

pub async fn fs_rename(
    State(state): State<Arc<crate::AppState>>,
    headers: HeaderMap,
    Query(q): Query<ProjectDirQuery>,
    Json(body): Json<RenameBody>,
) -> ApiResult<Json<SuccessPathResponse>> {
    let old_path = body
        .old_path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .ok_or_else(|| AppError::bad_request("oldPath is required"))?;
    let new_path = body
        .new_path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .ok_or_else(|| AppError::bad_request("newPath is required"))?;

    let (base_old, resolved_old) = resolve_workspace_path_from_context(
        state.as_ref(),
        &headers,
        q.directory.as_deref(),
        old_path,
    )
    .await?;
    let (base_new, resolved_new) = resolve_workspace_path_from_context(
        state.as_ref(),
        &headers,
        q.directory.as_deref(),
        new_path,
    )
    .await?;

    if normalize_for_workspace_compare(&base_old) != normalize_for_workspace_compare(&base_new) {
        return Err(AppError::bad_request(
            "Source and destination must share the same workspace root",
        ));
    }

    tokio::fs::rename(&resolved_old, &resolved_new)
        .await
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => AppError::not_found("Source path not found"),
            std::io::ErrorKind::PermissionDenied => AppError::forbidden("Access denied"),
            _ => AppError::internal(err.to_string()),
        })?;

    publish_fs_changed_event(
        &base_old,
        "rename",
        [resolved_old.as_path(), resolved_new.as_path()],
        Some(&resolved_old),
        Some(&resolved_new),
    );

    Ok(Json(SuccessPathResponse {
        success: true,
        path: to_api_path(&resolved_new),
    }))
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub path: Option<String>,
    #[serde(default, rename = "respectGitignore")]
    pub respect_gitignore: bool,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ListItem {
    pub name: String,
    pub path: String,
    pub is_directory: bool,
    pub is_file: bool,
    pub is_symbolic_link: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListResponse {
    pub path: String,
    pub entries: Vec<ListItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    pub total: usize,
    pub has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<usize>,
}

async fn git_check_ignore(dir: &Path, names: &[String]) -> HashSet<String> {
    if names.is_empty() {
        return HashSet::new();
    }

    let mut cmd = Command::new("git");
    cmd.arg("check-ignore").arg("--");
    for n in names {
        cmd.arg(n);
    }
    cmd.current_dir(dir);
    cmd.stdin(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.kill_on_drop(true);

    let out = match tokio::time::timeout(std::time::Duration::from_secs(10), cmd.output()).await {
        Ok(Ok(out)) if out.status.success() => out,
        _ => return HashSet::new(),
    };

    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

pub async fn fs_list(Query(q): Query<ListQuery>) -> ApiResult<Json<ListResponse>> {
    let raw_path = q
        .path
        .as_deref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .unwrap_or_else(|| home_dir_env().unwrap_or_default());
    let resolved = resolve_path(&raw_path);
    let abs = if resolved.is_absolute() {
        resolved
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(resolved)
    };

    let meta = tokio::fs::metadata(&abs)
        .await
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => AppError::not_found("Directory not found"),
            std::io::ErrorKind::PermissionDenied => {
                AppError::forbidden("Access to directory denied")
            }
            _ => AppError::internal("Failed to list directory"),
        })?;
    if !meta.is_dir() {
        return Err(AppError::bad_request("Specified path is not a directory"));
    }

    let mut rd = tokio::fs::read_dir(&abs)
        .await
        .map_err(|err| AppError::internal(err.to_string()))?;

    let mut raw_entries = Vec::new();
    let mut names = Vec::new();
    while let Ok(Some(entry)) = rd.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        names.push(name.clone());
        raw_entries.push(entry);
    }

    let ignored = if q.respect_gitignore {
        git_check_ignore(&abs, &names).await
    } else {
        HashSet::new()
    };

    let mut entries = Vec::new();
    for entry in raw_entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if q.respect_gitignore && ignored.contains(&name) {
            continue;
        }

        let path = entry.path();
        let ft = match entry.file_type().await {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        let is_symbolic_link = ft.is_symlink();
        let mut is_directory = ft.is_dir();
        if !is_directory
            && is_symbolic_link
            && let Ok(st) = tokio::fs::metadata(&path).await
        {
            is_directory = st.is_dir();
        }

        entries.push(ListItem {
            name,
            path: to_api_path(&path),
            is_directory,
            is_file: ft.is_file(),
            is_symbolic_link,
        });
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));

    let total = entries.len();
    let offset = q.offset.unwrap_or(0).min(total);
    let limit = q
        .limit
        .map(|v| v.clamp(1, MAX_FS_LIST_LIMIT))
        .filter(|v| *v > 0);

    let (page_entries, has_more, next_offset) = if let Some(limit) = limit {
        let end = offset.saturating_add(limit).min(total);
        let has_more = end < total;
        let next_offset = if has_more { Some(end) } else { None };
        (entries[offset..end].to_vec(), has_more, next_offset)
    } else if offset > 0 {
        (entries[offset..].to_vec(), false, None)
    } else {
        (entries, false, None)
    };

    Ok(Json(ListResponse {
        path: to_api_path(&abs),
        entries: page_entries,
        offset: if q.limit.is_some() || offset > 0 {
            Some(offset)
        } else {
            None
        },
        limit,
        total,
        has_more,
        next_offset,
    }))
}
