use super::*;

pub(super) fn contains_case_insensitive(text: &str, query: &str) -> bool {
    let trimmed = query.trim();
    !trimmed.is_empty()
        && text
            .to_lowercase()
            .contains(trimmed.to_lowercase().as_str())
}

pub(super) fn find_search_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }

    let mut ranges = Vec::new();
    let mut search_start = 0;
    while search_start < text.len() {
        let Some((start, end)) = find_query_match_from(text, query, search_start) else {
            break;
        };
        ranges.push(start..end);
        search_start = if end > start {
            end
        } else {
            next_grapheme_boundary(text, start)
        };
    }
    ranges
}

pub(super) fn find_query_match_from(
    text: &str,
    query: &str,
    start_at: usize,
) -> Option<(usize, usize)> {
    if query.is_ascii() {
        let mut starts = text[start_at..]
            .char_indices()
            .map(|(offset, _)| start_at + offset)
            .collect::<Vec<_>>();
        if !starts.contains(&text.len()) {
            starts.push(text.len());
        }
        for start in starts {
            let end = start.saturating_add(query.len());
            if let Some(slice) = text.get(start..end)
                && slice.eq_ignore_ascii_case(query)
            {
                return Some((start, end));
            }
        }
        None
    } else {
        text[start_at..]
            .find(query)
            .map(|offset| start_at + offset)
            .map(|start| (start, start + query.len()))
    }
}

pub(super) fn next_grapheme_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    let grapheme = text[index..].graphemes(true).next().unwrap_or_default();
    index + grapheme.len()
}
