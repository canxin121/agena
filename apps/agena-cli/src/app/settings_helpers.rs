use super::*;

pub(super) fn format_setting_value_inline(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => "unset".to_string(),
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Number(value) => value.to_string(),
        JsonValue::String(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                "\"\"".to_string()
            } else if trimmed.chars().count() > 64 {
                format!("\"{}…\"", trimmed.chars().take(64).collect::<String>())
            } else {
                format!("\"{trimmed}\"")
            }
        }
        other => {
            let rendered = other.to_string();
            if rendered.chars().count() > 72 {
                format!("{}…", rendered.chars().take(72).collect::<String>())
            } else {
                rendered
            }
        }
    }
}

pub(super) fn setting_value_input_text(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => String::new(),
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Number(value) => value.to_string(),
        JsonValue::String(value) => value.clone(),
        other => other.to_string(),
    }
}
