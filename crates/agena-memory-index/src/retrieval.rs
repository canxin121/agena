use std::{collections::BTreeSet, iter::Peekable, ops::Range, str::CharIndices};

use crate::{DEFAULT_EXCERPT_CHARS, MemoryExcerpt, MemorySearchDocument, MemorySearchHit};

const MAX_QUERY_CHARS: usize = 4096;
const MAX_WORD_TERMS: usize = 64;
const MAX_CJK_TERMS: usize = 128;
const MAX_LITERAL_TERMS: usize = 16;
pub(crate) const MAX_WORD_BYTES: usize = 256;

/// Search input is literal text, never Tantivy's query language. Bound each
/// class independently so a long Chinese sentence cannot crowd out a code ID.
#[derive(Debug, Default)]
pub(crate) struct LiteralQuery {
    pub words: Vec<String>,
    pub cjk_ngrams: Vec<String>,
    pub literals: Vec<String>,
}

impl LiteralQuery {
    pub fn new(input: &str) -> Self {
        let input = lowercase(&bounded_query(input));
        let literals = input
            .split(|c: char| {
                is_cjk(c)
                    || !(c.is_alphanumeric()
                        || is_combining_mark(c)
                        || matches!(c, '/' | '\\' | ':' | '_' | '-' | '.'))
            })
            .map(|part| part.trim_end_matches('.'))
            .filter(|part| {
                part.len() <= MAX_WORD_BYTES
                    && part
                        .chars()
                        .any(|c| matches!(c, '/' | '\\' | ':' | '_' | '-' | '.'))
                    && (2..=16).contains(&WordSpans::new(part).count())
            })
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        // A precise identifier is a single retrieval unit. Its generic parts
        // must not independently make old_marker match new_marker.
        let literal_words = literals
            .iter()
            .flat_map(|literal| WordSpans::new(literal).map(|span| &literal[span]))
            .collect::<BTreeSet<_>>();
        let mut words = Vec::new();
        let mut grams = Vec::new();
        for span in WordSpans::new(&input) {
            let word = &input[span];
            if literal_words.contains(word) {
                continue;
            }
            if word.len() <= MAX_WORD_BYTES && !is_stop_word(word) {
                words.push(word.to_owned());
            }
            if word.chars().next().is_some_and(is_cjk) {
                let chars = word.chars().collect::<Vec<_>>();
                for start in 0..chars.len() {
                    for width in 2..=4 {
                        if let Some(end) =
                            start.checked_add(width).filter(|end| *end <= chars.len())
                        {
                            let gram = chars[start..end].iter().collect::<String>();
                            if !is_stop_word(&gram) {
                                grams.push(gram);
                            }
                        }
                    }
                }
            }
        }
        Self {
            words: bounded_unique(words, MAX_WORD_TERMS),
            cjk_ngrams: bounded_unique(grams, MAX_CJK_TERMS),
            literals: bounded_unique(literals, MAX_LITERAL_TERMS),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty() && self.cjk_ngrams.is_empty() && self.literals.is_empty()
    }

    pub fn evidence(&self, text: &str) -> (f32, Vec<String>) {
        let lower = lowercase(text);
        let mut matched = BTreeSet::new();
        let words = WordSpans::new(&lower)
            .map(|span| &lower[span])
            .collect::<Vec<_>>();
        for word in &words {
            if self
                .words
                .binary_search_by(|term| term.as_str().cmp(word))
                .is_ok()
            {
                matched.insert((*word).to_owned());
            }
        }
        for term in &self.cjk_ngrams {
            if lower.contains(term) {
                matched.insert(term.clone());
            }
        }
        for literal in &self.literals {
            let parts = WordSpans::new(literal)
                .map(|span| &literal[span])
                .collect::<Vec<_>>();
            if words
                .windows(parts.len())
                .any(|window| window == parts.as_slice())
            {
                matched.insert(literal.clone());
            }
        }
        let score = matched.iter().map(|term| self.term_weight(term)).sum();
        (score, matched.into_iter().collect())
    }

    /// Find a real source match inside a window, retaining original byte
    /// offsets even when Unicode lowercasing changes the number of bytes.
    fn anchor(&self, body: &str, window: Range<usize>) -> Option<Range<usize>> {
        let text = &body[window.clone()];
        let words = WordSpans::new(text)
            .map(|span| {
                let term = lowercase(&text[span.clone()]);
                (term, window.start + span.start..window.start + span.end)
            })
            .collect::<Vec<_>>();
        let mut best: Option<(f32, usize, Range<usize>)> = None;
        let mut consider = |term: &str, span: Range<usize>| {
            let weight = self.term_weight(term);
            let len = body[span.clone()].chars().count();
            if best.as_ref().is_none_or(|(old_weight, old_len, _)| {
                weight > *old_weight || (weight == *old_weight && len > *old_len)
            }) {
                best = Some((weight, len, span));
            }
        };
        for (term, span) in &words {
            if self.words.binary_search(term).is_ok()
                && word_boundary(body, span.start)
                && word_boundary(body, span.end)
            {
                consider(term, span.clone());
            }
        }
        for term in &self.cjk_ngrams {
            if let Some(offset) = text.find(term) {
                consider(
                    term,
                    window.start + offset..window.start + offset + term.len(),
                );
            }
        }
        for literal in &self.literals {
            let parts = WordSpans::new(literal)
                .map(|span| &literal[span])
                .collect::<Vec<_>>();
            if let Some(matched) = words.windows(parts.len()).find(|candidate| {
                candidate
                    .iter()
                    .map(|(word, _)| word.as_str())
                    .eq(parts.iter().copied())
                    && word_boundary(body, candidate[0].1.start)
                    && word_boundary(body, candidate[candidate.len() - 1].1.end)
            }) {
                consider(
                    literal,
                    matched[0].1.start..matched[matched.len() - 1].1.end,
                );
            }
        }
        best.map(|(_, _, span)| span)
    }

    fn term_weight(&self, term: &str) -> f32 {
        if self
            .literals
            .binary_search_by(|literal| literal.as_str().cmp(term))
            .is_ok()
        {
            8.0
        } else if self
            .words
            .binary_search_by(|word| word.as_str().cmp(term))
            .is_ok()
        {
            4.0
        } else {
            // A longer shared CJK span carries more evidence than a bigram.
            term.chars().count().saturating_sub(1) as f32
        }
    }

    pub fn hit(&self, document: MemorySearchDocument, score: f32) -> MemorySearchHit {
        let excerpt = excerpt(&document.body, self, DEFAULT_EXCERPT_CHARS);
        let (_, matched_terms) = self.evidence(&format!(
            "{}\n{}\n{}\n{}",
            document.name, document.description, document.path, excerpt.text
        ));
        MemorySearchHit {
            document,
            score,
            excerpt,
            matched_terms,
        }
    }
}

/// Keep both ends: context builders often put the newest question after older
/// context, while direct search callers normally put the important words first.
fn bounded_query(input: &str) -> String {
    let Some((head_end, _)) = input.char_indices().nth(MAX_QUERY_CHARS) else {
        return input.to_owned();
    };
    let half = MAX_QUERY_CHARS / 2;
    let head_end = input[..head_end]
        .char_indices()
        .nth(half)
        .map_or(head_end, |(offset, _)| offset);
    let tail_start = input
        .char_indices()
        .rev()
        .nth(half - 1)
        .map_or(head_end, |(offset, _)| offset);
    format!("{} {}", &input[..head_end], &input[tail_start..])
}

// Match Tantivy's LowerCaser, which folds each character independently (in
// particular it does not use str::to_lowercase's final-sigma special case).
pub(crate) fn lowercase(text: &str) -> String {
    if text.is_ascii() {
        text.to_ascii_lowercase()
    } else {
        text.chars().flat_map(char::to_lowercase).collect()
    }
}

fn bounded_unique(terms: Vec<String>, limit: usize) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut unique = terms
        .into_iter()
        .filter(|term| seen.insert(term.clone()))
        .collect::<Vec<_>>();
    if unique.len() > limit {
        let keep_head = limit / 2;
        let tail = unique.split_off(unique.len() - (limit - keep_head));
        unique.truncate(keep_head);
        unique.extend(tail);
    }
    unique.sort_unstable();
    unique
}

