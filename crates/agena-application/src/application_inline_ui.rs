//! Read-time expansion of immutable reply references. Every lookup uses the
//! current session's membership capability (including forks/rewinds), never a
//! process-global UI registry. Reads do not invoke plugins or write anything.
use crate::{Application, ApplicationError};
use agena_domain::{
    InlineDocument, InlineUiReferences, PreparedInlineUi, inline_reference_candidates,
    inline_reference_tool_id, is_inline_ui_prepare_tool,
};
use agena_storage::kernel::{KernelError, KernelReader, SessionId, ToolPhase};

pub(crate) async fn resolve_inline_reference(
    reader: &KernelReader,
    session: SessionId,
    reference: &str,
) -> Result<Option<InlineDocument>, KernelError> {
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
    Ok(Some(prepared.document))
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
}

#[cfg(test)]
mod tests;
