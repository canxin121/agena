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
    }
}

fn invalid(message: impl Into<String>) -> PluginError {
    PluginError::invalid_params(message.into())
}
pub(super) fn validate(notebook: &mut Value) -> SdkResult<()> {
    if notebook.get("nbformat").and_then(Value::as_u64) != Some(4) {
        return Err(invalid("notebook.edit_cell supports nbformat 4 only"));
    }
    let minor = notebook
        .get("nbformat_minor")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid("notebook is missing nbformat_minor"))?;
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
    }
    // IDs entered nbformat in 4.5. Do not add unknown fields to older files.
    if minor >= 5 {
        for cell in cells {
            if cell.get("id").is_none() {
                let id = loop {
                    let candidate = uuid::Uuid::new_v4().simple().to_string();
                    if ids.insert(candidate.clone()) {
                        break candidate;
                    }
                };
                cell["id"] = Value::String(id);
            }
        }
    }
    super::schema::validate(notebook, minor)
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
    fn official_reference_corpus_and_additional_id_invariant_agree() {
        let corpus: Value =
            serde_json::from_str(include_str!("schemas/reference-corpus.json")).unwrap();
        for case in corpus["cases"].as_array().unwrap() {
            let mut notebook = case["notebook"].clone();
            let expected = case
                .get("agena_valid")
                .unwrap_or(&case["valid"])
                .as_bool()
                .unwrap();
            let result = validate(&mut notebook);
            assert_eq!(result.is_ok(), expected, "{}: {result:?}", case["name"]);
            if expected {
                assert_eq!(
                    notebook, case["notebook"],
                    "validation must preserve existing notebook data"
                );
            }
        }
    }
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
