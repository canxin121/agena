//! UI-only headline localization for parts served to a client.
//!
//! A part keeps its stored English presentation: the model-facing contract and
//! older clients read the same string. A client that asks for a language gets
//! the known headline vocabulary mapped at the serve boundary, so the part
//! cache stays language-free and a request never mutates stored facts.

use agena_api::live::{SessionChangeResource, SessionPartsResource};
use agena_api::part::PartResource;
use agena_domain::PartDocument;

/// Localize the human headline of every part that carries a presentation.
pub fn localize_part_headlines(parts: &mut [PartResource], locale: Option<&str>) {
    let Some(locale) = requested_locale(locale) else {
        return;
    };
    for part in parts.iter_mut() {
        localize_part_with(part, locale);
    }
}

/// Localize one part that is served on its own: a live push, a live snapshot
/// row, or the presentation section of a single tool call.
pub fn localize_part(part: &mut PartResource, locale: Option<&str>) {
    let Some(locale) = requested_locale(locale) else {
        return;
    };
    localize_part_with(part, locale);
}

/// Localize the parts of a live session snapshot. Run summaries carry no
/// presentation, so the part list is the whole human-facing payload.
pub fn localize_session_parts(snapshot: &mut SessionPartsResource, locale: Option<&str>) {
    localize_part_headlines(&mut snapshot.parts, locale);
}

/// Localize the part carried by a live session change. Removals and session
/// metadata carry no stored headline, so they pass through unchanged.
pub fn localize_session_change(change: &mut SessionChangeResource, locale: Option<&str>) {
    let Some(locale) = requested_locale(locale) else {
        return;
    };
    let part = match change {
        SessionChangeResource::PartAdded { part, .. }
        | SessionChangeResource::PartUpdated { part, .. } => part,
        _ => return,
    };
    localize_part_with(part, locale);
}

/// Localize one tool detail document (for example the on-demand presentation
/// section of a tool call).
pub fn localize_part_document(document: &mut PartDocument, locale: Option<&str>) {
    let Some(locale) = requested_locale(locale) else {
        return;
    };
    localize_document(document, locale);
}

fn localize_part_with(part: &mut PartResource, locale: &str) {
    if let Some(presentation) = part.presentation.as_mut() {
        localize_document(presentation, locale);
    }
}

fn localize_document(document: &mut PartDocument, locale: &str) {
    document.title = agena_tool::localize_tool_title(document.title.as_str(), locale);
}

fn requested_locale(locale: Option<&str>) -> Option<&str> {
    locale.map(str::trim).filter(|locale| !locale.is_empty())
}
