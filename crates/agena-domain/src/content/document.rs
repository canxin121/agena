//! Bounded semantic documents and exact, typed mutations. The source owns
//! checkpoint creation; consumers own layout and all observation preferences.

use serde::{Deserialize, Serialize};

use crate::{ViewBlock, WebSearchResult};

pub const MAX_DOCUMENT_BYTES: usize = 64 * 1024;
pub const MAX_DOCUMENT_BLOCKS: usize = 128;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentDocument {
    pub blocks: Vec<ViewBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DocumentMutation {
    Insert {
        after: Option<String>,
        block: ViewBlock,
    },
    Replace {
        block: ViewBlock,
    },
    Remove {
        block_id: String,
    },
    AppendText {
        block_id: String,
        text: String,
    },
    AppendRows {
        block_id: String,
        rows: Vec<Vec<serde_json::Value>>,
    },
    AppendSearchResults {
        block_id: String,
        items: Vec<WebSearchResult>,
        total: Option<u64>,
    },
    Progress {
        block_id: String,
        phase: String,
        completed: u64,
        total: Option<u64>,
        unit: Option<String>,
    },
}

fn validate_id(id: &str) -> Result<(), &'static str> {
    if id.is_empty() || id.len() > 128 || id.chars().any(char::is_control) {
        return Err("document blocks require a stable nonempty id of at most 128 bytes");
    }
    Ok(())
}

fn validate_block(block: &ViewBlock) -> Result<(), &'static str> {
    validate_id(block.block_id().ok_or("document block has no id")?)?;
    match block {
        // Resources are attached to their owning Part. A nested content
        // reference would create a second authorization/GC ownership graph.
        ViewBlock::Content { .. } => Err("nested content references are not document blocks"),
        ViewBlock::Table { columns, rows, .. } => {
            if columns.is_empty()
                || columns.len() > 64
                || rows.iter().any(|row| row.len() != columns.len())
            {
                return Err("document table rows must match its nonempty columns");
            }
            Ok(())
        }
        ViewBlock::Progress {
            completed, total, ..
        } if total.is_some_and(|total| *completed > total) => {
            Err("progress completed count exceeds its total")
        }
        _ => Ok(()),
    }
}

impl DocumentMutation {
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Insert { after, block } => {
                if let Some(after) = after {
                    validate_id(after)?;
                }
                validate_block(block)
            }
            Self::Replace { block } => validate_block(block),
            Self::Remove { block_id }
            | Self::AppendText { block_id, .. }
            | Self::AppendRows { block_id, .. }
            | Self::AppendSearchResults { block_id, .. } => validate_id(block_id),
            Self::Progress {
                block_id,
                completed,
                total,
                ..
            } => {
                validate_id(block_id)?;
                if total.is_some_and(|total| *completed > total) {
                    return Err("progress completed count exceeds its total");
                }
                Ok(())
            }
        }
    }
}

