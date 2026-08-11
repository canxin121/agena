use std::path::Path;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt as _;

use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::attachment::{AttachmentItem, AttachmentKind, AttachmentSource};
use agena_plugin_host::sdk::host_api::HostImageInput;
use agena_provider::{ProviderImageCapabilities, ProviderImageInput, ProviderNativeToolArtifact};

use crate::tool::ToolExecutor;

const HOST_IMAGE_MAX_BYTES: usize = 50 * 1024 * 1024;

pub(super) async fn prepare_provider_image_inputs(
    executor: &ToolExecutor,
    inputs: &[HostImageInput],
    capabilities: &ProviderImageCapabilities,
) -> Result<Vec<ProviderImageInput>, PluginError> {
    if inputs.len() > capabilities.max_input_images.unwrap_or(u32::MAX) as usize {
        return Err(PluginError::invalid_params(format!(
            "image request contains {} input image(s), exceeding the active route limit of {}",
            inputs.len(),
            capabilities.max_input_images.unwrap_or(u32::MAX)
        )));
    }

    let mut prepared = Vec::with_capacity(inputs.len());
    for (index, input) in inputs.iter().enumerate() {
        let value = match input {
            HostImageInput::Path { path } => prepare_path_image(executor, path, capabilities).await,
            HostImageInput::Attachment { attachment } => {
                prepare_attachment_image(executor, attachment, capabilities).await
            }
        }
        .map_err(|error| {
            PluginError::invalid_params(format!("invalid image input {index}: {error}"))
        })?;
        prepared.push(value);
    }
    Ok(prepared)
}

async fn prepare_attachment_image(
    executor: &ToolExecutor,
    attachment: &AttachmentItem,
    capabilities: &ProviderImageCapabilities,
) -> Result<ProviderImageInput, String> {
    if attachment.kind != AttachmentKind::Image {
        return Err(format!(
            "attachment kind must be image, found {}",
            attachment.kind
        ));
    }
    match &attachment.source {
        AttachmentSource::LocalPath { path } => {
            prepare_path_image(executor, path, capabilities)
                .await
                .map_err(|error| error.to_string())
        }
        AttachmentSource::Base64 { data } => prepare_image_bytes(
            data,
            attachment.mime.as_str(),
            attachment.filename.clone(),
            attachment.size_bytes,
            attachment.sha256.as_deref(),
            capabilities,
        ),
        AttachmentSource::DataUrl { url } => {
            let (data_mime, encoded) = agena_runtime_tools::parse_base64_image_data_url(url)
                .ok_or_else(|| "attachment data URL must contain a base64 image".to_owned())?;
            let declared_mime = normalize_image_mime(attachment.mime.as_str());
            let data_mime = normalize_image_mime(data_mime.as_str());
            if !declared_mime.is_empty() && declared_mime != data_mime {
                return Err(format!(
                    "attachment MIME `{declared_mime}` does not match data URL MIME `{data_mime}`"
                ));
            }
            prepare_image_bytes(
                encoded.as_str(),
                data_mime.as_str(),
                attachment.filename.clone(),
                attachment.size_bytes,
                attachment.sha256.as_deref(),
                capabilities,
            )
        }
        AttachmentSource::Url { .. } => Err(
            "remote URL image inputs are not accepted by the direct image host API; materialize the image as a permitted local attachment first"
                .to_owned(),
        ),
        AttachmentSource::FileId { .. } => Err(
            "provider file-id image inputs are not portable across the selected route"
                .to_owned(),
        ),
    }
}

async fn prepare_path_image(
    executor: &ToolExecutor,
    path: &str,
    capabilities: &ProviderImageCapabilities,
) -> Result<ProviderImageInput, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("image path must not be empty".to_owned());
    }
    let target = executor.resolve_target_path(path);
    let metadata = tokio::fs::metadata(&target)
        .await
        .map_err(|error| format!("cannot stat image path `{}`: {error}", target.display()))?;
    if !metadata.is_file() {
        return Err(format!(
            "image path is not a regular file: {}",
            target.display()
        ));
    }
    let route_limit = capabilities
        .max_input_bytes
        .unwrap_or(HOST_IMAGE_MAX_BYTES as u64)
        .min(HOST_IMAGE_MAX_BYTES as u64);
    if metadata.len() == 0 {
        return Err(format!("image file is empty: {}", target.display()));
    }
    if metadata.len() > route_limit {
        return Err(format!(
            "image file has {} bytes, exceeding the active route limit of {route_limit} bytes",
            metadata.len()
        ));
    }
    let file = tokio::fs::File::open(&target)
        .await
        .map_err(|error| format!("cannot read image path `{}`: {error}", target.display()))?;
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or_default());
    file.take(route_limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| format!("cannot read image path `{}`: {error}", target.display()))?;
    if bytes.len() as u64 > route_limit {
        return Err(format!(
            "image file grew beyond the active route limit of {route_limit} bytes while being read"
        ));
    }
    let filename = target
        .file_name()
        .and_then(|value| value.to_str())
        .map(ToOwned::to_owned);
    prepare_decoded_image(bytes, None, filename, None, None, capabilities)
}

