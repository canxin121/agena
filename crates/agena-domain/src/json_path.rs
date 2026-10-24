//! Pure dotted JSON-path parsing and lookup values.
//!
//! This syntax is shared by settings presentation and Runtime configuration
//! editing, but does not depend on a concrete configuration schema, file, or
//! service implementation.

use serde_json::Value as JsonValue;

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
/// Error parsing or evaluating a JSON path.
pub enum JsonPathError {
    #[error("invalid settings path `{path}`: trailing escape")]
    TrailingEscape { path: String },
    #[error("invalid settings path `{path}`: unterminated quoted segment")]
    UnterminatedQuote { path: String },
    #[error("settings path segments must not be empty")]
    EmptySegment,
}

/// Parses a dotted path with single- or double-quoted path segments.
pub fn parse_json_path(path: &str) -> Result<Vec<String>, JsonPathError> {
    let input = path.trim();
    if input.is_empty() {
        return Ok(Vec::new());
    }
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut quoted_segment = false;
    for ch in input.chars() {
        if let Some(quote_char) = quote {
            if escaped {
                current.push(ch);
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == quote_char {
                quote = None;
                quoted_segment = true;
                continue;
            }
            current.push(ch);
            continue;
        }
        match ch {
            '.' => {
                push_path_segment(&mut segments, &current, quoted_segment)?;
                current.clear();
                quoted_segment = false;
            }
            '"' | '\'' if current.trim().is_empty() => {
                current.clear();
                quote = Some(ch);
            }
            other => current.push(other),
        }
    }
    if escaped {
        return Err(JsonPathError::TrailingEscape {
            path: path.to_owned(),
        });
    }
    if quote.is_some() {
        return Err(JsonPathError::UnterminatedQuote {
            path: path.to_owned(),
        });
    }
    push_path_segment(&mut segments, &current, quoted_segment)?;
    Ok(segments)
}

/// Returns the JSON value at a path, or `Null` when the path does not exist.
pub fn get_json_path(value: &JsonValue, path: Option<&str>) -> Result<JsonValue, JsonPathError> {
    let Some(path) = path.map(str::trim).filter(|path| !path.is_empty()) else {
        return Ok(value.clone());
    };
    let segments = parse_json_path(path)?;
    let mut cursor = value;
    for segment in segments {
        cursor = match cursor {
            JsonValue::Object(object) => match object.get(segment.as_str()) {
                Some(value) => value,
                None => return Ok(JsonValue::Null),
            },
            JsonValue::Array(items) => match segment
                .parse::<usize>()
                .ok()
                .and_then(|index| items.get(index))
            {
                Some(value) => value,
                None => return Ok(JsonValue::Null),
            },
            _ => return Ok(JsonValue::Null),
        };
    }
    Ok(cursor.clone())
}

/// Formats path segments using quotes only when a segment needs them.
pub fn format_json_path(segments: &[String]) -> String {
    segments
        .iter()
        .map(|segment| {
            if segment
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
            {
                segment.clone()
            } else {
                format!("\"{}\"", segment.replace('\\', "\\\\").replace('"', "\\\""))
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

fn push_path_segment(
    segments: &mut Vec<String>,
    segment: &str,
    quoted: bool,
) -> Result<(), JsonPathError> {
    let segment = if quoted {
        segment.to_owned()
    } else {
        segment.trim().to_owned()
    };
    if segment.is_empty() {
        return Err(JsonPathError::EmptySegment);
    }
    segments.push(segment);
    Ok(())
}