impl ContentDocument {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.blocks.len() > MAX_DOCUMENT_BLOCKS {
            return Err("document block budget exhausted");
        }
        let mut ids = std::collections::HashSet::new();
        for block in &self.blocks {
            validate_block(block)?;
            if !ids.insert(block.block_id().expect("validated block id")) {
                return Err("duplicate document block id");
            }
        }
        if self.byte_len() > MAX_DOCUMENT_BYTES {
            return Err(
                "document byte budget exhausted; use a separate content resource for large bodies",
            );
        }
        Ok(())
    }

    pub fn byte_len(&self) -> usize {
        serde_json::to_vec(self)
            .expect("semantic document serializes")
            .len()
    }

    fn index(&self, id: &str) -> Result<usize, &'static str> {
        self.blocks
            .iter()
            .position(|block| block.block_id() == Some(id))
            .ok_or("document mutation refers to a missing block")
    }

    /// A rejected mutation never alters the last accepted checkpoint. The
    /// clone is bounded by the semantic document budget, independent of log
    /// volume, Part history, database size, and observer count.
    pub fn updated(&self, event: &DocumentMutation) -> Result<Self, &'static str> {
        event.validate()?;
        let mut next = self.clone();
        match event {
            DocumentMutation::Insert { after, block } => {
                let id = block.block_id().expect("validated id");
                if next.blocks.iter().any(|block| block.block_id() == Some(id)) {
                    return Err("document insertion reuses an existing block id");
                }
                let index = after
                    .as_deref()
                    .map(|id| next.index(id).map(|index| index + 1))
                    .transpose()?
                    .unwrap_or(0);
                next.blocks.insert(index, block.clone());
            }
            DocumentMutation::Replace { block } => {
                let index = next.index(block.block_id().expect("validated id"))?;
                next.blocks[index] = block.clone();
            }
            DocumentMutation::Remove { block_id } => {
                let index = next.index(block_id)?;
                next.blocks.remove(index);
            }
            DocumentMutation::AppendText { block_id, text } => {
                let index = next.index(block_id)?;
                match &mut next.blocks[index] {
                    ViewBlock::Text { text: target, .. }
                    | ViewBlock::Markdown { text: target, .. }
                    | ViewBlock::Log { text: target, .. }
                    | ViewBlock::Diff { diff: target, .. } => target.push_str(text),
                    _ => {
                        return Err(
                            "text may be appended only to text, markdown, log or diff blocks",
                        );
                    }
                }
            }
            DocumentMutation::AppendRows { block_id, rows } => {
                let index = next.index(block_id)?;
                let ViewBlock::Table {
                    columns,
                    rows: target,
                    ..
                } = &mut next.blocks[index]
                else {
                    return Err("rows require a table block");
                };
                if rows.iter().any(|row| row.len() != columns.len()) {
                    return Err("appended rows differ from table columns");
                }
                target.extend(rows.iter().cloned());
            }
            DocumentMutation::AppendSearchResults {
                block_id,
                items,
                total,
            } => {
                let index = next.index(block_id)?;
                let ViewBlock::SearchResults {
                    items: target,
                    total: count,
                    ..
                } = &mut next.blocks[index]
                else {
                    return Err("search items require a search-results block");
                };
                target.extend(items.iter().cloned());
                if total.is_some() {
                    *count = *total;
                }
            }
            DocumentMutation::Progress {
                block_id,
                phase,
                completed,
                total,
                unit,
            } => {
                let block = ViewBlock::Progress {
                    id: block_id.clone(),
                    phase: phase.clone(),
                    completed: *completed,
                    total: *total,
                    unit: unit.clone(),
                };
                match next.index(block_id) {
                    Ok(index) if matches!(next.blocks[index], ViewBlock::Progress { .. }) => {
                        next.blocks[index] = block
                    }
                    Ok(_) => return Err("progress cannot replace a different block kind"),
                    Err(_) => next.blocks.push(block),
                }
            }
        }
        next.validate()?;
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn typed_mutations_preserve_order_and_reject_invalid_updates_atomically() {
        let start = ContentDocument::default();
        let table = start
            .updated(&DocumentMutation::Insert {
                after: None,
                block: ViewBlock::Table {
                    id: Some("rows".into()),
                    columns: vec!["name".into()],
                    rows: vec![],
                },
            })
            .unwrap();
        let populated = table
            .updated(&DocumentMutation::AppendRows {
                block_id: "rows".into(),
                rows: vec![vec![json!("中")]],
            })
            .unwrap();
        assert!(
            populated
                .updated(&DocumentMutation::AppendRows {
                    block_id: "rows".into(),
                    rows: vec![vec![]]
                })
                .is_err()
        );
        assert!(
            populated
                .updated(&DocumentMutation::AppendText {
                    block_id: "rows".into(),
                    text: "wrong type".into()
                })
                .is_err()
        );
        assert_eq!(
            table.blocks[0],
            ViewBlock::Table {
                id: Some("rows".into()),
                columns: vec!["name".into()],
                rows: vec![]
            }
        );
        let progress = populated
            .updated(&DocumentMutation::Progress {
                block_id: "work".into(),
                phase: "scan".into(),
                completed: 2,
                total: Some(3),
                unit: Some("files".into()),
            })
            .unwrap();
        assert_eq!(progress.blocks[1].block_id(), Some("work"));
        assert!(
            progress
                .updated(&DocumentMutation::Progress {
                    block_id: "work".into(),
                    phase: "scan".into(),
                    completed: 4,
                    total: Some(3),
                    unit: None
                })
                .is_err()
        );
        assert_eq!(
            progress
                .updated(&DocumentMutation::Remove {
                    block_id: "rows".into()
                })
                .unwrap()
                .blocks
                .len(),
            1
        );
    }
}