fn prepare_image_bytes(
    encoded: &str,
    declared_mime: &str,
    filename: Option<String>,
    declared_size: Option<u64>,
    declared_sha256: Option<&str>,
    capabilities: &ProviderImageCapabilities,
) -> Result<ProviderImageInput, String> {
    let encoded = encoded.trim();
    if encoded.is_empty() {
        return Err("base64 image payload must not be empty".to_owned());
    }
    let route_limit = capabilities
        .max_input_bytes
        .unwrap_or(HOST_IMAGE_MAX_BYTES as u64)
        .min(HOST_IMAGE_MAX_BYTES as u64) as usize;
    if encoded.len().saturating_mul(3) / 4 > route_limit {
        return Err(format!(
            "base64 image payload exceeds the active route limit of {route_limit} bytes"
        ));
    }
    let bytes = STANDARD
        .decode(encoded.as_bytes())
        .map_err(|error| format!("invalid base64 image payload: {error}"))?;
    prepare_decoded_image(
        bytes,
        Some(declared_mime),
        filename,
        declared_size,
        declared_sha256,
        capabilities,
    )
}

fn prepare_decoded_image(
    bytes: Vec<u8>,
    declared_mime: Option<&str>,
    filename: Option<String>,
    declared_size: Option<u64>,
    declared_sha256: Option<&str>,
    capabilities: &ProviderImageCapabilities,
) -> Result<ProviderImageInput, String> {
    let route_limit = capabilities
        .max_input_bytes
        .unwrap_or(HOST_IMAGE_MAX_BYTES as u64)
        .min(HOST_IMAGE_MAX_BYTES as u64);
    if bytes.is_empty() {
        return Err("decoded image payload must not be empty".to_owned());
    }
    if bytes.len() as u64 > route_limit {
        return Err(format!(
            "decoded image has {} bytes, exceeding the active route limit of {route_limit} bytes",
            bytes.len()
        ));
    }
    let detected_mime = detect_image_mime(bytes.as_slice())
        .ok_or_else(|| "unsupported or unrecognized image signature".to_owned())?;
    let declared_mime = declared_mime
        .map(normalize_image_mime)
        .filter(|value| !value.is_empty());
    if declared_mime
        .as_deref()
        .is_some_and(|declared| declared != detected_mime)
    {
        return Err(format!(
            "declared MIME `{}` does not match detected image MIME `{detected_mime}`",
            declared_mime.as_deref().unwrap_or_default()
        ));
    }
    if !capabilities
        .accepted_input_mime_types
        .iter()
        .map(|value| normalize_image_mime(value))
        .any(|value| value == detected_mime)
    {
        return Err(format!(
            "detected image MIME `{detected_mime}` is not supported by the active route"
        ));
    }
    if declared_size.is_some_and(|size| size != bytes.len() as u64) {
        return Err(format!(
            "declared attachment size does not match decoded size {}",
            bytes.len()
        ));
    }
    let sha256 = hex::encode(Sha256::digest(bytes.as_slice()));
    if declared_sha256
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some_and(|declared| !declared.eq_ignore_ascii_case(sha256.as_str()))
    {
        return Err("declared attachment SHA-256 does not match decoded bytes".to_owned());
    }
    Ok(ProviderImageInput {
        mime: detected_mime.to_owned(),
        data_base64: STANDARD.encode(bytes.as_slice()),
        filename,
        size_bytes: bytes.len() as u64,
        sha256,
    })
}

