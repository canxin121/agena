//! User send is the authorization gesture for explicitly selected model-input
//! resources. References and UI thumbnails remain lazy. Snapshot bytes before
//! persistence and bind them to the selected route; history cannot silently
//! transfer them to another provider/model.
use super::{AppError, Session, SessionManager};
use agena_domain::{AccessKind, AttachmentSource, ModelRef, PermissionDecision};
use agena_runtime_contracts::part_content::{TypedContent, attachment_from_file_ref};
use agena_runtime_tools::media_input;

impl SessionManager {
    pub(super) async fn materialize_user_media(
        &self,
        session: &Session,
        model: &ModelRef,
        mut parts: Vec<TypedContent>,
    ) -> Result<Vec<TypedContent>, AppError> {
        if !parts.iter().any(|part|matches!(part,TypedContent::FileRef(reference) if reference.extra.get("delivery").and_then(serde_json::Value::as_str)==Some("model_input"))) {return Ok(parts);}
        let state = self.execution_state();
        let executor = state
            .tool_executor
            .for_session_context_async(&session.runtime.execution)
            .await;
        let root = executor
            .workspace_root()
            .canonicalize()
            .map_err(|error| AppError::Internal(format!("media workspace unavailable: {error}")))?;
        let route = state.provider_registry.media_route_binding(model)?;
        let mut used = 0usize;
        let mut count = 0usize;
        for part in &mut parts {
            let TypedContent::FileRef(reference) = part else {
                continue;
            };
            if reference
                .extra
                .get("delivery")
                .and_then(serde_json::Value::as_str)
                != Some("model_input")
            {
                continue;
            }
            let current = attachment_from_file_ref(reference);
            if current.attachments.len() != 1 {
                return Err(AppError::Config(
                    "one user resource must reference exactly one input file".into(),
                ));
            }
            let attachment = &current.attachments[0];
            let AttachmentSource::LocalPath { path } = &attachment.source else {
                return Err(AppError::Config("direct model input requires an explicitly selected workspace file; provider IDs and arbitrary URLs are not portable".into()));
            };
            let path_buf = root.join(path);
            let target = path_buf.canonicalize().map_err(|error| {
                AppError::Config(format!("selected attachment is unavailable: {error}"))
            })?;
            if !target.starts_with(&root) {
                return Err(AppError::Config("selected attachment escapes the active workspace; copy it through the explicit upload action".into()));
            }
            if !matches!(
                executor
                    .principal()
                    .authorize_path_access(AccessKind::Read, &root, &target),
                PermissionDecision::Allow
            ) {
                return Err(AppError::Config(
                    "attachment read requires permission; no content was sent".into(),
                ));
            }
            count += 1;
            if count > media_input::MAX_MEDIA_INPUTS {
                return Err(AppError::Config(
                    "at most eight media resources may be sent in one message".into(),
                ));
            }
            let expected = reference.sha.clone();
            let selected_path = path.clone();
            let selected_root = root.clone();
            let prepared = tokio::task::spawn_blocking(move || {
                media_input::read_local(&selected_root, &selected_path, expected.as_deref())
            })
            .await
            .map_err(|e| AppError::Internal(format!("media preparation failed: {e}")))?
            .map_err(AppError::Config)?;
            used = used.saturating_add(prepared.bytes.len());
            if used > media_input::MAX_MEDIA_BATCH_BYTES {
                return Err(AppError::Config(
                    "message media exceeds the 40 MiB total input limit".into(),
                ));
            }
            let bound = prepared.attachment(Some(&route));
            state
                .provider_registry
                .validate_media_inputs(model, std::slice::from_ref(&bound))?;
            let path = reference.path.clone();
            let mut value =
                super::super::store::file_ref_from_attachment(&crate::part::AttachmentPart {
                    attachments: vec![bound],
                });
            // Keep local presentation path, but do not duplicate the base64
            // payload into both canonical source and attachments snapshot.
            value.path = path;
            value.extra.remove("source");
            value
                .extra
                .insert("delivery".into(), serde_json::json!("model_input"));
            value.extra.insert(
                "delivery_route".into(),
                serde_json::json!(model.to_string()),
            );
            value
                .extra
                .insert("input_sha256".into(), serde_json::json!(prepared.sha256));
            value
                .extra
                .insert("input_transport".into(), serde_json::json!("inline"));
            *reference = value;
        }
        Ok(parts)
    }
}

#[cfg(test)]
mod tests;
