//! UI-only translations for composed tool headlines.
//!
//! `agena_tool` composes English headlines such as
//! `Execute command · cargo test · passed` and stores them with the durable
//! transcript facts. A client renders the same headline in its own language by
//! mapping the known vocabulary segment by segment: action phrases, status and
//! fact phrases, and count units. Dynamic values (paths, commands, ids, codes)
//! are never translated, and a locale without a vocabulary keeps the English
//! headline, so a missing translation never blanks a row.
//!
//! Every locale file pairs the English key with its translation and keeps the
//! canonical key sets below.

mod locale_ar;
mod locale_de;
mod locale_es;
mod locale_fr;
mod locale_hi;
mod locale_ja;
mod locale_ko;
mod locale_pt;
mod locale_zh;

/// Canonical action keys, in the order the English table declares them.
pub const ACTION_KEYS: &[&str] = &[
    "Start agent",
    "Message agent",
    "Wait for agents",
    "Continue agent",
    "Cancel agent",
    "Read",
    "Read files",
    "Write",
    "Apply patch",
    "Find files",
    "Search files",
    "Replace text",
    "Inspect file",
    "Search AST",
    "Inspect syntax tree",
    "Rewrite AST",
    "Read document",
    "Run command",
    "Execute command",
    "Spawn background command",
    "Watch command output",
    "Open interactive terminal",
    "Read terminal output",
    "List processes",
    "Show process logs",
    "Stop process",
    "Interact with terminal",
    "Resize terminal",
    "Signal terminal",
    "Start monitor",
    "Stop monitor",
    "Search tools",
    "List tools",
    "Read tool help",
    "List tool tags",
    "Call tool",
    "List plugins",
    "Search plugins",
    "List plugin tags",
    "Ask user",
    "Send notification",
    "Fetch page",
    "Read pages",
    "Continue reading page",
    "Search crawled pages",
    "Read memory",
    "Save memory",
    "Delete memory",
    "Update settings",
    "Reset settings",
    "Edit text",
    "Edit image",
    "Rename session",
    "Open page",
    "View image",
    "Inspect symbol",
    "Create",
    "Update",
    "Delete",
];

/// Canonical status and fact keys.
pub const FACT_KEYS: &[&str] = &[
    "passed",
    "failed",
    "cancelled",
    "declined",
    "timed out",
    "completed",
    "running",
    "queued",
    "pending",
    "permission denied",
    "response received",
    "image response received",
    "more available",
    "truncated",
    "no output",
    "No output.",
    "search was partial or truncated",
    "1 file changed",
];

/// Canonical count-unit keys. Each key starts with the space that separates it
/// from the count in an English headline.
pub const UNIT_KEYS: &[&str] = &[
    " lines",
    " items",
    " matches",
    " files",
    " tools",
    " chunks",
    " events",
    " paths",
    " results",
    " packages",
    " steps",
    " tasks",
    " sources",
    " sessions",
    " servers",
    " tools available",
];

/// Action phrases that are composed as `Action subject` instead of
/// `Action · subject`; mirrors `action_is_noun_phrase_with_subject`.
const SUBJECT_ACTIONS: &[&str] = &[
    "Read",
    "Write",
    "Create",
    "Update",
    "Delete",
    "Open page",
    "Fetch page",
    "View image",
    "Inspect file",
    "Inspect symbol",
    "Read memory",
    "Save memory",
    "Delete memory",
    "Update settings",
    "Reset settings",
    "Edit text",
    "Edit image",
    "Rename session",
];

/// Separator used by [`crate::compose_tool_title`] between headline parts.
const SEGMENT_SEPARATOR: &str = " · ";

struct Vocabulary {
    actions: &'static [(&'static str, &'static str)],
    facts: &'static [(&'static str, &'static str)],
    units: &'static [(&'static str, &'static str)],
    /// Exit-code label with a `{code}` placeholder.
    exit_code: &'static str,
}

macro_rules! vocabulary {
    ($tag:literal, $actions:path, $facts:path, $units:path, $exit_code:path) => {
        (
            $tag,
            Vocabulary {
                actions: $actions,
                facts: $facts,
                units: $units,
                exit_code: $exit_code,
            },
        )
    };
}

