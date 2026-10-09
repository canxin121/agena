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

#[cfg(test)]
mod tests {
    use super::{
        localize_part, localize_part_headlines, localize_session_change, localize_session_parts,
    };
    use agena_api::live::{SessionChangeResource, SessionPartsResource};
    use agena_api::part::PartResource;

    fn tool_part(title: &str) -> PartResource {
        let mut part = PartResource {
            part_id: 1,
            kind: "tool_call".to_owned(),
            ..PartResource::default()
        };
        part.presentation = Some(agena_domain::PartDocument {
            title: title.to_owned(),
            summary: String::new(),
            blocks: Vec::new(),
        });
        part
    }

    #[test]
    fn a_requested_language_localizes_the_headline_only() {
        let mut parts = vec![tool_part("Execute command · cargo test · passed")];
        localize_part_headlines(&mut parts, Some("zh-CN"));
        assert_eq!(
            parts[0]
                .presentation
                .as_ref()
                .map(|presentation| presentation.title.as_str()),
            Some("执行命令 · cargo test · 通过")
        );
    }

    #[test]
    fn an_absent_or_unsupported_language_keeps_the_stored_headline() {
        let mut parts = vec![tool_part("Execute command · cargo test · passed")];
        localize_part_headlines(&mut parts, None);
        localize_part_headlines(&mut parts, Some("  "));
        localize_part_headlines(&mut parts, Some("nl-NL"));
        assert_eq!(
            parts[0]
                .presentation
                .as_ref()
                .map(|presentation| presentation.title.as_str()),
            Some("Execute command · cargo test · passed")
        );
    }

    #[test]
    fn a_live_push_localizes_the_part_it_carries() {
        let mut change = SessionChangeResource::PartAdded {
            session_id: 4,
            part: Box::new(tool_part("Execute command · cargo test · passed")),
        };
        localize_session_change(&mut change, Some("zh-CN"));
        let SessionChangeResource::PartAdded { part, .. } = &change else {
            panic!("the part push should stay a part push");
        };
        assert_eq!(
            part.presentation
                .as_ref()
                .map(|presentation| presentation.title.as_str()),
            Some("执行命令 · cargo test · 通过")
        );
    }

    #[test]
    fn a_change_without_a_headline_is_untouched() {
        let mut change = SessionChangeResource::PartRemoved {
            session_id: 4,
            part_id: 9,
        };
        localize_session_change(&mut change, Some("zh-CN"));
        assert!(matches!(
            change,
            SessionChangeResource::PartRemoved { part_id: 9, .. }
        ));
    }

    #[test]
    fn a_live_snapshot_localizes_every_part() {
        let mut snapshot = SessionPartsResource {
            session_id: 4,
            version: 9,
            parts: vec![tool_part("Execute command · cargo test · passed")],
            runs: Vec::new(),
            user_message_count: None,
            page: Default::default(),
        };
        localize_session_parts(&mut snapshot, Some("zh-CN"));
        assert_eq!(
            snapshot.parts[0]
                .presentation
                .as_ref()
                .map(|presentation| presentation.title.as_str()),
            Some("执行命令 · cargo test · 通过")
        );
    }

    #[test]
    fn an_absent_language_leaves_a_single_served_part_english() {
        let mut part = tool_part("Execute command · cargo test · passed");
        localize_part(&mut part, None);
        assert_eq!(
            part.presentation
                .as_ref()
                .map(|presentation| presentation.title.as_str()),
            Some("Execute command · cargo test · passed")
        );
    }
}
