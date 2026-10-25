use super::*;

/// Upload directly into a File Search store using the current provider only.
/// The signed remote upload URL is accepted only on the approved origin.
pub(super) async fn file_search(
    service: &Service<'_>,
    input: &ResourceInput,
    metadata_url: &url::Url,
) -> Result<Value> {
    let path = input
        .path
        .as_deref()
        .ok_or_else(|| invalid("knowledge.upload requires one path"))?;
    if !input.paths.is_empty() {
        return Err(invalid("knowledge.upload accepts one file per call"));
    }
    let bytes =
        authorized_input(service, path, input.expected_sha256.as_deref(), MAX_UPLOAD).await?;
    let mut start = metadata_url.clone();
    let base = url::Url::parse(&service.connection.base_url)
        .map_err(|_| invalid("invalid provider URL"))?;
    let prefix = base
        .path()
        .trim_end_matches('/')
        .strip_suffix("/v1beta")
        .ok_or_else(|| invalid("File Search upload requires a v1beta connection"))?;
    let resource = metadata_url
        .path()
        .strip_prefix(base.path().trim_end_matches('/'))
        .ok_or_else(|| invalid("invalid File Search upload path"))?;
    start.set_path(&format!("{prefix}/upload/v1beta{resource}"));
    official_service::validate_provider_endpoint(service.host, start.as_str()).await?;
    let filename = std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("input");
    let mime = input
        .parameters
        .get("mimeType")
        .and_then(Value::as_str)
        .unwrap_or(multipart::file_mime(filename));
    let mut metadata = input.parameters.clone();
    metadata
        .entry("displayName".into())
        .or_insert_with(|| json!(filename));
    let begin = service
        .client
        .post(start)
        .headers(official_service::provider_headers(&service.connection)?)
        .header("x-agena-provider-api", "gemini")
        .header("X-Goog-Upload-Protocol", "resumable")
        .header("X-Goog-Upload-Command", "start")
        .header("X-Goog-Upload-Header-Content-Length", bytes.len())
        .header("X-Goog-Upload-Header-Content-Type", mime)
        .json(&metadata)
        .send()
        .await
        .map_err(|e| {
            ambiguous(format!(
                "File Search upload initialization failed; do not automatically repeat: {}",
                e.without_url()
            ))
        })?;
    if !begin.status().is_success() {
        return Err(ambiguous(format!(
            "File Search upload initialization returned HTTP {}",
            begin.status()
        )));
    }
    let target = begin
        .headers()
        .get("x-goog-upload-url")
        .and_then(|s| s.to_str().ok())
        .ok_or_else(|| ambiguous("provider did not return a resumable upload URL"))?;
    let target = url::Url::parse(target).map_err(|_| invalid("invalid resumable upload URL"))?;
    if target.origin() != base.origin()
        || !target.username().is_empty()
        || target.password().is_some()
        || target.fragment().is_some()
    {
        return Err(invalid(
            "resumable upload changed the approved provider origin; file bytes were not sent",
        ));
    }
    official_service::validate_provider_endpoint(service.host, target.as_str()).await?;
    let response = service
        .client
        .post(target)
        .headers(official_service::provider_headers(&service.connection)?)
        .header("x-agena-provider-api", "gemini")
        .header("X-Goog-Upload-Offset", "0")
        .header("X-Goog-Upload-Command", "upload, finalize")
        .header("content-type", mime)
        .body(bytes)
        .send()
        .await
        .map_err(|e| {
            ambiguous(format!(
                "File Search upload acceptance is unknown; do not automatically repeat: {}",
                e.without_url()
            ))
        })?;
    let (status, request_id, value) =
        official_service::read_json_response_bounded(response, "gemini", "File Search upload")
            .await?;
    if !status.is_success() {
        return Err(ambiguous(format!(
            "File Search upload returned HTTP {status}"
        )));
    }
    Ok(json!({"request_id":request_id,"response":value}))
}