/// The same word boundaries are used by the index, portable search and snippet
/// selector. In particular, 数据库PostgreSQL contains two lexical words.
pub(crate) struct WordSpans<'a> {
    chars: Peekable<CharIndices<'a>>,
}

impl<'a> WordSpans<'a> {
    pub fn new(text: &'a str) -> Self {
        Self {
            chars: text.char_indices().peekable(),
        }
    }
}

impl Iterator for WordSpans<'_> {
    type Item = Range<usize>;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some((start, first)) = self.chars.next() {
            if !first.is_alphanumeric() {
                continue;
            }
            let script = is_cjk(first);
            let mut end = start + first.len_utf8();
            while let Some(&(offset, next)) = self.chars.peek() {
                if !is_combining_mark(next) && (!next.is_alphanumeric() || is_cjk(next) != script) {
                    break;
                }
                end = offset + next.len_utf8();
                self.chars.next();
            }
            return Some(start..end);
        }
        None
    }
}

pub(crate) fn is_cjk(character: char) -> bool {
    matches!(character as u32,
        0x3040..=0x30ff | 0x31f0..=0x31ff | 0x3400..=0x4dbf |
        0x4e00..=0x9fff | 0xac00..=0xd7af | 0xf900..=0xfaff |
        0x20000..=0x2fa1f | 0x30000..=0x323af
    )
}

