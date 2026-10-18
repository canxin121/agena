//! Tool terminal values contain bounded facts and immutable content/blob
//! references. Preparing this value never invokes the external effect again.
use super::*;
use crate::session::kernel::media::{MediaBinding, uri};
use agena_domain::{AttachmentSource, ContentField, ContentFormat, ContentKind, ContentRef};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

impl SessionManager {
    pub(super) async fn prepare_tool_result(
        &self,
        tool: &ToolHandle,
        session: SessionId,
        model: &ModelRef,
        providers: &ProviderRegistry,
        executor: &ToolExecutor,
        invocation: &agena_domain::ToolInvocation,
        execution: &ToolInvocationExecution,
    ) -> Result<(Value, Vec<PreparedBlob>), AppError> {
        let mut raw = execution.view.raw_output(&execution.output);
        let mut blobs = Vec::new();
        let mut media_bytes = 0usize;
        if raw.attachments.len() > 8 {
            return Err(AppError::Config(
                "tool result contains more than eight attachments".into(),
            ));
        }
        let route = providers.media_route_binding(model)?;
        let _preparation = if raw.attachments.is_empty() {
            None
        } else {
            Some(
                super::media::MEDIA_PREPARATIONS
                    .acquire()
                    .await
                    .map_err(|_| AppError::Cancelled)?,
            )
        };
        for (index, attachment) in raw.attachments.iter_mut().enumerate() {
            let bytes = match &attachment.source {
                AttachmentSource::Base64 { data } | AttachmentSource::ProviderData { data, .. } => {
                    if let AttachmentSource::ProviderData { route: bound, .. } = &attachment.source
                        && bound != &route
                    {
                        return Err(AppError::Config(
                            "tool media is bound to a different provider route".into(),
                        ));
                    }
                    if data.len() > 28 * 1024 * 1024 {
                        return Err(AppError::Config(
                            "tool media exceeds its 20 MiB byte budget".into(),
                        ));
                    }
                    Some(
                        STANDARD
                            .decode(data)
                            .map_err(|error| AppError::internal_error(&error))?,
                    )
                }
                AttachmentSource::DataUrl { url } => {
                    let (header, data) = url.split_once(',').ok_or_else(|| {
                        AppError::Config("tool returned an invalid media data URL".into())
                    })?;
                    if !header.ends_with(";base64") || data.len() > 28 * 1024 * 1024 {
                        return Err(AppError::Config(
                            "tool media must use a bounded base64 data URL".into(),
                        ));
                    }
                    Some(
                        STANDARD
                            .decode(data)
                            .map_err(|error| AppError::internal_error(&error))?,
                    )
                }
                AttachmentSource::LocalPath { path } => {
                    let root = executor
                        .workspace_root()
                        .canonicalize()
                        .map_err(|error| AppError::internal_error(&error))?;
                    let target = root
                        .join(path)
                        .canonicalize()
                        .map_err(|error| AppError::internal_error(&error))?;
                    if !target.starts_with(&root)
                        || !matches!(
                            executor.principal().authorize_path_access(
                                agena_domain::AccessKind::Read,
                                &root,
                                &target
                            ),
                            agena_domain::PermissionDecision::Allow
                        )
                    {
                        None
                    } else {
                        let path = path.clone();
                        let expected = attachment.sha256.clone();
                        Some(
                            super::media::MEDIA_WORKERS
                                .run(move || {
                                    agena_runtime_tools::media_input::read_local(
                                        &root,
                                        &path,
                                        expected.as_deref(),
                                    )
                                })
                                .await
                                .map_err(|error| AppError::internal_error(&error))?
                                .map_err(AppError::Config)?
                                .bytes,
                        )
                    }
                }
                AttachmentSource::Url { url } => {
                    if url.len() > 16 * 1024 {
                        return Err(AppError::Config(
                            "tool attachment URL exceeds its metadata budget".into(),
                        ));
                    }
                    None
                }
                AttachmentSource::FileId { file_id } => {
                    if file_id.len() > 16 * 1024 {
                        return Err(AppError::Config(
                            "tool attachment file ID exceeds its metadata budget".into(),
                        ));
                    }
                    None
                }
            };
            let Some(bytes) = bytes else {
                continue;
            };
            media_bytes = media_bytes.saturating_add(bytes.len());
            if bytes.len() > agena_runtime_tools::media_input::MAX_MEDIA_BYTES
                || media_bytes > agena_runtime_tools::media_input::MAX_MEDIA_BATCH_BYTES
            {
                return Err(AppError::Config(
                    "tool media exceeds its immutable attachment budget".into(),
                ));
            }
            let hash = hex::encode(Sha256::digest(&bytes));
            let command = request_key("tool-attachment", &format!("{}:{index}", tool.id().get()));
            let blob = self
                .prepare_immutable_blob(command, session, &bytes)
                .await?;
            let mut metadata = attachment.clone();
            metadata.source = AttachmentSource::Base64 {
                data: String::new(),
            };
            metadata.size_bytes = Some(bytes.len() as u64);
            metadata.sha256 = Some(hash.clone());
            let binding = MediaBinding {
                version: 1,
                model: model.clone(),
                route: route.clone(),
                blob: blob.descriptor().id,
                attachment: metadata,
            };
            attachment.source = AttachmentSource::Url {
                url: uri(&binding).map_err(kernel_error)?,
            };
            attachment.size_bytes = Some(bytes.len() as u64);
            attachment.sha256 = Some(hash);
            blobs.push(blob);
        }
        // A large structured result is stored exactly once as a JSON text
        // source. The row contains its pointer and clients read bounded pages.
        if let Some(payload) = &raw.payload {
            let bytes = bounded_json(payload, 64 * 1024 * 1024)?;
            if bytes.len() > 64 * 1024 {
                let text =
                    String::from_utf8(bytes).map_err(|error| AppError::internal_error(&error))?;
                let command = request_key("tool-result-json", &tool.id().get().to_string());
                let source = persist(|| tool.begin_text(command))
                    .await
                    .map_err(kernel_error)?
                    .value;
                append_text_chunks(&source, &text)
                    .await
                    .map_err(kernel_error)?;
                raw.fields.push(ContentField {
                    pointer: String::new(),
                    resource: ContentRef {
                        resource_id: source.id(),
                        kind: ContentKind::Text,
                    },
                    format: ContentFormat::Json,
                });
                raw.payload = None;
            }
        }
        let value = json!({"raw_output":raw,"title":execution.view.title,"summary":execution.view.summary,"metadata":execution.view.metadata,"tool_api_call":invocation.tool_api_call,"invocation":invocation});
        if bounded_json(&value, 768 * 1024).is_err() {
            return Err(AppError::Config(
                "tool result metadata exceeds its storage budget".into(),
            ));
        }
        Ok((value, blobs))
    }
    pub(super) async fn prepare_immutable_blob(
        &self,
        command: RequestId,
        session: SessionId,
        bytes: &[u8],
    ) -> Result<PreparedBlob, AppError> {
        let mut wait = Duration::from_millis(25);
        loop {
            let mut input = std::io::Cursor::new(bytes);
            match self
                .kernel()
                .sessions
                .prepare_blob(command, session, bytes.len() as u64, &mut input)
                .await
            {
                Ok(blob) => return Ok(blob),
                Err(error) if transient(&error) => {
                    tokio::time::sleep(wait).await;
                    wait = (wait * 2).min(Duration::from_secs(1));
                }
                Err(error) => return Err(kernel_error(error)),
            }
        }
    }
}
pub(super) async fn append_text_chunks(
    source: &TextOutput,
    text: &str,
) -> agena_storage::kernel::Result<()> {
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + 64 * 1024).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        loop {
            match source.append(&text[start..end]).await {
                Ok(_) => break,
                Err(KernelError::Busy) => tokio::time::sleep(Duration::from_millis(10)).await,
                Err(error) => return Err(error),
            }
        }
        start = end;
    }
    Ok(())
}
fn bounded_json(value: &impl serde::Serialize, limit: usize) -> Result<Vec<u8>, AppError> {
    struct Writer {
        bytes: Vec<u8>,
        limit: usize,
    }
    impl std::io::Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.bytes.len().saturating_add(bytes.len()) > self.limit {
                return Err(std::io::Error::other("tool result exceeds its byte budget"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Writer {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut writer, value)?;
    Ok(writer.bytes)
}
