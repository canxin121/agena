//! Runtime-neutral session projections and text-search helpers.
//!
//! The application owns persistence, routing, and terminal effects. These
//! helpers only transform API/domain values or search text, so they belong to
//! the session presentation owner.

use std::ops::Range;

use agena_api::resource::SessionExecutionContextResource;
use agena_domain::{ModelRef, UserInputQuestion};
use agena_tui::user_input::UserInputAnswerDraft;
use unicode_segmentation::UnicodeSegmentation;

pub fn model_name_status_label(model: &ModelRef) -> String {
    model.model_id.to_string()
}

pub fn execution_model_status_label(execution: &SessionExecutionContextResource) -> Option<String> {
    let provider_id = execution
        .model_provider_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let adapter_id = execution
        .model_adapter_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let model_id = execution
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if provider_id.is_none() && model_id.is_none() {
        return None;
    }

    Some(match adapter_id {
        Some(adapter_id) => format!(
            "{}/{}/{}",
            provider_id.unwrap_or("auto"),
            adapter_id,
            model_id.unwrap_or("default")
        ),
        None => format!(
            "{}/{}",
            provider_id.unwrap_or("auto"),
            model_id.unwrap_or("default")
        ),
    })
}

pub fn execution_model_name_status_label(
    execution: &SessionExecutionContextResource,
) -> Option<String> {
    execution
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

pub fn user_input_answer_values(
    question: &UserInputQuestion,
    draft: &UserInputAnswerDraft,
) -> Vec<String> {
    let mut values = draft
        .option_indexes
        .iter()
        .filter_map(|index| question.options.get(*index))
        .filter(|option| !option.custom)
        .map(|option| option.label.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if question
        .custom_option_index()
        .is_some_and(|custom| draft.option_indexes.contains(&custom))
    {
        values.extend(
            draft
                .custom_values
                .iter()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
        );
    }
    if question.multiple {
        values
    } else {
        values.into_iter().take(1).collect()
    }
}

pub fn user_input_question_label(question: &UserInputQuestion) -> &str {
    let header = question.header.trim();
    if !header.is_empty() {
        header
    } else {
        question.question.as_str()
    }
}

/// One normalized query reused across all transcript rows. Matching and
/// highlighting share the same Unicode lowercase semantics and byte ranges.
pub struct PreparedSearch {
    query: String,
}

impl PreparedSearch {
    pub fn new(query: &str) -> Self {
        Self {
            query: query.trim().to_lowercase(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.query.is_empty()
    }

    pub fn contains(&self, text: &str) -> bool {
        !self.is_empty() && lowercase_for_search(text).contains(&self.query)
    }

    pub fn ranges(&self, text: &str) -> Vec<Range<usize>> {
        if self.is_empty() {
            return Vec::new();
        }
        let lowered = lowercase_for_search(text);
        if text.is_ascii() {
            return lowered
                .match_indices(&self.query)
                .map(|(start, matched)| start..start + matched.len())
                .collect();
        }
        // Unicode lowercase may change UTF-8 length (Kelvin sign) or expand
        // one source char (İ). Map monotonically through original characters
        // once, rather than collecting/rescanning all suffix offsets per hit.
        // Contextual lowercase sigma changes its value, not its byte length.
        let mut original = text.char_indices();
        let mut lowered_end = 0usize;
        let mut current = 0..0;
        let mut ranges: Vec<Range<usize>> = Vec::new();
        for (start, matched) in lowered.match_indices(&self.query) {
            while lowered_end <= start {
                let Some((offset, ch)) = original.next() else {
                    return ranges;
                };
                current = offset..offset + ch.len_utf8();
                lowered_end += ch.to_lowercase().map(char::len_utf8).sum::<usize>();
            }
            let source_start = current.start;
            let end = start + matched.len();
            while lowered_end < end {
                let Some((offset, ch)) = original.next() else {
                    return ranges;
                };
                current = offset..offset + ch.len_utf8();
                lowered_end += ch.to_lowercase().map(char::len_utf8).sum::<usize>();
            }
            if let Some(previous) = ranges.last_mut()
                && previous.end > source_start
            {
                previous.end = current.end;
            } else {
                ranges.push(source_start..current.end);
            }
        }
        ranges
    }
}

fn lowercase_for_search(text: &str) -> std::borrow::Cow<'_, str> {
    if text.is_ascii() && !text.bytes().any(|byte| byte.is_ascii_uppercase()) {
        std::borrow::Cow::Borrowed(text)
    } else {
        std::borrow::Cow::Owned(text.to_lowercase())
    }
}

pub fn contains_case_insensitive(text: &str, query: &str) -> bool {
    PreparedSearch::new(query).contains(text)
}

pub fn find_search_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    PreparedSearch::new(query).ranges(text)
}

pub fn find_query_match_from(text: &str, query: &str, start_at: usize) -> Option<(usize, usize)> {
    let start_at = snap_to_char_boundary(text, start_at);
    let search = PreparedSearch::new(query);
    if search.is_empty() {
        return None;
    }
    let source = &text[start_at..];
    let lowered = lowercase_for_search(source);
    let start = lowered.find(&search.query)?;
    let end = start + search.query.len();
    if source.is_ascii() {
        return Some((start_at + start, start_at + end));
    }
    let mut lowered_offset = 0usize;
    let mut source_start = None;
    for (offset, ch) in source.char_indices() {
        lowered_offset += ch.to_lowercase().map(char::len_utf8).sum::<usize>();
        if source_start.is_none() && lowered_offset > start {
            source_start = Some(offset);
        }
        if lowered_offset >= end {
            return Some((start_at + source_start?, start_at + offset + ch.len_utf8()));
        }
    }
    None
}

pub fn next_grapheme_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    let grapheme = text[index..].graphemes(true).next().unwrap_or_default();
    index + grapheme.len()
}

/// Back the byte index off to the nearest UTF-8 char boundary (0 when it
/// lands inside a multi-byte char), so text slicing never panics.
fn snap_to_char_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

#[cfg(test)]
mod search_tests {
    use super::*;

    #[test]
    fn matching_and_highlighting_agree_for_unicode_expansion_and_context() {
        for (text, query, expected) in [
            ("ÉCLAIR éclair", "é", vec!["É", "é"]),
            ("İSTANBUL", "i", vec!["İ"]),
            ("Kelvin k", "K", vec!["K", "k"]),
            ("ΟΣ ΟΣΑ", "ος", vec!["ΟΣ"]),
            ("中a中A", "a", vec!["a", "A"]),
            ("aaaaa", "aa", vec!["aa", "aa"]),
            ("👩‍💻A👩‍💻", "👩‍💻", vec!["👩‍💻", "👩‍💻"]),
        ] {
            let search = PreparedSearch::new(query);
            let ranges = search.ranges(text);
            assert_eq!(
                search.contains(text),
                !ranges.is_empty(),
                "{text:?}/{query:?}"
            );
            assert_eq!(
                ranges
                    .iter()
                    .map(|range| &text[range.clone()])
                    .collect::<Vec<_>>(),
                expected
            );
            assert!(ranges.windows(2).all(|pair| pair[0].end <= pair[1].start));
            assert_eq!(
                find_query_match_from(text, query, 0),
                ranges.first().map(|range| (range.start, range.end))
            );
        }
        assert!(PreparedSearch::new(" \n ").ranges("abc").is_empty());
        assert!(!PreparedSearch::new(" \n ").contains("abc"));
        assert_eq!(find_query_match_from("éA", "a", 1), Some((2, 3)));
    }

    #[test]
    fn dense_matches_are_ordered_without_suffix_offset_tables() {
        let source = "a".repeat(32 * 1024);
        let ranges = find_search_ranges(&source, "A");
        assert_eq!(ranges.len(), source.len());
        assert!(
            ranges
                .iter()
                .enumerate()
                .all(|(index, range)| *range == (index..index + 1))
        );
        assert_eq!(find_query_match_from(&source, "A", 0), Some((0, 1)));
    }

    #[test]
    fn lowercase_offsets_remain_valid_for_all_unicode_case_mappings() {
        // Check every character with nontrivial lowercase on both sides of a
        // match; this catches UTF-8 shortening/expansion without a lookup table.
        for value in 0..=0x10ffff {
            let Some(ch) = char::from_u32(value) else {
                continue;
            };
            let lower = ch.to_lowercase().collect::<String>();
            if lower == ch.to_string() || lower.trim().is_empty() {
                continue;
            }
            let text = format!("x{ch}x{ch}");
            let search = PreparedSearch::new(&lower);
            for range in search.ranges(&text) {
                assert!(text.get(range.clone()).is_some(), "U+{value:X}: {range:?}");
            }
            assert!(search.contains(&text));
        }
    }
}
