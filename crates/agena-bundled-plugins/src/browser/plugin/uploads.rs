use super::*;
use agena_plugin_host::sdk::host_api::PathPermissionQuery;
use serde_json::json;

const MAX_UPLOAD_BYTES: u64 = 100 * 1024 * 1024;
const MAX_PAGE_UPLOAD_BYTES: u64 = 2 * MAX_UPLOAD_BYTES;
const MAX_PAGE_UPLOAD_FILES: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
#[input(
    trim("session_id", "selector", "frame_id"),
    non_empty("session_id", "selector"),
    non_empty_if_present("frame_id")
)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserUploadInput {
    session_id: String,
    /// CSS or @eN selector for an input of type file.
    #[arg(max_chars = 4096)]
    selector: String,
    /// Workspace-relative or absolute paths, including local attachment paths.
    #[schemars(length(min = 1, max = 16))]
    files: Vec<String>,
    /// Discover frame_id with browser.frames.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 128)]
    frame_id: Option<String>,
    #[serde(default = "default_return_snapshot")]
    return_snapshot: bool,
    #[serde(default = "default_browser_action_timeout_ms")]
    #[schemars(range(min = 1, max = 120000))]
    timeout_ms: u64,
}

#[derive(Default)]
pub(super) struct UploadStore {
    dirs: Vec<tempfile::TempDir>,
    bytes: u64,
    files: usize,
}

impl BrowserPlugin {
    pub(super) async fn upload_browser_files(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserUploadInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let access = self.browser_access(context, &input.session_id).await?;
        let mut sources = Vec::new();
        let host = self.state()?.host.as_ref();
        for path in &input.files {
            let path = path.trim();
            if path.is_empty() || path.len() > 4096 {
                return Err(PluginError::invalid_params(
                    "Each file requires a nonempty path",
                ));
            }
            let path = Path::new(path);
            let absolute = if path.is_absolute() {
                path.to_owned()
            } else if context.is_global() {
                return Err(PluginError::invalid_params(
                    "Global sessions require absolute upload paths",
                ));
            } else {
                self.workspace_root()?.join(path)
            };
            let authorized = absolute
                .to_str()
                .ok_or_else(|| PluginError::invalid_params("Upload paths must be UTF-8"))?;
            host.require_path_permission(PathPermissionQuery::read(authorized))
                .await?;
            sources.push(absolute);
        }
        let maximum = {
            let store = access.page.uploads.lock().await;
            if store.files + sources.len() > MAX_PAGE_UPLOAD_FILES {
                return Err(PluginError::invalid_params(
                    "This page's staged file limit was reached. Open another page to continue.",
                ));
            }
            MAX_UPLOAD_BYTES.min(MAX_PAGE_UPLOAD_BYTES.saturating_sub(store.bytes))
        };
        let (directory, files, size) = stage_files(sources, maximum).await?;
        {
            // Chromium reads input files when the page uses them, which can be
            // after this tool call has returned.
            let mut store = access.page.uploads.lock().await;
            store.bytes += size;
            store.files += files.len();
            store.dirs.push(directory);
        }
        self.run_browser_command(
            &access, "upload",
            json!({"action": "upload", "selector": input.selector, "files": files, "frameId": input.frame_id}),
            input.timeout_ms, input.return_snapshot,
        ).await
    }
}

async fn stage_files(
    paths: Vec<PathBuf>,
    byte_limit: u64,
) -> SdkResult<(tempfile::TempDir, Vec<PathBuf>, u64)> {
    let dir = tempfile::Builder::new()
        .prefix("agena-browser-upload-")
        .tempdir()
        .map_err(|error| PluginError::internal_error(&error))?;
    let policy = agena_process::sandbox::SandboxPolicy {
        disabled: true,
        ..Default::default()
    };
    let mut size = 0_u64;
    let mut staged = Vec::with_capacity(paths.len());
    for (index, path) in paths.iter().enumerate() {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| PluginError::invalid_params("Each upload must identify a named file"))?;
        let contents = agena_process::sandbox_file::read(path, byte_limit - size, None, &policy)
            .await
            .map_err(|error| {
                plugin_internal_error_with_context("cannot stage browser upload", &error)
            })?;
        size += contents.len() as u64;
        let parent = dir.path().join(index.to_string());
        tokio::fs::create_dir(&parent)
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        let destination = parent.join(name);
        tokio::fs::write(&destination, contents)
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        staged.push(destination);
    }
    Ok((dir, staged, size))
}
