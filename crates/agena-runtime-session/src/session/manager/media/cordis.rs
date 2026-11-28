use super::*;
use agena_domain::{
    ActivityProvenance, ComposerActivity, ResourceActivity, ResourceKind, safe_attachment_filename,
};
use agena_plugin_host::agent_plugin::CordisFollowupBlock;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Default)]
pub(in super::super) struct CordisAttachments(Vec<CordisAttachment>);

struct CordisAttachment {
    resource: ResourceActivity,
    provenance: ActivityProvenance,
    data: AttachmentData,
}

enum AttachmentData {
    Image(UserMediaSnapshot),
    File { bytes: Arc<Vec<u8>>, sha256: String },
}

pub(in super::super) struct CordisFollowup {
    pub document: ComposerDocument,
    pub images: UserMediaSnapshots,
    files: std::collections::BTreeSet<usize>,
}

impl CordisAttachments {
    pub(in super::super) async fn admit(
        manager: &SessionManager,
        session: SessionId,
        state: &ExecutionState,
        context: &SessionExecutionContext,
        document: ComposerDocument,
    ) -> Result<Self, AppError> {
        let mut attachments = Self::default();
        let mut batch_bytes = 0;
        let access = UserMediaAccess::new(session, state, context)?;
        for node in document.0 {
            let ComposerNode::Activity { activity } = node else {
                continue;
            };
            let ComposerActivity {
                payload,
                mut provenance,
                ..
            } = *activity;
            let ActivityPayload::Resource(mut resource) = payload else {
                return Err(attachment_required());
            };
            if resource.kind == ResourceKind::Directory {
                return Err(attachment_required());
            }
            if attachments.0.len() >= media_input::MAX_MEDIA_INPUTS {
                return Err(AppError::Config(
                    "A command can receive at most eight attachments".into(),
                ));
            }
            let (bytes, blob) = match &resource.reference {
                ResourceReference::WorkspacePath { path } => {
                    let expected = provenance.content_hash.as_deref();
                    let target = access.workspace_path(path, expected)?;
                    let bytes = MEDIA_WORKERS
                        .run(move || media_input::read_local_bytes(&target))
                        .await
                        .map_err(|error| AppError::internal_error(&error))?
                        .map_err(AppError::Config)?;
                    (bytes, None)
                }
                ResourceReference::Artifact { uri, sha256 } => {
                    let reader = manager.open_selected_artifact(session, uri, sha256).await?;
                    let mut bytes = Vec::with_capacity(reader.descriptor().byte_len as usize);
                    reader.copy_to(&mut bytes).await.map_err(kernel_error)?;
                    (bytes, Some(reader.into_prepared()))
                }
                _ => return Err(attachment_required()),
            };
            batch_bytes += bytes.len();
            if batch_bytes > media_input::MAX_MEDIA_BATCH_BYTES {
                return Err(AppError::Config(
                    "Command attachments exceed the 40 MiB input limit".into(),
                ));
            }
            resource.name = safe_attachment_filename(&resource.name);
            if resource.name.is_empty() {
                resource.name = "attachment".into();
            }
            let name = resource.name.clone();
            let expect_image = resource.kind == ResourceKind::Image
                && resource.delivery == ResourceDelivery::ModelInput;
            let expected = provenance.content_hash.clone();
            let data = MEDIA_WORKERS
                .run(move || {
                    let data = if expect_image || media_input::raster_mime(&bytes).is_some() {
                        let media = media_input::from_bytes(&name, bytes, None)?;
                        if media.kind != agena_domain::AttachmentKind::Image {
                            return Err("The selected image has an unsupported format".into());
                        }
                        AttachmentData::Image(UserMediaSnapshot {
                            media: Arc::new(media),
                            blob,
                        })
                    } else {
                        let sha256 = hex::encode(Sha256::digest(&bytes));
                        AttachmentData::File {
                            bytes: Arc::new(bytes),
                            sha256,
                        }
                    };
                    if !media_input::matches_revision(data.sha256(), expected.as_deref()) {
                        return Err("The attachment changed since it was selected".into());
                    }
                    Ok(data)
                })
                .await
                .map_err(|error| AppError::internal_error(&error))?
                .map_err(AppError::Config)?;
            provenance.content_hash = Some(data.sha256().to_owned());
            attachments.0.push(CordisAttachment {
                resource,
                provenance,
                data,
            });
        }
        Ok(attachments)
    }

    pub(in super::super) fn metadata(&self) -> Vec<Value> {
        self.0.iter().map(CordisAttachment::metadata).collect()
    }

