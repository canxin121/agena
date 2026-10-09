//! UI-only headline localization for parts served to a client.
//!
//! A part keeps its stored English presentation: the model-facing contract and
//! older clients read the same string. A client that asks for a language gets
//! the known headline vocabulary mapped at the serve boundary, so the part
//! cache stays language-free and a request never mutates stored facts.

use agena_api::part::PartResource;
use agena_domain::PartDocument;

/// Localize the human headline of every part that carries a presentation.
pub fn localize_part_headlines(parts: &mut [PartResource], locale: Option<&str>) {
    let Some(locale) = requested_locale(locale) else {
        return;
    };
    for part in parts.iter_mut() {
        if let Some(presentation) = part.presentation.as_mut() {
            localize_document(presentation, locale);
        }
    }
}

/// Localize one tool detail document (for example the on-demand presentation
/// section of a tool call).
pub fn localize_part_document(document: &mut PartDocument, locale: Option<&str>) {
    let Some(locale) = requested_locale(locale) else {
        return;
    };
    localize_document(document, locale);
}

fn localize_document(document: &mut PartDocument, locale: &str) {
    document.title = agena_tool::localize_tool_title(document.title.as_str(), locale);
}

fn requested_locale(locale: Option<&str>) -> Option<&str> {
    locale.map(str::trim).filter(|locale| !locale.is_empty())
}

#[cfg(test)]
mod tests {
    use super::localize_part_headlines;
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
        localize_part_headlines(&mut parts, Some("de-DE"));
        assert_eq!(
            parts[0]
                .presentation
                .as_ref()
                .map(|presentation| presentation.title.as_str()),
            Some("Execute command · cargo test · passed")
        );
    }
}