fn is_combining_mark(character: char) -> bool {
    matches!(character as u32,
        0x0300..=0x036f | 0x1ab0..=0x1aff | 0x1dc0..=0x1dff |
        0x20d0..=0x20ff | 0x3099..=0x309a | 0xfe20..=0xfe2f
    )
}

pub(crate) fn cjk_text(text: &str) -> String {
    WordSpans::new(text)
        .map(|span| &text[span])
        .filter(|word| word.chars().next().is_some_and(is_cjk))
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_stop_word(word: &str) -> bool {
    matches!(
        word,
        "a" | "an"
            | "and"
            | "are"
            | "as"
            | "at"
            | "be"
            | "been"
            | "but"
            | "by"
            | "can"
            | "could"
            | "did"
            | "do"
            | "does"
            | "for"
            | "from"
            | "had"
            | "has"
            | "have"
            | "how"
            | "i"
            | "if"
            | "in"
            | "is"
            | "it"
            | "its"
            | "me"
            | "my"
            | "not"
            | "of"
            | "on"
            | "or"
            | "our"
            | "please"
            | "should"
            | "so"
            | "than"
            | "that"
            | "the"
            | "their"
            | "them"
            | "there"
            | "these"
            | "they"
            | "this"
            | "those"
            | "to"
            | "us"
            | "was"
            | "we"
            | "were"
            | "what"
            | "when"
            | "where"
            | "which"
            | "who"
            | "why"
            | "will"
            | "with"
            | "would"
            | "you"
            | "your"
            | "我们"
            | "你们"
            | "他们"
            | "这个"
            | "那个"
            | "这些"
            | "那些"
            | "什么"
            | "是什么"
            | "为什么"
            | "怎么"
            | "怎么样"
            | "如何"
            | "之前"
            | "以前"
            | "上次"
            | "今天"
            | "现在"
            | "一下"
            | "请问"
            | "告诉"
            | "告诉我"
            | "记得"
            | "还记得"
            | "是否"
            | "有没有"
    )
}

fn word_boundary(body: &str, offset: usize) -> bool {
    if offset == 0 || offset == body.len() {
        return true;
    }
    let before = body[..offset].chars().next_back().unwrap();
    let after = body[offset..].chars().next().unwrap();
    !is_combining_mark(before)
        && !is_combining_mark(after)
        && !(before.is_alphanumeric() && after.is_alphanumeric() && is_cjk(before) == is_cjk(after))
}

fn char_end(text: &str, start: usize, count: usize) -> usize {
    text[start..]
        .char_indices()
        .nth(count)
        .map_or(text.len(), |(offset, _)| start + offset)
}

pub(crate) fn excerpt(body: &str, query: &LiteralQuery, max_chars: usize) -> MemoryExcerpt {
    if body.is_empty() || max_chars == 0 {
        return MemoryExcerpt {
            text: String::new(),
            start_line: 0,
            end_line: 0,
        };
    }
    let mut best = 0..char_end(body, 0, max_chars);
    let mut best_score = 0.0;
    let mut best_len = usize::MAX;
    if !query.is_empty() {
        let mut consider = |candidate: Range<usize>| {
            let text = &body[candidate.clone()];
            let (score, _) = query.evidence(text);
            let chars = text.chars().count();
            if score > best_score || (score > 0.0 && score == best_score && chars < best_len) {
                best = candidate;
                best_score = score;
                best_len = chars;
            }
        };
        let mut paragraph_offset = 0;
        for paragraph in body.split_inclusive("\n\n") {
            let trimmed = paragraph.trim();
            let start = paragraph_offset + paragraph.len() - paragraph.trim_start().len();
            let end = start + trimmed.len();
            paragraph_offset += paragraph.len();
            if start == end {
                continue;
            }
            if trimmed.chars().take(max_chars.saturating_add(1)).count() <= max_chars {
                consider(start..end);
                continue;
            }
            // Under a small budget, start at the relevant line instead of
            // carrying a suffix of a very long, unrelated preceding line.
            let mut line_offset = start;
            for line in body[start..end].split_inclusive('\n') {
                let line_start = line_offset + line.len() - line.trim_start().len();
                let line_end = line_start + line.trim().len();
                line_offset += line.len();
                if line_start == line_end {
                    continue;
                }
                if line
                    .trim()
                    .chars()
                    .take(max_chars.saturating_add(1))
                    .count()
                    <= max_chars
                {
                    consider(line_start..line_end);
                    continue;
                }
                let mut window_start = line_start;
                while window_start < line_end {
                    let window_end = char_end(&body[..line_end], window_start, max_chars);
                    if let Some(anchor) = query.anchor(body, window_start..window_end) {
                        let before = max_chars / 4;
                        let context_start = if before == 0 {
                            anchor.start
                        } else {
                            body[line_start..anchor.start]
                                .char_indices()
                                .rev()
                                .nth(before - 1)
                                .map_or(line_start, |(offset, _)| line_start + offset)
                        };
                        let context_end = char_end(&body[..line_end], context_start, max_chars);
                        consider(context_start..context_end);
                    }
                    if window_end == line_end {
                        break;
                    }
                    // Overlap protects a phrase crossing a window boundary.
                    window_start = char_end(body, window_start, (max_chars / 2).max(1));
                }
            }
        }
    }
    let text = &body[best.clone()];
    let start_line = body[..best.start]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1;
    let end_line = start_line + text.bytes().filter(|byte| *byte == b'\n').count()
        - usize::from(text.ends_with('\n'));
    MemoryExcerpt {
        text: text.to_owned(),
        start_line,
        end_line,
    }
}

pub(crate) fn sort_hits(hits: &mut [MemorySearchHit]) {
    hits.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.document.name.cmp(&right.document.name))
            .then_with(|| left.document.id.cmp(&right.document.id))
    });
}
