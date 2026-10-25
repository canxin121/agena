use super::*;

// Multipart APIs use bracketed object keys and repeated array fields rather
// than a JSON string in one field. Explicit bracketed keys remain supported.
pub(super) fn fields(object: &serde_json::Map<String, Value>) -> Result<Vec<(String, String)>> {
    fn append(
        out: &mut Vec<(String, String)>,
        key: String,
        value: &Value,
        depth: usize,
    ) -> Result<()> {
        if depth > 16 || out.len() >= 4096 {
            return Err(invalid("multipart parameters exceed nesting/field limits"));
        }
        match value {
            Value::Null => {}
            Value::Array(values) => {
                for (i, value) in values.iter().enumerate() {
                    let nested = if value.is_object() || value.is_array() {
                        format!("{key}[{i}]")
                    } else if key.ends_with("[]") {
                        key.clone()
                    } else {
                        format!("{key}[]")
                    };
                    append(out, nested, value, depth + 1)?;
                }
            }
            Value::Object(values) => {
                for (name, value) in values {
                    append(out, format!("{key}[{name}]"), value, depth + 1)?;
                }
            }
            Value::String(value) => out.push((key, value.clone())),
            _ => out.push((key, value.to_string())),
        }
        Ok(())
    }
    let mut result = Vec::new();
    for (key, value) in object {
        append(&mut result, key.clone(), value, 0)?;
    }
    Ok(result)
}

pub(super) fn file_mime(filename: &str) -> &'static str {
    match std::path::Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "wav" => "audio/wav",
        "mp3" | "mpeg" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "aac" => "audio/aac",
        "flac" => "audio/flac",
        "webm" => "audio/webm",
        "m4a" | "mp4" => "audio/mp4",
        "pdf" => "application/pdf",
        "jsonl" => "application/jsonl",
        "json" => "application/json",
        "txt" | "md" | "csv" => "text/plain",
        _ => "application/octet-stream",
    }
}