    pub(in super::super) fn followup(
        &self,
        session: SessionId,
        blocks: Vec<CordisFollowupBlock>,
    ) -> Result<CordisFollowup, AppError> {
        let mut followup = CordisFollowup {
            document: ComposerDocument::default(),
            images: UserMediaSnapshots::new(),
            files: Default::default(),
        };
        let mut attachment_count = 0;
        for block in blocks {
            let node = match block {
                CordisFollowupBlock::Text { text } => ComposerNode::Text { text },
                CordisFollowupBlock::Attachment { index } => {
                    let attachment = self.0.get(index).ok_or_else(|| {
                        AppError::Config(
                            "A Cordis followup referred to an attachment outside this invocation"
                                .into(),
                        )
                    })?;
                    attachment_count += 1;
                    if attachment_count > media_input::MAX_MEDIA_INPUTS {
                        return Err(AppError::Config(
                            "A Cordis followup can contain at most eight attachments".into(),
                        ));
                    }
                    let id = ActivityId::new();
                    let mut resource = attachment.resource.clone();
                    match &attachment.data {
                        AttachmentData::Image(image) => {
                            resource.delivery = ResourceDelivery::ModelInput;
                            resource.kind = ResourceKind::Image;
                            resource.media_type = Some(image.media.mime.clone());
                            resource.size_bytes = Some(image.media.size_bytes);
                            resource.width = image.media.width;
                            resource.height = image.media.height;
                            followup.images.insert(id, image.clone());
                        }
                        AttachmentData::File { bytes, sha256 } => {
                            let path = file_snapshot_path(session, sha256, &resource.name);
                            resource.delivery = ResourceDelivery::Reference;
                            resource.kind = ResourceKind::File;
                            resource.reference = ResourceReference::WorkspacePath {
                                path: path.to_string_lossy().into_owned(),
                            };
                            resource.media_type = None;
                            resource.size_bytes = Some(bytes.len() as u64);
                            resource.width = None;
                            resource.height = None;
                            followup.files.insert(index);
                        }
                    }
                    ComposerNode::Activity {
                        activity: Box::new(ComposerActivity {
                            id,
                            payload: ActivityPayload::Resource(resource),
                            provenance: attachment.provenance.clone(),
                        }),
                    }
                }
            };
            followup.document.0.push(node);
        }
        if followup.document.is_empty() {
            return Err(AppError::Config("The Cordis followup is empty".into()));
        }
        Ok(followup)
    }

    pub(in super::super) async fn publish_files(
        &self,
        session: SessionId,
        followup: &CordisFollowup,
    ) -> Result<(), AppError> {
        for index in &followup.files {
            let attachment = &self.0[*index];
            let AttachmentData::File { bytes, sha256 } = &attachment.data else {
                unreachable!("selected Cordis file snapshots contain file bytes");
            };
            let path = file_snapshot_path(session, sha256, &attachment.resource.name);
            // An ordinary file is presented to the model as a read-only path,
            // matching Cordis file blocks even for routes without media input.
            let bytes = bytes.clone();
            MEDIA_WORKERS
                .run(move || publish_snapshot(&path, &bytes))
                .await
                .map_err(|error| AppError::internal_error(&error))?
                .map_err(|error| {
                    AppError::Config(format!(
                        "Unable to save the selected file snapshot: {error}"
                    ))
                })?;
        }
        Ok(())
    }
}

impl CordisAttachment {
    fn metadata(&self) -> Value {
        let id = format!("sha256:{}", self.data.sha256());
        match &self.data {
            AttachmentData::Image(image) => json!({
                "type": "image",
                "attachment": {
                    "attachmentId": id, "name": self.resource.name, "bytes": image.media.size_bytes,
                    "mediaType": image.media.mime, "width": image.media.width, "height": image.media.height,
                }
            }),
            AttachmentData::File { bytes, .. } => json!({
                "type": "file",
                "attachment": { "attachmentId": id, "name": self.resource.name, "bytes": bytes.len() }
            }),
        }
    }
}

impl AttachmentData {
    fn sha256(&self) -> &str {
        match self {
            Self::Image(image) => &image.media.sha256,
            Self::File { sha256, .. } => sha256,
        }
    }
}

fn attachment_required() -> AppError {
    AppError::Config("Cordis commands accept selected files and images from this session".into())
}

fn file_snapshot_path(session: SessionId, sha256: &str, name: &str) -> PathBuf {
    agena_storage_layout::StorageLayout::current()
        .session_artifacts_dir(session.get())
        .join("cordis-files")
        .join(sha256)
        .join(name)
}

fn publish_snapshot(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::{fs, io::ErrorKind};

    let directory = path.parent().expect("Cordis file path has a parent");
    agena_storage_layout::secure_directory(directory)?;
    if directory.canonicalize()? != directory {
        return Err(std::io::Error::other(
            "attachment directory changed during validation",
        ));
    }
    let permissions = {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            Some(fs::Permissions::from_mode(0o400))
        }
        #[cfg(not(unix))]
        {
            None
        }
    };
    match agena_runtime_tools::atomic_create_file(path, bytes, permissions) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            if !path.symlink_metadata()?.file_type().is_file() {
                return Err(std::io::Error::other(
                    "attachment snapshot is not a regular file",
                ));
            }
            agena_runtime_tools::verify_file_contents(path, bytes)?;
        }
        Err(error) => return Err(error),
    }
    #[cfg(not(unix))]
    {
        agena_storage_layout::secure_file(path)?;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_readonly(true);
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}
