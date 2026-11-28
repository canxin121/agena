use std::{collections::BTreeSet, sync::Arc};

use aho_corasick::{AhoCorasick, MatchKind};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{McpError, McpResult};

// The unescaped set of JavaScript's encodeURIComponent.
const URI_COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

/// Redactions bound to the credentials actually passed to one transport.
#[derive(Default, Clone)]
pub struct McpRedactions(Option<Arc<AhoCorasick>>);

impl McpRedactions {
    pub fn for_credentials(credentials: Vec<String>) -> McpResult<Self> {
        let mut patterns = BTreeSet::new();
        for value in credentials.into_iter().filter(|value| !value.is_empty()) {
            patterns.insert(utf8_percent_encode(&value, URI_COMPONENT).to_string());
            patterns
                .insert(url::form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>());
            patterns.insert(value);
        }
        if patterns.is_empty() {
            return Ok(Self::default());
        }
        let matcher = AhoCorasick::builder()
            .match_kind(MatchKind::LeftmostLongest)
            .build(patterns)
            .map_err(|_| {
                McpError::Malformed("MCP credential redaction exceeds its limit".into())
            })?;
        Ok(Self(Some(Arc::new(matcher))))
    }

    pub(crate) fn text(&self, value: String) -> String {
        let Some(matcher) = self.0.as_ref().filter(|matcher| matcher.is_match(&value)) else {
            return value;
        };
        let mut text = String::new();
        matcher.replace_all_with(&value, &mut text, |_, _, result| {
            result.push_str("[redacted]");
            true
        });
        text
    }

    pub(crate) fn payload<T: Serialize + DeserializeOwned>(&self, value: T) -> McpResult<T> {
        if self.0.is_none() {
            return Ok(value);
        }
        let mut wire = serde_json::to_value(value)?;
        self.redact_value(&mut wire);
        serde_json::from_value(wire).map_err(Into::into)
    }

    fn redact_value(&self, value: &mut Value) {
        match value {
            Value::String(text) => *text = self.text(std::mem::take(text)),
            Value::Array(items) => {
                for item in items {
                    self.redact_value(item);
                }
            }
            Value::Object(object) => {
                *object = std::mem::take(object)
                    .into_iter()
                    .map(|(key, mut value)| {
                        self.redact_value(&mut value);
                        (self.text(key), value)
                    })
                    .collect();
            }
            _ => {}
        }
    }

    pub(crate) fn error(&self, error: McpError) -> McpError {
        if self.0.is_none() {
            return error;
        }
        match error {
            McpError::Transport(text) => McpError::Transport(self.text(text)),
            McpError::Http(text) => McpError::Http(self.text(text)),
            McpError::Auth(text) => McpError::Auth(self.text(text)),
            McpError::Malformed(text) => McpError::Malformed(self.text(text)),
            McpError::Timeout(text) => McpError::Timeout(self.text(text)),
            McpError::Rpc { code, message } => McpError::Rpc {
                code,
                message: self.text(message),
            },
            McpError::Io(error) => McpError::Io(std::io::Error::new(
                error.kind(),
                self.text(error.to_string()),
            )),
            McpError::Serde(error) => McpError::Malformed(self.text(error.to_string())),
            other => other,
        }
    }
}

#[cfg(test)]
#[test]
fn redacts_overlapping_credentials_and_url_encoded_copies_in_payloads() {
    let redactions =
        McpRedactions::for_credentials(vec!["foo".into(), "foo/bar".into(), "x y!".into()])
            .unwrap();
    let payload = serde_json::json!({"foo/bar": ["foo", "foo%2Fbar", "x%20y!", "x+y%21"]});
    assert_eq!(
        redactions.payload(payload).unwrap(),
        serde_json::json!({"[redacted]": ["[redacted]", "[redacted]", "[redacted]", "[redacted]"]}),
    );
}
