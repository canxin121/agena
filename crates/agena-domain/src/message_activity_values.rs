use crate::FileChangeKind;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Text content part.
pub struct TextPart {
    pub text: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub synthetic: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Reasoning content part (summary, raw, or encrypted content).
pub struct ReasoningPart {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub summary: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub raw_content: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encrypted_content: Option<String>,
}

impl ReasoningPart {
    pub fn summary_text(&self) -> String {
        self.summary.concat()
    }

    pub fn raw_text(&self) -> String {
        self.raw_content.concat()
    }

    pub fn preferred_text(&self) -> String {
        if self.summary.is_empty() {
            self.raw_text()
        } else {
            self.summary_text()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Record of a file change (create, edit, delete, rename).
pub struct FileChangeRecord {
    pub path: String,
    pub kind: FileChangeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Web search result entry.
pub struct WebSearchResult {
    pub title: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Error content part with a user-facing problem.
pub struct ErrorPart {
    pub problem: agena_failure::UserProblem,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use super::{ReasoningPart, TextPart};

    #[test]
    fn preserves_compact_text_json_and_prefers_summary_reasoning() {
        let text = TextPart {
            text: "hello".into(),
            synthetic: false,
        };
        assert_eq!(serde_json::to_string(&text).unwrap(), r#"{"text":"hello"}"#);

        let reasoning = ReasoningPart {
            summary: vec!["sum".into(), "mary".into()],
            raw_content: vec!["raw".into()],
            encrypted_content: None,
        };
        assert_eq!(reasoning.summary_text(), "summary");
        assert_eq!(reasoning.preferred_text(), "summary");
    }
}
