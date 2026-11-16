//! Media preparation inside a standalone, network-free native worker.
use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};

use crate::media_input::{MAX_MEDIA_BYTES, PreparedMedia, PreparedMediaTransfer};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectInput {
    filename: String,
    bytes_base64: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum InspectResult {
    Media { media: PreparedMediaTransfer },
    Unsupported,
}

fn validate_size(length: usize) -> Result<(), ToolError> {
    if length > MAX_MEDIA_BYTES {
        return Err(ToolError::invalid_input(
            "media inspection input exceeds 20 MiB",
        ));
    }
    Ok(())
}

/// Narrow the host-compiled policy to exact input files while retaining every
/// explicit denial and class boundary. The caller has already authorized the
/// requested resource; this helper never grants a parent directory.
fn media_policy(policy: &SandboxPolicy, reads: &[PathBuf], scratch: &Path) -> SandboxPolicy {
    let mut policy = policy.clone();
    policy.readonly_paths = reads.to_vec();
    policy.readwrite_paths = vec![scratch.to_path_buf()];
    policy.private_paths = vec![scratch.to_path_buf()];
    policy.network = agena_process::sandbox::SandboxNetwork::Disabled;
    policy.timeout_ms = Some(policy.timeout_ms.unwrap_or(120_000).min(120_000));
    policy
}

async fn isolated(
    operation: &str,
    input: Value,
    policy: &SandboxPolicy,
    reads: Vec<PathBuf>,
) -> Result<ToolInvokeOutput, ToolError> {
    let scratch = tempfile::Builder::new()
        .prefix("agena-media-worker-")
        .tempdir()?;
    let scratch_path = fs::canonicalize(scratch.path())?;
    let policy = media_policy(policy, &reads, &scratch_path);
    // Only exact inputs enter the worker's local filter. The original kernel
    // policy, including denied paths, remains the outer enforcement boundary.
    let snapshot = ExecutionPrincipal::new(
        PermissionPolicy::allow_all(),
        ToolPermissionPolicy::allow_all(),
    )
    .sandbox_permission_snapshot(&scratch_path);
    let request = Request {
        operation: operation.to_owned(),
        input,
        workspace_root: scratch_path.clone(),
        snapshot,
        read_paths: reads,
        write_paths: vec![scratch_path.clone()],
    };
    run_worker_request(&request, &scratch_path, &policy, None).await
}

fn decode_media(payload: Option<Value>) -> Result<PreparedMedia, ToolError> {
    let transfer: PreparedMediaTransfer = serde_json::from_value(
        payload.ok_or_else(|| ToolError::plugin("sandbox media response has no payload"))?,
    )
    .map_err(|error| ToolError::plugin(format!("invalid sandbox media response: {error}")))?;
    transfer.into_prepared().map_err(ToolError::invalid_input)
}

/// Read and validate an already-authorized exact file without giving the media
/// parser access to the rest of the host or to any network endpoint.
pub async fn read_media_with_policy(
    path: &Path,
    expected: Option<&str>,
    policy: &SandboxPolicy,
) -> Result<PreparedMedia, ToolError> {
    if !path.is_absolute() {
        return Err(ToolError::invalid_input(
            "media worker input must be an absolute path",
        ));
    }
    let target = path.to_path_buf();
    let expected = expected.map(str::to_owned);
    if policy.disabled {
        return WORKERS
            .run(move || {
                crate::media_input::read_local(
                    Path::new(""),
                    &target.to_string_lossy(),
                    expected.as_deref(),
                )
                .map_err(ToolError::invalid_input)
            })
            .await
            .map_err(|error| ToolError::plugin(format!("media preparation failed: {error}")))?;
    }
    let policy = agena_process::worker::restrict_read(policy, &target)?;
    let target = policy.readonly_paths.first().cloned().ok_or_else(|| {
        ToolError::invalid_input("media worker policy has no approved input file")
    })?;
    let output = isolated(
        "media_read",
        serde_json::json!({"path": target, "expected_sha256": expected}),
        &policy,
        vec![target],
    )
    .await?;
    let prepared = decode_media(output.payload)?;
    if expected
        .as_ref()
        .is_some_and(|expected| !prepared.sha256.eq_ignore_ascii_case(expected))
    {
        return Err(ToolError::invalid_input(
            "media revision changed before sandbox result delivery",
        ));
    }
    Ok(prepared)
}

/// Inspect caller-owned bytes. The filename is metadata and is never opened.
/// Unsupported or undecodable content is None; worker/IPC failures are errors.
pub async fn inspect_media_bytes_with_policy(
    filename: &str,
    bytes: Vec<u8>,
    policy: &SandboxPolicy,
) -> Result<Option<PreparedMedia>, ToolError> {
    validate_size(bytes.len())?;
    let filename = agena_domain::safe_attachment_filename(filename);
    let filename = if filename.is_empty() {
        "input".to_owned()
    } else {
        filename
    };
    if filename.len() > 1024 {
        return Err(ToolError::invalid_input(
            "media filename exceeds 1024 bytes",
        ));
    }
    if policy.disabled {
        return WORKERS
            .run(move || Ok(crate::media_input::from_bytes(&filename, bytes, None).ok()))
            .await
            .map_err(|error| ToolError::plugin(format!("media inspection failed: {error}")))?;
    }
    let input = InspectInput {
        filename,
        bytes_base64: STANDARD.encode(bytes),
    };
    let output = isolated(
        "media_inspect",
        serde_json::to_value(input).map_err(|error| ToolError::invalid_input_error(&error))?,
        policy,
        Vec::new(),
    )
    .await?;
    decode_inspection(output.payload)
}

fn decode_inspection(payload: Option<Value>) -> Result<Option<PreparedMedia>, ToolError> {
    let response: InspectResult = serde_json::from_value(
        payload.ok_or_else(|| ToolError::plugin("media inspection response has no payload"))?,
    )
    .map_err(|error| ToolError::plugin(format!("invalid media inspection response: {error}")))?;
    match response {
        InspectResult::Unsupported => Ok(None),
        InspectResult::Media { media } => media
            .into_prepared()
            .map(Some)
            .map_err(ToolError::invalid_input),
    }
}

pub(super) fn inspect_in_worker(input: Value) -> Result<ToolInvokeOutput, ToolError> {
    let input: InspectInput =
        serde_json::from_value(input).map_err(|error| ToolError::invalid_input_error(&error))?;
    if input.filename.is_empty()
        || input.filename.len() > 1024
        || agena_domain::safe_attachment_filename(&input.filename) != input.filename
    {
        return Err(ToolError::invalid_input(
            "invalid media inspection filename",
        ));
    }
    if input.bytes_base64.len() > MAX_MEDIA_BYTES.div_ceil(3) * 4 {
        return Err(ToolError::invalid_input(
            "encoded media input exceeds 20 MiB",
        ));
    }
    let bytes = STANDARD
        .decode(input.bytes_base64)
        .map_err(|_| ToolError::invalid_input("media inspection input is not valid base64"))?;
    validate_size(bytes.len())?;
    let response = match crate::media_input::from_bytes(&input.filename, bytes, None) {
        Ok(media) => InspectResult::Media {
            media: media.into(),
        },
        Err(_) => InspectResult::Unsupported,
    };
    let mut output = ToolInvokeOutput::text("Media inspected in MXC sandbox");
    output.payload = Some(
        serde_json::to_value(response).map_err(|error| ToolError::invalid_input_error(&error))?,
    );
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_policy_keeps_denials_and_grants_only_the_exact_input_and_private_scratch() {
        let original = SandboxPolicy {
            readonly_paths: vec![PathBuf::from("/workspace")],
            readwrite_paths: vec![PathBuf::from("/output")],
            denied_paths: vec![PathBuf::from("/workspace/secret")],
            default_denied_paths: vec![PathBuf::from("/home")],
            configured_filesystem: true,
            network: agena_process::sandbox::SandboxNetwork::Native {
                policy: agena_domain::NetworkPermissionConfig::allow_all(),
            },
            ..Default::default()
        };
        let input = PathBuf::from("/workspace/media.png");
        let narrowed = media_policy(&original, &[input.clone()], Path::new("/private/worker"));
        assert_eq!(narrowed.readonly_paths, vec![input]);
        assert_eq!(
            narrowed.readwrite_paths,
            vec![PathBuf::from("/private/worker")]
        );
        assert_eq!(narrowed.denied_paths, original.denied_paths);
        assert_eq!(narrowed.default_denied_paths, original.default_denied_paths);
        assert!(narrowed.configured_filesystem);
        assert!(matches!(
            narrowed.network,
            agena_process::sandbox::SandboxNetwork::Disabled
        ));
    }

    #[test]
    fn unsupported_content_is_distinct_from_invalid_ipc() {
        let unsupported = inspect_in_worker(serde_json::json!({
            "filename":"image.png", "bytes_base64":STANDARD.encode(b"not an image"),
        }))
        .unwrap();
        assert!(decode_inspection(unsupported.payload).unwrap().is_none());
        assert!(
            inspect_in_worker(serde_json::json!({
                "filename":"image.png", "bytes_base64":"%%%invalid",
            }))
            .is_err()
        );
        assert!(decode_inspection(None).is_err());
        assert!(decode_inspection(Some(serde_json::json!({}))).is_err());
    }

    #[test]
    fn inspected_media_uses_metadata_filename_and_validates_returned_hash() {
        let output = inspect_in_worker(serde_json::json!({
            "filename":"document.pdf", "bytes_base64":STANDARD.encode(b"%PDF-1.7\n"),
        }))
        .unwrap();
        let mut payload = output.payload.unwrap();
        let prepared = decode_inspection(Some(payload.clone())).unwrap().unwrap();
        assert_eq!(prepared.filename, "document.pdf");
        assert_eq!(prepared.bytes, b"%PDF-1.7\n");
        payload["media"]["sha256"] = Value::String("0".repeat(64));
        assert!(decode_inspection(Some(payload)).is_err());
    }

    #[tokio::test]
    async fn direct_inspection_preserves_the_same_non_media_contract() {
        let policy = SandboxPolicy {
            disabled: true,
            ..Default::default()
        };
        assert!(
            inspect_media_bytes_with_policy("broken.png", b"bad".to_vec(), &policy)
                .await
                .unwrap()
                .is_none()
        );
        let media = inspect_media_bytes_with_policy("report.pdf", b"%PDF-1.7\n".to_vec(), &policy)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(media.mime, "application/pdf");
    }
}
