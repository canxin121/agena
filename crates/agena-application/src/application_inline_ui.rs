//! Read-time expansion of immutable reply references. Every lookup uses the
//! current session's membership capability (including forks/rewinds), never a
//! process-global UI registry. Reads do not invoke plugins or write anything.
use crate::{Application, ApplicationError};
use agena_domain::{
    InlineDocument, InlineSnapshot, InlineUiReferences, PreparedInlineUi,
    inline_reference_candidates, inline_reference_tool_id, is_inline_ui_prepare_tool,
};
use agena_runtime::plugins::inline_ui::InlineUiStore;
use agena_storage::kernel::{KernelError, KernelReader, SessionId, ToolPhase};

pub(crate) async fn resolve_inline_reference(
    reader: &KernelReader,
    session: SessionId,
    reference: &str,
) -> Result<Option<InlineDocument>, KernelError> {
    Ok(
        resolve_inline_snapshot(reader, session, reference, InlineUiStore::default())
            .await?
            .map(|snapshot| snapshot.document),
    )
}

/// One authorization check and one atomic snapshot read. Keeping revision and
/// content together avoids mixing two live heads when an update races a read.
async fn resolve_inline_snapshot(
    reader: &KernelReader,
    session: SessionId,
    reference: &str,
    store: InlineUiStore,
) -> Result<Option<InlineSnapshot>, KernelError> {
    let Some(id) = inline_reference_tool_id(reference) else {
        return Ok(None);
    };
    let Ok(tool_id) = id.try_into() else {
        return Ok(None);
    };
    let tool = match reader.tool(session, tool_id).await {
        Ok(tool) => tool,
        Err(KernelError::NotFound(_) | KernelError::Ownership) => return Ok(None),
        Err(error) => return Err(error),
    };
    if tool.phase != ToolPhase::Succeeded {
        return Ok(None);
    }
    let Some(result) = tool.result.as_ref() else {
        return Ok(None);
    };
    let Some(name) = result
        .pointer("/invocation/name")
        .and_then(serde_json::Value::as_str)
    else {
        return Ok(None);
    };
    if !is_inline_ui_prepare_tool(name) {
        return Ok(None);
    }
    let Some(value) = result.pointer("/raw_output/payload/inline_ui_prepared") else {
        return Ok(None);
    };
    let Ok(prepared) = serde_json::from_value::<PreparedInlineUi>(value.clone()) else {
        return Ok(None);
    };
    if prepared.reference != reference || prepared.validate().is_err() {
        return Ok(None);
    }
    let reference_owned = reference.to_owned();
    let live = tokio::task::spawn_blocking(move || store.read(session.get(), &reference_owned))
        .await
        .map_err(|error| KernelError::Storage(error.to_string()))?
        .map_err(|error| KernelError::Storage(error.to_string()))?;
    Ok(Some(live.unwrap_or(InlineSnapshot {
        reference: reference.into(),
        revision: 1,
        phase: agena_domain::InlinePhase::Complete,
        updated_at_ms: 0,
        last_call_id: id,
        document: prepared.document,
    })))
}

/// A sidecar for assistant text. Parsing code/quotation boundaries belongs to
/// renderers; merely finding a token does not execute or render its content.
pub(crate) async fn resolve_inline_text(
    reader: &KernelReader,
    session: SessionId,
    text: &str,
) -> Result<Option<InlineUiReferences>, KernelError> {
    if !text.contains("ui_") {
        return Ok(None);
    }
    let mut refs = InlineUiReferences::new();
    for reference in inline_reference_candidates(text) {
        if let Some(document) = resolve_inline_reference(reader, session, &reference).await? {
            refs.insert(reference, document);
        }
    }
    Ok(Some(refs))
}

impl Application {
    pub async fn read_inline_ui_reference(
        &self,
        session_id: i64,
        reference: &str,
    ) -> Result<InlineDocument, ApplicationError> {
        let session = crate::kernel_session_id(session_id)?;
        resolve_inline_reference(&self.session_kernel()?.reader, session, reference)
            .await
            .map_err(crate::kernel_error)?
            .ok_or_else(|| {
                ApplicationError::not_found("Inline content is unavailable in this conversation.")
            })
    }
    /// A versioned snapshot; access is checked against the originating tool's
    /// membership before touching any private live state.
    pub async fn read_inline_ui_snapshot(
        &self,
        session_id: i64,
        reference: &str,
    ) -> Result<agena_domain::InlineSnapshot, ApplicationError> {
        let session = crate::kernel_session_id(session_id)?;
        resolve_inline_snapshot(
            &self.session_kernel()?.reader,
            session,
            reference,
            InlineUiStore::default(),
        )
        .await
        .map_err(crate::kernel_error)?
        .ok_or_else(|| {
            ApplicationError::not_found("Inline content is unavailable in this conversation.")
        })
    }
    pub async fn watch_inline_ui(
        &self,
        session_id: i64,
        reference: &str,
    ) -> Result<agena_runtime::plugins::inline_ui::InlineUiSubscription, ApplicationError> {
        let subscription = agena_runtime::plugins::inline_ui::InlineUiStore::default()
            .subscribe(session_id, reference)
            .map_err(|error| ApplicationError::internal_error(&error))?;
        self.read_inline_ui_reference(session_id, reference).await?;
        Ok(subscription)
    }
}

#[cfg(test)]
mod tests;
