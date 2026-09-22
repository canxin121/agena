//! Signed, session/workspace/credential-scoped remote handles. Metadata is
//! durable but raw local bytes and API keys are never written to these records.
use super::*;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Read,
    sync::{LazyLock, Mutex, Weak},
};
static GATES: LazyLock<Mutex<BTreeMap<PathBuf, Weak<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Record {
    pub handle: String,
    pub provider: String,
    pub owner_session: i64,
    pub owner_workspace: String,
    pub connection: String,
    pub reference: String,
    pub remote_id: String,
    pub filename: String,
    pub mime: String,
    pub kind: agena_domain::AttachmentKind,
    pub sha256: String,
    pub size_bytes: u64,
    pub state: String,
    pub expires_at: Option<String>,
    pub request_id: Option<String>,
    pub signature: String,
}
fn hmac(key: &[u8], message: &[u8]) -> String {
    let mut block = [0u8; 64];
    if key.len() > 64 {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let inner = block.map(|b| b ^ 0x36);
    let outer = block.map(|b| b ^ 0x5c);
    let mut digest = Sha256::new();
    digest.update(inner);
    digest.update(message);
    let result = digest.finalize();
    let mut digest = Sha256::new();
    digest.update(outer);
    digest.update(result);
    hex::encode(digest.finalize())
}
impl Record {
    pub(super) fn is_expired(&self) -> bool {
        self.expires_at.as_deref().is_some_and(|value| {
            value
                .parse::<i64>()
                .map(|seconds| seconds <= chrono::Utc::now().timestamp())
                .unwrap_or_else(|_| {
                    chrono::DateTime::parse_from_rfc3339(value)
                        .is_ok_and(|when| when <= chrono::Utc::now())
                })
        })
    }
    fn sign(&mut self, key: &str) {
        self.signature.clear();
        self.signature = hmac(
            key.as_bytes(),
            &serde_json::to_vec(self).expect("record serialization"),
        );
    }
    fn valid_signature(&self, key: &str) -> bool {
        let mut signed = self.clone();
        signed.sign(key);
        self.signature.len() == signed.signature.len()
            && self
                .signature
                .bytes()
                .zip(signed.signature.bytes())
                .fold(0, |diff, (a, b)| diff | (a ^ b))
                == 0
    }
    fn output(&self, warning: Option<String>) -> SdkResult<ToolInvokeOutput> {
        let text = format!(
            "Cloud file {} · {}\nProvider: {}\nFile: {} ({} bytes, {})\nContent SHA-256: {}\nRemote file ID: {}\nExpiry reported by provider: {:?}\nThis handle is bound to this workspace/session/connection. It is not a local file or an analysis result.{}",
            self.handle,
            self.state,
            self.provider,
            self.filename,
            self.size_bytes,
            self.mime,
            self.sha256,
            self.remote_id,
            self.expires_at,
            warning
                .as_ref()
                .map(|w| format!("\nWarning: {w}"))
                .unwrap_or_default()
        );
        Ok(ToolInvokeOutput::from_parts(
            format!("{} cloud file", self.provider),
            format!("{} · {} bytes", self.state, self.size_bytes),
            text,
            Some(json!({
                "handle":self.handle,"provider":self.provider,"execution_location":"vendor_cloud","state":self.state,"remote_file_id":self.remote_id,"filename":self.filename,"mime":self.mime,"size_bytes":self.size_bytes,"sha256":self.sha256,"expires_at":self.expires_at,"request_id":self.request_id,"storage_warning":warning,"safe_to_repeat_upload_automatically":false,"deletion_acknowledged":self.state=="deleted","physical_erasure_guaranteed":false,
            })),
            Default::default(),
            Vec::new(),
        ))
    }
}
impl Service<'_> {
    fn connection(&self, key: &str) -> SdkResult<String> {
        Ok(hmac(
            key.as_bytes(),
            format!(
                "agena-cloud-media-connection-v1\0{}\0{}",
                self.provider,
                self.base()?
            )
            .as_bytes(),
        ))
    }
    fn record_path(&self, handle: &str) -> SdkResult<PathBuf> {
        if !handle.starts_with("media_")
            || handle.len() != 38
            || !handle[6..].bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(PluginError::invalid_params("invalid cloud media handle"));
        }
        Ok(self
            .root
            .join(".agena/artifacts/provider-tools/media")
            .join(format!("{handle}.json")))
    }
    async fn gate(&self) -> SdkResult<tokio::sync::OwnedMutexGuard<()>> {
        let root = self
            .root
            .canonicalize()
            .map_err(|e| PluginError::internal_error(&e))?
            .join(format!(".agena/cloud-media-lock-{}", self.provider));
        let gate = {
            let mut gates = GATES
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            gates.retain(|_, g| g.strong_count() > 0);
            if let Some(gate) = gates.get(&root).and_then(Weak::upgrade) {
                gate
            } else {
                let gate = Arc::new(tokio::sync::Mutex::new(()));
                gates.insert(root, Arc::downgrade(&gate));
                gate
            }
        };
        Ok(gate.lock_owned().await)
    }
    async fn save(&self, record: &Record, create: bool) -> SdkResult<()> {
        let path = self.record_path(&record.handle)?;
        let root = self.root.to_owned();
        let bytes = serde_json::to_vec(record).map_err(|e| PluginError::internal_error(&e))?;
        tokio::task::spawn_blocking(move || {
            let root = root.canonicalize()?;
            let relative = path
                .strip_prefix(&root)
                .map_err(|_| std::io::Error::other("media record outside workspace"))?;
            let mut directory = root.clone();
            for component in relative.parent().unwrap().components() {
                directory.push(component);
                match std::fs::symlink_metadata(&directory) {
                    Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
                    Ok(_) => {
                        return Err(std::io::Error::other(
                            "media record directory is not a regular directory",
                        ));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        std::fs::create_dir(&directory)?;
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            std::fs::set_permissions(
                                &directory,
                                std::fs::Permissions::from_mode(0o700),
                            )?;
                        }
                    }
                    Err(e) => return Err(e),
                }
            }
            if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(std::io::Error::other("media record cannot be a symlink"));
            }
            if create {
                agena_runtime_tools::atomic_create_file(&path, &bytes, None)
            } else {
                agena_runtime_tools::atomic_write_file(&path, &bytes)
            }
        })
        .await
        .map_err(|e| PluginError::internal(format!("media record worker failed: {e}")))?
        .map_err(|e| PluginError::internal_error(&e))
    }
    pub(super) async fn load_record(
        &self,
        context: &ToolInvokeContext<'_>,
        handle: &str,
        key: &str,
    ) -> SdkResult<Record> {
        let path = self.record_path(handle)?;
        let root = self
            .root
            .canonicalize()
            .map_err(|e| PluginError::internal_error(&e))?;
        let data = tokio::task::spawn_blocking(move || -> std::io::Result<Vec<u8>> {
            let target = path.canonicalize()?;
            if !target.starts_with(&root) {
                return Err(std::io::Error::other("cloud record escapes workspace"));
            }
            let mut options = std::fs::OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
            }
            let file = options.open(path)?;
            if !file.metadata()?.is_file() {
                return Err(std::io::Error::other("invalid cloud record"));
            }
            let mut bytes = Vec::new();
            file.take(32769).read_to_end(&mut bytes)?;
            if bytes.len() > 32768 {
                return Err(std::io::Error::other("cloud record exceeds safety limit"));
            }
            Ok(bytes)
        })
        .await
        .map_err(|e| PluginError::internal(format!("media record read failed: {e}")))?
        .map_err(|_| PluginError::invalid_params("cloud handle was not found in this workspace"))?;
        let record: Record = serde_json::from_slice(&data)
            .map_err(|_| PluginError::invalid_params("invalid cloud media record"))?;
        if !record.valid_signature(key)
            || record.owner_session != context.session_id
            || record.provider != self.provider
            || record.handle != handle
            || record.owner_workspace
                != self
                    .root
                    .canonicalize()
                    .map_err(|e| PluginError::internal_error(&e))?
                    .display()
                    .to_string()
            || record.connection != self.connection(key)?
        {
            return Err(PluginError::invalid_params(
                "cloud handle is not owned by this session/provider/connection or its record was modified; no remote request was sent",
            ));
        }
        Ok(record)
    }
    fn remote_endpoint(&self, id: &str) -> SdkResult<String> {
        let path = if self.provider == "gemini" {
            let token = id.strip_prefix("files/").ok_or_else(|| {
                PluginError::internal("Google file response omitted files/ identity")
            })?;
            if token.is_empty()
                || !token
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            {
                return Err(PluginError::internal(
                    "unsafe Google remote file identifier",
                ));
            }
            id.to_owned()
        } else {
            if id.is_empty()
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            {
                return Err(PluginError::internal(
                    "unsafe provider remote file identifier",
                ));
            }
            format!(
                "{}files/{id}",
                if self.provider == "claude" { "v1/" } else { "" }
            )
        };
        self.endpoint(&path)
    }
    fn update_remote(&self, record: &mut Record, value: &Value) -> SdkResult<()> {
        let value = value.get("file").unwrap_or(value);
        let id=value.get(if self.provider=="gemini"{"name"}else{"id"}).and_then(Value::as_str).ok_or_else(||PluginError::internal("cloud upload response omitted a file identity; inspect the request receipt before repeating"))?;
        let endpoint = self.remote_endpoint(id)?;
        if !record.remote_id.is_empty() && record.remote_id != id {
            return Err(PluginError::internal(
                "cloud file response identity changed",
            ));
        }
        record.remote_id = id.into();
        record.reference = if self.provider == "gemini" {
            let uri = value
                .get("uri")
                .and_then(Value::as_str)
                .unwrap_or(&endpoint);
            let uri = url::Url::parse(uri).map_err(|e| PluginError::internal_error(&e))?;
            if uri.origin() != self.base()?.origin()
                || uri.query().is_some()
                || uri.fragment().is_some()
            {
                return Err(PluginError::internal(
                    "cloud file URI is outside the approved provider origin",
                ));
            }
            uri.to_string()
        } else {
            id.to_owned()
        };
        record.state = match self.provider {
            "gemini" => match value.get("state").and_then(Value::as_str) {
                Some("ACTIVE") => "ready",
                Some("PROCESSING") => "processing",
                Some("FAILED") => "failed",
                _ => "unknown",
            },
            "chatgpt" => match value.get("status").and_then(Value::as_str) {
                Some("error" | "failed") => "failed",
                Some("uploaded" | "processing") => "processing",
                Some("processed") | None => "ready",
                _ => "unknown",
            },
            _ => "ready",
        }
        .into();
        record.expires_at = value
            .get("expirationTime")
            .or_else(|| value.get("expires_at"))
            .filter(|v| !v.is_null())
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| v.to_string())
            });
        if record.is_expired() {
            record.state = "expired".into();
        }
        Ok(())
    }
    pub async fn upload(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &UploadInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.base()?;
        if self.provider == "gemini" && input.expires_in_seconds.is_some() {
            return Err(PluginError::invalid_params(
                "Google Files controls its own expiration; omit expires_in_seconds and inspect the returned expires_at",
            ));
        }
        let expiry = input.expires_in_seconds.unwrap_or(86400);
        if !(3600..=2592000).contains(&expiry) {
            return Err(PluginError::invalid_params(
                "expires_in_seconds must be 3600..2592000",
            ));
        }
        let key = official_service::env_secret(self.key_env, self.provider)?;
        let root = self.root.to_owned();
        let path = input.path.clone();
        let expected = input.expected_sha256.clone();
        let media = tokio::task::spawn_blocking(move || {
            media_input::read_local(&root, &path, expected.as_deref())
        })
        .await
        .map_err(|e| PluginError::internal_error(&e))?
        .map_err(PluginError::invalid_params)?;
        if self.provider != "gemini"
            && !matches!(
                media.kind,
                agena_domain::AttachmentKind::Image
                    | agena_domain::AttachmentKind::Pdf
                    | agena_domain::AttachmentKind::File
            )
        {
            return Err(PluginError::invalid_params(
                "this file upload tool supports image/PDF/text inputs only",
            ));
        }
        let _gate = self.gate().await?;
        let handle = format!(
            "media_{}",
            &hex::encode(Sha256::digest(format!(
                "{}:{}:{}:{}",
                self.root.display(),
                self.provider,
                context.session_id,
                context.call_id
            )))[..32]
        );
        let mut record = Record {
            handle,
            provider: self.provider.into(),
            owner_session: context.session_id,
            owner_workspace: self
                .root
                .canonicalize()
                .map_err(|e| PluginError::internal_error(&e))?
                .display()
                .to_string(),
            connection: self.connection(&key)?,
            reference: String::new(),
            remote_id: String::new(),
            filename: media.filename.clone(),
            mime: media.mime.clone(),
            kind: media.kind,
            sha256: media.sha256.clone(),
            size_bytes: media.size_bytes,
            state: "submission_unknown".into(),
            expires_at: None,
            request_id: None,
            signature: String::new(),
        };
        if self.record_path(&record.handle)?.exists() {
            let prior = self.load_record(context, &record.handle, &key).await?;
            if prior.sha256 != record.sha256 {
                return Err(PluginError::invalid_params(
                    "call id already reserved for different media bytes",
                ));
            }
            return prior.output(Some("Existing upload attempt was not repeated. Query status; an unknown submission may require provider-side inspection.".into()));
        }
        record.sign(&key);
        self.save(&record, true).await?;
        let attempt = self.upload_bytes(&media, expiry, &key).await;
        let response=match attempt{Ok(response)=>response,Err(error)=>return record.output(Some(format!("Upload did not return a confirmed result: {}. Remote acceptance is unknown; do not automatically repeat.",error.failure.user.fallback)))};
        record.request_id = response.request_id;
        if let Err(error) = self.update_remote(&mut record, &response.value) {
            record.state = "response_invalid".into();
            record.sign(&key);
            let persistence = self
                .save(&record, false)
                .await
                .err()
                .map(|_| " Local metadata could not be updated.")
                .unwrap_or_default();
            return record.output(Some(format!("Provider response was received but metadata validation failed: {}. Inspect provider records before retrying.{persistence}",error.failure.user.fallback)));
        }
        record.sign(&key);
        let warning=self.save(&record,false).await.err().map(|e|format!("Remote upload succeeded, but its local manifest could not be saved: {}. Retain the remote file ID; do not repeat upload.",e.failure.user.fallback));
        record.output(warning)
    }
    async fn upload_bytes(
        &self,
        media: &PreparedMedia,
        expiry: u32,
        key: &str,
    ) -> SdkResult<ProviderHttpResponse> {
        if self.provider != "gemini" {
            let mut form = reqwest::multipart::Form::new().part(
                "file",
                reqwest::multipart::Part::bytes(media.bytes.clone())
                    .file_name(media.filename.clone())
                    .mime_str(&media.mime)
                    .map_err(|e| PluginError::invalid_params_error(&e))?,
            );
            if self.provider == "chatgpt" {
                form = form
                    .text("purpose", "user_data")
                    .text("expires_after[anchor]", "created_at")
                    .text("expires_after[seconds]", expiry.to_string());
            } else {
                form = form.text("expires_in_seconds", expiry.to_string());
            }
            return self
                .response(
                    self.authenticated(
                        crate::PROVIDER_HTTP_CLIENT
                            .post(self.endpoint(if self.provider == "claude" {
                                "v1/files"
                            } else {
                                "files"
                            })?)
                            .multipart(form),
                        key,
                    ),
                    "file upload",
                )
                .await;
        }
        let mut start = self.base()?;
        let original = start.path().trim_end_matches('/');
        let (prefix, version) = original.rsplit_once('/').unwrap_or(("", "v1beta"));
        start.set_path(&format!("{prefix}/upload/{version}/files"));
        let begin = self
            .authenticated(
                crate::PROVIDER_HTTP_CLIENT
                    .post(start)
                    .header("X-Goog-Upload-Protocol", "resumable")
                    .header("X-Goog-Upload-Command", "start")
                    .header("X-Goog-Upload-Header-Content-Length", media.size_bytes)
                    .header("X-Goog-Upload-Header-Content-Type", &media.mime)
                    .json(&json!({"file":{"display_name":media.filename}})),
                key,
            )
            .send()
            .await
            .map_err(|e| PluginError::internal_error(&e))?;
        if !begin.status().is_success() {
            return Err(PluginError::internal(format!(
                "Google upload initialization failed with HTTP {}",
                begin.status()
            )));
        }
        let target = begin
            .headers()
            .get("x-goog-upload-url")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| PluginError::internal("Google did not return an upload URL"))?;
        let target = url::Url::parse(target).map_err(|e| PluginError::internal_error(&e))?;
        if target.origin() != self.base()?.origin()
            || !target.username().is_empty()
            || target.password().is_some()
            || target.fragment().is_some()
        {
            return Err(PluginError::invalid_params(
                "resumable upload URL changed the approved provider origin; bytes were not forwarded",
            ));
        }
        self.response(
            crate::PROVIDER_HTTP_CLIENT
                .post(target)
                .timeout(std::time::Duration::from_secs(
                    self.timeout_secs.clamp(1, 300),
                ))
                .header("X-Goog-Upload-Offset", "0")
                .header("X-Goog-Upload-Command", "upload, finalize")
                .header("content-type", &media.mime)
                .body(media.bytes.clone()),
            "file upload completion",
        )
        .await
    }
    pub async fn file_control(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &FileInput,
        delete: bool,
    ) -> SdkResult<ToolInvokeOutput> {
        self.base()?;
        let key = official_service::env_secret(self.key_env, self.provider)?;
        let _gate = self.gate().await?;
        let mut record = self.load_record(context, &input.handle, &key).await?;
        if record.state == "deleted" || record.remote_id.is_empty() {
            return record.output(None);
        }
        let url = self.remote_endpoint(&record.remote_id)?;
        let request = if delete {
            crate::PROVIDER_HTTP_CLIENT.delete(url)
        } else {
            crate::PROVIDER_HTTP_CLIENT.get(url)
        };
        if delete {
            record.state = "deletion_unconfirmed".into();
            record.sign(&key);
            self.save(&record, false).await?;
        }
        let response = self
            .authenticated(request, &key)
            .send()
            .await
            .map_err(|error| {
                PluginError::internal(format!(
                    "cloud file request failed; completion is not confirmed: {error}"
                ))
            })?;
        let status = response.status();
        if delete && (status.is_success() || status == reqwest::StatusCode::NOT_FOUND) {
            if self.provider == "chatgpt"
                && status.is_success()
                && status != reqwest::StatusCode::NO_CONTENT
            {
                let (_, request_id, body) = official_service::read_json_response_bounded(
                    response,
                    self.provider,
                    "file deletion",
                )
                .await?;
                record.request_id = request_id;
                if body.get("deleted").and_then(Value::as_bool) != Some(true)
                    || body.get("id").and_then(Value::as_str) != Some(record.remote_id.as_str())
                {
                    return record.output(Some("Provider did not confirm deletion of the requested file; query status before retrying.".into()));
                }
            }
            record.state = "deleted".into();
        } else if status == reqwest::StatusCode::NOT_FOUND {
            record.state = "unavailable".into();
        } else {
            let (status, request_id, value) = official_service::read_json_response_bounded(
                response,
                self.provider,
                "file status",
            )
            .await?;
            if !status.is_success() {
                return Err(PluginError::internal(format!(
                    "cloud file status returned HTTP {status}"
                )));
            }
            self.update_remote(&mut record, &value)?;
            record.request_id = request_id;
        }
        record.sign(&key);
        let warning = self.save(&record, false).await.err().map(|e| {
            format!(
                "Remote response received; local status persistence failed: {}",
                e.failure.user.fallback
            )
        });
        record.output(warning)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_cloud_records_cannot_be_modified_or_moved_to_other_credentials() {
        assert_eq!(
            hmac(&[0x0b; 20], b"Hi There"),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        let mut record = Record {
            handle: "media_00000000000000000000000000000000".into(),
            provider: "chatgpt".into(),
            owner_session: 1,
            owner_workspace: "/fixture".into(),
            connection: "test".into(),
            reference: "file-x".into(),
            remote_id: "file-x".into(),
            filename: "image.png".into(),
            mime: "image/png".into(),
            kind: agena_domain::AttachmentKind::Image,
            sha256: "fixture".into(),
            size_bytes: 1,
            state: "ready".into(),
            expires_at: None,
            request_id: None,
            signature: String::new(),
        };
        record.sign("test-key");
        assert!(record.valid_signature("test-key"));
        assert!(!record.valid_signature("other-key"));
        record.owner_session = 2;
        assert!(!record.valid_signature("test-key"));
    }
}