pub(super) async fn persist_provider_image_artifacts(
    workspace_root: &Path,
    session_id: i64,
    call_id: i64,
    artifacts: &[ProviderNativeToolArtifact],
) -> Result<Vec<AttachmentItem>, PluginError> {
    if artifacts.is_empty() {
        return Err(PluginError::internal(
            "provider image response did not contain any artifacts",
        ));
    }
    let artifact_id = format!("image-tool-{call_id}");
    let mut attachments = Vec::with_capacity(artifacts.len());
    for (index, artifact) in artifacts.iter().enumerate() {
        let (data_mime, encoded) = agena_runtime_tools::parse_base64_image_data_url(&artifact.uri)
            .ok_or_else(|| {
                PluginError::internal(
                    "provider image response was not a base64 data URL; transient URLs and file ids are not exposed by the direct image host API",
                )
            })?;
        let bytes = STANDARD.decode(encoded.as_bytes()).map_err(|error| {
            PluginError::internal(format!(
                "provider returned invalid base64 image data: {error}"
            ))
        })?;
        let detected_mime = detect_image_mime(bytes.as_slice()).ok_or_else(|| {
            PluginError::internal("provider returned an unrecognized image payload")
        })?;
        let declared_mime = normalize_image_mime(if data_mime.trim().is_empty() {
            artifact.mime.as_str()
        } else {
            data_mime.as_str()
        });
        if declared_mime != detected_mime {
            return Err(PluginError::internal(format!(
                "provider image MIME `{declared_mime}` does not match detected payload `{detected_mime}`"
            )));
        }
        let saved = agena_runtime_tools::persist_generated_image_artifact(
            workspace_root,
            session_id,
            artifact_id.as_str(),
            index,
            detected_mime,
            artifact.name.as_deref(),
            artifact.uri.as_str(),
        )
        .await
        .map_err(|error| PluginError::internal(error.to_string()))?
        .ok_or_else(|| {
            PluginError::internal("provider image artifact could not be persisted as managed media")
        })?;
        attachments.push(AttachmentItem {
            kind: AttachmentKind::Image,
            mime: detected_mime.to_owned(),
            source: AttachmentSource::LocalPath { path: saved.path },
            filename: Some(saved.filename),
            title: None,
            size_bytes: Some(saved.size_bytes),
            sha256: Some(saved.sha256),
            width: None,
            height: None,
            duration_ms: None,
            page_count: None,
        });
    }
    Ok(attachments)
}

fn normalize_image_mime(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "image/jpg" => "image/jpeg".to_owned(),
        other => other.to_owned(),
    }
}

fn detect_image_mime(bytes: &[u8]) -> Option<&'static str> {
    match infer::get(bytes)?.mime_type() {
        "image/png" => Some("image/png"),
        "image/jpeg" => Some("image/jpeg"),
        "image/gif" => Some("image/gif"),
        "image/webp" => Some("image/webp"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        STANDARD, detect_image_mime, normalize_image_mime, persist_provider_image_artifacts,
        prepare_decoded_image,
    };
    use agena_provider::{ProviderImageCapabilities, ProviderNativeToolArtifact};
    use base64::Engine as _;

    fn capabilities() -> ProviderImageCapabilities {
        ProviderImageCapabilities {
            generate: true,
            edit: true,
            accepted_input_mime_types: vec!["image/png".to_owned()],
            max_input_bytes: Some(1024),
            max_input_images: Some(1),
        }
    }

    #[test]
    fn image_signatures_are_not_inferred_from_extensions() {
        assert_eq!(
            detect_image_mime(b"\x89PNG\r\n\x1a\nrest"),
            Some("image/png")
        );
        assert_eq!(detect_image_mime(b"not an image"), None);
        assert_eq!(normalize_image_mime(" IMAGE/JPG "), "image/jpeg");
    }

    #[test]
    fn decoded_image_boundary_checks_mime_size_and_hash() {
        let bytes = b"\x89PNG\r\n\x1a\nfixture".to_vec();
        let prepared = prepare_decoded_image(
            bytes.clone(),
            Some("image/png"),
            Some("fixture.png".to_owned()),
            Some(bytes.len() as u64),
            None,
            &capabilities(),
        )
        .expect("valid input");
        assert_eq!(prepared.mime, "image/png");
        assert_eq!(prepared.size_bytes, bytes.len() as u64);
        assert_eq!(STANDARD.decode(prepared.data_base64).unwrap(), bytes);

        let error = prepare_decoded_image(
            b"\x89PNG\r\n\x1a\nfixture".to_vec(),
            Some("image/jpeg"),
            None,
            None,
            None,
            &capabilities(),
        )
        .expect_err("MIME mismatch must be rejected");
        assert!(error.contains("does not match detected"));
    }

    #[tokio::test]
    async fn provider_data_url_is_persisted_before_becoming_attachment() {
        let workspace = tempfile::tempdir().expect("workspace");
        let data = STANDARD.encode(b"\x89PNG\r\n\x1a\nfixture");
        let attachments = persist_provider_image_artifacts(
            workspace.path(),
            17,
            23,
            &[ProviderNativeToolArtifact {
                uri: format!("data:image/png;base64,{data}"),
                mime: "image/png".to_owned(),
                name: Some("fixture.png".to_owned()),
                size_bytes: None,
                sha256: None,
            }],
        )
        .await
        .expect("persist output");
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].mime, "image/png");
        assert!(attachments[0].size_bytes.is_some());
        assert!(attachments[0].sha256.is_some());
        let agena_plugin_host::sdk::attachment::AttachmentSource::LocalPath { path } =
            &attachments[0].source
        else {
            panic!("managed local attachment expected");
        };
        assert!(std::path::Path::new(path).is_file());
    }
}
