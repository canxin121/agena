use std::path::Path;

use agena_domain::AttachmentItem;
use agena_macros::ToolInput;
use agena_plugin_host::sdk::ToolInvokeOutput;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{PluginError, SdkResult, parse_notebook, read_notebook_bytes, sha256};

const MAX_PAGE_BYTES: usize = 256 * 1024;
const MAX_CELL_TEXT_BYTES: usize = 24 * 1024;
const MAX_IMAGE_DATA_BYTES: usize = 12 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(trim("notebook_path"), non_empty("notebook_path"))]
#[serde(deny_unknown_fields)]
pub(super) struct NotebookReadInput {
    pub(super) notebook_path: String,
    /// Absolute base folder for a relative path in a global session.
    pub(super) folder: Option<String>,
    /// Zero-based index of the first cell. Defaults to 0.
    #[serde(default)]
    offset: usize,
    /// Maximum cells to return. A large result can require additional pages.
    limit: Option<usize>,
}

pub(super) fn read_file(path: &Path, input: &NotebookReadInput) -> SdkResult<ToolInvokeOutput> {
    if path.extension().and_then(|value| value.to_str()) != Some("ipynb") {
        return Err(PluginError::invalid_params(
            "notebook.read requires an .ipynb file",
        ));
    }
    if input.limit == Some(0) {
        return Err(PluginError::invalid_params("limit must be positive"));
    }
    let bytes = read_notebook_bytes(path)?;
    let revision = sha256(&bytes);
    let notebook = parse_notebook(&bytes)?;
    drop(bytes);
    let cells = notebook["cells"]
        .as_array()
        .ok_or_else(|| PluginError::invalid_params("notebook has no cells array"))?;
    let language = notebook
        .pointer("/metadata/language_info/name")
        .and_then(Value::as_str)
        .unwrap_or("python");
    let language = &language[..language.floor_char_boundary(language.len().min(128))];
    let mut page = Vec::new();
    let mut images = Images::default();
    let mut page_bytes = 0usize;
    for (index, cell) in cells
        .iter()
        .enumerate()
        .skip(input.offset)
        .take(input.limit.unwrap_or(usize::MAX))
    {
        let mut budget = CellText::default();
        let source = budget.read([&cell["source"]]);
        let mut outputs = Vec::new();
        let previous_images = images.items.len();
        let previous_image_bytes = images.bytes;
        if let Some(saved) = cell["outputs"].as_array() {
            for (output_index, output) in saved.iter().enumerate() {
                if output_index >= 128 {
                    budget.truncated = true;
                    break;
                }
                let kind = output["output_type"].as_str().unwrap_or_default();
                match kind {
                    "stream" => {
                        outputs.push(
                            json!({"output_type": kind, "text":budget.read([&output["text"]])}),
                        );
                    }
                    "execute_result" | "display_data" | "update_display_data" => {
                        let data = &output["data"];
                        let preview = data
                            .get("text/plain")
                            .or_else(|| data.get("text/markdown"))
                            .unwrap_or(&Value::Null);
                        let text = budget.read([preview]);
                        let image = images.add(data, index, output_index);
                        outputs.push(json!({"output_type": kind, "text": text, "image": image}));
                    }
                    "error" => {
                        let name = budget.read([&output["ename"]]);
                        let message = budget.read([&output["evalue"]]);
                        let traceback = budget.read([&output["traceback"]]);
                        let text = format!("{name}: {message}\n{traceback}");
                        outputs.push(json!({"output_type": kind, "text": text}));
                    }
                    _ => {}
                }
            }
        }
        let cell_type = cell["cell_type"].as_str().unwrap_or("unknown");
        let cell_type = &cell_type[..cell_type.floor_char_boundary(cell_type.len().min(32))];
        let cell_id = cell["id"].as_str().filter(|id| id.len() <= 64);
        let row = json!({
            "cell": index, "cell_id": cell_id, "cellType": cell_type,
            "source": source, "language": language, "execution_count": cell["execution_count"].as_u64(),
            "outputs": outputs, "truncated": budget.truncated,
        });
        let row_size = serde_json::to_vec(&row)
            .map_err(|error| PluginError::internal_error(&error))?
            .len()
            + 1;
        if page_bytes.saturating_add(row_size) > MAX_PAGE_BYTES && !page.is_empty() {
            images.items.truncate(previous_images);
            images.bytes = previous_image_bytes;
            break;
        }
        page_bytes += row_size;
        page.push(row);
    }
    let next = input.offset.saturating_add(page.len());
    let next_offset = (next < cells.len()).then_some(next);
    let summary = format!("Read {} of {} notebook cells", page.len(), cells.len());
    let payload = json!({
        "notebook_path": input.notebook_path,
        "sha256":revision,
        "cell_count":cells.len(),
        "offset": input.offset,
        "next_offset": next_offset,
        "cells": page,
    });
    Ok(ToolInvokeOutput::from_parts(
        format!("Read notebook {}", input.notebook_path),
        summary,
        payload.to_string(),
        Some(payload),
        Default::default(),
        images.items,
    ))
}

struct CellText {
    remaining: usize,
    truncated: bool,
}

impl Default for CellText {
    fn default() -> Self {
        Self {
            remaining: MAX_CELL_TEXT_BYTES,
            truncated: false,
        }
    }
}

impl CellText {
    fn read<'a>(&mut self, parts: impl IntoIterator<Item = &'a Value>) -> String {
        let mut text = String::new();
        for part in parts {
            let strings = part
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_else(|| std::slice::from_ref(part));
            for string in strings.iter().filter_map(Value::as_str) {
                let length = string.floor_char_boundary(string.len().min(self.remaining));
                text.push_str(&string[..length]);
                self.remaining -= length;
                if length < string.len() {
                    self.truncated = true;
                    return text;
                }
            }
        }
        text
    }
}

#[derive(Default)]
struct Images {
    items: Vec<AttachmentItem>,
    bytes: usize,
}

impl Images {
    fn add(&mut self, data: &Value, cell: usize, output: usize) -> Option<Value> {
        let (mime, extension, encoded) = [("image/png", "png"), ("image/jpeg", "jpg")]
            .into_iter()
            .find_map(|(mime, extension)| {
                let value = data.get(mime)?;
                let parts = value
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or_else(|| std::slice::from_ref(value));
                let mut encoded = String::new();
                for part in parts {
                    let part = part.as_str()?;
                    if encoded.len().saturating_add(part.len()) > MAX_IMAGE_DATA_BYTES {
                        return Some((mime, extension, None));
                    }
                    encoded.push_str(part);
                }
                Some((mime, extension, Some(encoded)))
            })?;
        let attachment = encoded.and_then(|encoded| {
            if self.items.len() >= agena_runtime_tools::media_input::MAX_MEDIA_INPUTS
                || self.bytes.saturating_add(encoded.len()) > MAX_IMAGE_DATA_BYTES
            {
                return None;
            }
            let bytes = STANDARD
                .decode(
                    encoded
                        .bytes()
                        .filter(|byte| !byte.is_ascii_whitespace())
                        .collect::<Vec<_>>(),
                )
                .ok()?;
            let filename = format!("cell-{cell}-output-{output}.{extension}");
            let image =
                agena_runtime_tools::media_input::from_bytes(&filename, bytes, Some(mime)).ok()?;
            if image.mime != mime {
                return None;
            }
            self.bytes += encoded.len();
            let index = self.items.len();
            self.items.push(image.attachment(None));
            Some(json!({"mime": mime, "attachment_index": index, "filename":filename}))
        });
        Some(attachment.unwrap_or_else(|| json!({"mime": mime, "omitted": true})))
    }
}