const LOCALES: &[(&str, Vocabulary)] = &[
    vocabulary!(
        "zh-CN",
        locale_zh::simplified::ACTIONS,
        locale_zh::simplified::FACTS,
        locale_zh::simplified::UNITS,
        locale_zh::simplified::EXIT_CODE
    ),
    vocabulary!(
        "zh-TW",
        locale_zh::traditional::ACTIONS,
        locale_zh::traditional::FACTS,
        locale_zh::traditional::UNITS,
        locale_zh::traditional::EXIT_CODE
    ),
    vocabulary!(
        "ja-JP",
        locale_ja::ACTIONS,
        locale_ja::FACTS,
        locale_ja::UNITS,
        locale_ja::EXIT_CODE
    ),
    vocabulary!(
        "ko-KR",
        locale_ko::ACTIONS,
        locale_ko::FACTS,
        locale_ko::UNITS,
        locale_ko::EXIT_CODE
    ),
    vocabulary!(
        "fr-FR",
        locale_fr::ACTIONS,
        locale_fr::FACTS,
        locale_fr::UNITS,
        locale_fr::EXIT_CODE
    ),
    vocabulary!(
        "de-DE",
        locale_de::ACTIONS,
        locale_de::FACTS,
        locale_de::UNITS,
        locale_de::EXIT_CODE
    ),
    vocabulary!(
        "es-ES",
        locale_es::ACTIONS,
        locale_es::FACTS,
        locale_es::UNITS,
        locale_es::EXIT_CODE
    ),
    vocabulary!(
        "hi-IN",
        locale_hi::ACTIONS,
        locale_hi::FACTS,
        locale_hi::UNITS,
        locale_hi::EXIT_CODE
    ),
    vocabulary!(
        "ar-SA",
        locale_ar::ACTIONS,
        locale_ar::FACTS,
        locale_ar::UNITS,
        locale_ar::EXIT_CODE
    ),
    vocabulary!(
        "pt-BR",
        locale_pt::ACTIONS,
        locale_pt::FACTS,
        locale_pt::UNITS,
        locale_pt::EXIT_CODE
    ),
];

/// Translate a composed tool headline into `locale`.
pub fn localize_tool_title(title: &str, locale: &str) -> String {
    let Some(vocabulary) = vocabulary_for(locale) else {
        return title.to_owned();
    };
    title
        .split(SEGMENT_SEPARATOR)
        .map(|part| localize_segment(vocabulary, part.trim()))
        .collect::<Vec<_>>()
        .join(SEGMENT_SEPARATOR)
}

fn vocabulary_for(locale: &str) -> Option<&'static Vocabulary> {
    let normalized = locale.trim().replace('_', "-").to_ascii_lowercase();
    let language = normalized.split('-').next().unwrap_or_default();
    let canonical = match language {
        "zh" if is_traditional(&normalized) => "zh-TW",
        "zh" => "zh-CN",
        "ja" => "ja-JP",
        "ko" => "ko-KR",
        "fr" => "fr-FR",
        "de" => "de-DE",
        "es" => "es-ES",
        "hi" => "hi-IN",
        "ar" => "ar-SA",
        "pt" => "pt-BR",
        _ => return None,
    };
    LOCALES
        .iter()
        .find_map(|(tag, vocabulary)| (*tag == canonical).then_some(vocabulary))
}

/// Region and script decide between the two Chinese vocabularies; a bare `zh`
/// follows the repository default of simplified Chinese.
fn is_traditional(normalized: &str) -> bool {
    normalized.contains("hant")
        || normalized.contains("tw")
        || normalized.contains("hk")
        || normalized.contains("mo")
}

/// Localize one ` · `-separated headline segment.
fn localize_segment(vocabulary: &Vocabulary, segment: &str) -> String {
    if segment.is_empty() {
        return String::new();
    }
    if let Some(translated) = lookup(vocabulary.actions, segment) {
        return translated.to_owned();
    }
    if let Some(translated) = subject_action(vocabulary, segment) {
        return translated;
    }
    if let Some(translated) = lookup(vocabulary.facts, segment) {
        return translated.to_owned();
    }
    if let Some(translated) = exit_code(vocabulary, segment) {
        return translated;
    }
    if let Some(translated) = counted_unit(vocabulary, segment) {
        return translated;
    }
    segment.to_owned()
}

fn lookup(pairs: &'static [(&'static str, &'static str)], key: &str) -> Option<&'static str> {
    pairs
        .iter()
        .find_map(|(english, localized)| (*english == key).then_some(*localized))
}

/// `Read README.md` keeps its subject and localizes the action.
fn subject_action(vocabulary: &Vocabulary, segment: &str) -> Option<String> {
    SUBJECT_ACTIONS.iter().find_map(|english| {
        let rest = segment.strip_prefix(english)?.strip_prefix(' ')?;
        if rest.trim().is_empty() {
            return None;
        }
        let action = lookup(vocabulary.actions, english)?;
        Some(format!("{action} {rest}"))
    })
}

/// `exit 3` keeps the code and localizes the label.
fn exit_code(vocabulary: &Vocabulary, segment: &str) -> Option<String> {
    let code = segment.strip_prefix("exit ")?.trim();
    if code.is_empty() || !code.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    Some(vocabulary.exit_code.replace("{code}", code))
}

/// `12 lines`, `2/3 tools`, `36 matches` keep the count and localize the unit
/// word.
fn counted_unit(vocabulary: &Vocabulary, segment: &str) -> Option<String> {
    for (unit, localized) in vocabulary.units {
        let Some(count) = segment.strip_suffix(unit) else {
            continue;
        };
        if !is_count(count.trim_end()) {
            continue;
        }
        return Some(format!("{}{localized}", count.trim_end()));
    }
    None
}

/// Whether a fragment is a plain count: digits with `/`, `.`, `%`, `–` or a
/// trailing `+`.
fn is_count(fragment: &str) -> bool {
    let fragment = fragment.trim();
    if fragment.is_empty() {
        return false;
    }
    let mut digits = 0usize;
    for character in fragment.chars() {
        match character {
            '0'..='9' => digits += 1,
            '/' | '.' | '%' | '-' | '+' | '\u{2013}' | '~' => {}
            _ => return false,
        }
    }
    digits > 0
}
