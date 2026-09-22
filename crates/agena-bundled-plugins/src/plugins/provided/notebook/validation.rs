//! Notebook invariants and bounded serialization, checked before any file write.
use super::{MAX_NOTEBOOK_BYTES, PluginError, SdkResult};
use serde_json::Value;
use std::{collections::HashSet, io::Write};

pub(super) fn normalize_cell(
    cell: &mut serde_json::Map<String, Value>,
    kind: &str,
    preserve: bool,
) {
    if kind == "code" {
        cell.remove("attachments");
        if !preserve {
            cell.insert("outputs".into(), serde_json::json!([]));
            cell.insert("execution_count".into(), Value::Null);
        } else {
            cell.entry("outputs")
                .or_insert_with(|| serde_json::json!([]));
            cell.entry("execution_count").or_insert(Value::Null);
        }
    } else {
        cell.remove("outputs");
        cell.remove("execution_count");
        if kind != "markdown" {
            cell.remove("attachments");
        }
    }
}

fn invalid(message: impl Into<String>) -> PluginError {
    PluginError::invalid_params(message.into())
}
fn multiline(value: &Value) -> bool {
    value.is_string()
        || value
            .as_array()
            .is_some_and(|items| items.iter().all(Value::is_string))
}

pub(super) fn validate(notebook: &mut Value) -> SdkResult<()> {
    if notebook.get("nbformat").and_then(Value::as_u64) != Some(4) {
        return Err(invalid("notebook.edit_cell supports nbformat 4 only"));
    }
    let minor = notebook
        .get("nbformat_minor")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid("notebook is missing nbformat_minor"))?;
    if !notebook.get("metadata").is_some_and(Value::is_object) {
        return Err(invalid("notebook metadata must be an object"));
    }
    let cells = notebook
        .get_mut("cells")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| invalid("notebook has no cells array"))?;
    let mut ids = HashSet::new();
    for (index, cell) in cells.iter().enumerate() {
        let cell = cell
            .as_object()
            .ok_or_else(|| invalid(format!("cell {index} must be an object")))?;
        if let Some(id) = cell.get("id") {
            let id = id
                .as_str()
                .filter(|id| {
                    !id.is_empty()
                        && id.len() <= 64
                        && id
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
                })
                .ok_or_else(|| invalid(format!("cell {index} has an invalid id")))?;
            if !ids.insert(id.to_owned()) {
                return Err(invalid(format!("duplicate notebook cell id: {id}")));
            }
        }
        if !cell.get("metadata").is_some_and(Value::is_object)
            || !cell.get("source").is_some_and(multiline)
        {
            return Err(invalid(format!(
                "cell {index} requires object metadata and text source"
            )));
        }
        match cell.get("cell_type").and_then(Value::as_str) {
            Some("code") => {
                if cell.contains_key("attachments") {
                    return Err(invalid(format!(
                        "code cell {index} cannot contain attachments"
                    )));
                }
                if !cell
                    .get("execution_count")
                    .is_some_and(|v| v.is_null() || v.as_u64().is_some())
                {
                    return Err(invalid(format!(
                        "code cell {index} has invalid execution_count"
                    )));
                }
                let outputs = cell
                    .get("outputs")
                    .and_then(Value::as_array)
                    .ok_or_else(|| invalid(format!("code cell {index} requires outputs")))?;
                for output in outputs {
                    let valid = match output.get("output_type").and_then(Value::as_str) {
                        Some("stream") => {
                            matches!(
                                output.get("name").and_then(Value::as_str),
                                Some("stdout" | "stderr")
                            ) && output.get("text").is_some_and(multiline)
                        }
                        Some("error") => {
                            output.get("ename").is_some_and(Value::is_string)
                                && output.get("evalue").is_some_and(Value::is_string)
                                && output
                                    .get("traceback")
                                    .and_then(Value::as_array)
                                    .is_some_and(|items| items.iter().all(Value::is_string))
                        }
                        Some("display_data" | "execute_result") => {
                            output.get("data").is_some_and(Value::is_object)
                                && output.get("metadata").is_some_and(Value::is_object)
                                && (output["output_type"] != "execute_result"
                                    || output
                                        .get("execution_count")
                                        .is_some_and(|v| v.is_null() || v.as_u64().is_some()))
                        }
                        _ => false,
                    };
                    if !valid {
                        return Err(invalid(format!(
                            "code cell {index} contains an invalid output"
                        )));
                    }
                }
            }
            Some("markdown" | "raw") => {
                if cell.contains_key("outputs") || cell.contains_key("execution_count") {
                    return Err(invalid(format!(
                        "non-code cell {index} cannot contain execution fields"
                    )));
                }
                if let Some(attachments) = cell.get("attachments")
                    && (cell["cell_type"] != "markdown"
                        || !attachments
                            .as_object()
                            .is_some_and(|items| items.values().all(Value::is_object)))
                {
                    return Err(invalid(format!(
                        "cell {index} contains invalid attachments"
                    )));
                }
            }
            _ => return Err(invalid(format!("cell {index} has an unsupported type"))),
        }
    }
    if minor >= 5 {
        for cell in cells {
            if cell.get("id").is_none() {
                let id = loop {
                    let value = uuid::Uuid::new_v4().simple().to_string();
                    if ids.insert(value.clone()) {
                        break value;
                    }
                };
                cell["id"] = Value::String(id);
            }
        }
    }
    Ok(())
}

struct BoundedJson {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}
impl Write for BoundedJson {
    fn write(&mut self, input: &[u8]) -> std::io::Result<usize> {
        if input.len() > self.limit.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "notebook result exceeds its byte budget",
            ));
        }
        self.bytes.extend_from_slice(input);
        Ok(input.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub(super) fn serialize(notebook: &Value) -> SdkResult<Vec<u8>> {
    serialize_bounded(notebook, MAX_NOTEBOOK_BYTES as usize)
}
fn serialize_bounded(notebook: &Value, limit: usize) -> SdkResult<Vec<u8>> {
    let mut writer = BoundedJson {
        bytes: Vec::new(),
        limit,
        exceeded: false,
    };
    if let Err(error) = serde_json::to_writer_pretty(&mut writer, notebook) {
        return Err(if writer.exceeded {
            invalid("notebook result exceeds the 32 MiB limit; no changes were written")
        } else {
            PluginError::internal_error(&error)
        });
    }
    Ok(writer.bytes)
}

#[cfg(test)]
mod tests {
    use super::super::NotebookCellType;
    use super::*;
    #[test]
    fn byte_budget_covers_escaped_json_and_multibyte_text() {
        for value in [
            serde_json::json!({"text":"中😀\u{0001}"}),
            serde_json::json!(vec!["\n"; 32]),
        ] {
            let size = serde_json::to_vec_pretty(&value).unwrap().len();
            assert!(serialize_bounded(&value, size - 1).is_err());
            assert_eq!(serialize_bounded(&value, size).unwrap().len(), size);
        }
    }
    #[test]
    fn new_cells_have_supported_types() {
        for kind in [
            NotebookCellType::Code,
            NotebookCellType::Markdown,
            NotebookCellType::Raw,
        ] {
            let mut book = serde_json::json!({"nbformat":4,"nbformat_minor":5,"metadata":{},"cells":[super::super::new_cell(kind,"text")]});
            validate(&mut book).unwrap();
            assert!(book["cells"][0]["id"].is_string());
        }
    }
}
