//! Snapshot local bytes and return them to the current conversation model.
//! The session installs the route binder; the tool cannot choose a provider.

use std::sync::Arc;

use super::{
    ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput,
    payload::ReadAttachmentOutput,
};
use crate::{media_input, part::ReadMediaInput};

static MEDIA_READ_WORKERS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

impl ToolExecutor {
    pub fn with_media_input_binder(
        mut self,
        binder: Arc<super::tool_registry::MediaInputBinder>,
    ) -> Self {
        self.media_input_binder = Some(binder);
        self
    }
}

pub(super) async fn execute(
    executor: &ToolExecutor,
    input: ReadMediaInput,
) -> Result<ToolPayloadExecution, ToolError> {
    executor.ensure_not_cancelled()?;
    let binder = executor.media_input_binder.clone().ok_or_else(|| {
        ToolError::invalid_input("fs.read_media requires an active conversation model route")
    })?;
    let root = executor.workspace_root().to_path_buf();
    let display_path = executor.display_path(&executor.resolve_target_path(&input.path));
    let (prepared, item) = MEDIA_READ_WORKERS
        .run(move || {
            let prepared =
                media_input::read_local(&root, &input.path, input.expected_sha256.as_deref())
                    .map_err(ToolError::invalid_input)?;
            let item = binder(prepared.attachment(None))?;
            Ok::<_, ToolError>((prepared, item))
        })
        .await
        .map_err(|error| ToolError::plugin(format!("media reader failed: {error}")))??;
    executor.ensure_not_cancelled()?;
    let info = serde_json::json!({
        "path": display_path,
        "kind": prepared.kind,
        "mime": prepared.mime,
        "size_bytes": prepared.size_bytes,
        "sha256": prepared.sha256,
        "width": prepared.width,
        "height": prepared.height,
        "delivery": "model_input",
    });
    let mut view = ToolExecutionView::simple(
        format!("Read media {display_path}"),
        format!("{} · {} bytes", prepared.mime, prepared.size_bytes),
        format!(
            "Read {display_path} as {} ({} bytes, sha256={}). The file's actual contents are attached to this tool result for the current model. Analyze the attached content; do not infer it from the filename.",
            prepared.mime, prepared.size_bytes, prepared.sha256
        ),
    );
    view.metadata
        .insert("delivery".into(), "model_input".into());
    view.attachments.push(item);
    Ok(ToolPayloadExecution::new(
        ToolPayloadOutput::Read {
            read_info: Some(info),
            preview: None,
            truncated: false,
            loaded_paths: vec![display_path.clone()],
            attachment: Some(ReadAttachmentOutput {
                path: display_path,
                kind: prepared.kind,
                mime: prepared.mime,
                size_bytes: prepared.size_bytes,
                filename: Some(prepared.filename),
                width: prepared.width,
                height: prepared.height,
                duration_ms: None,
                page_count: None,
            }),
        },
        view,
    ))
}
