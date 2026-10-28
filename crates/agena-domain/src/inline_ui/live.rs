//! Revisioned agent-authored content. User input lives separately in UiState;
//! updates never smuggle commands, clear input drafts, or mutate old tool results.
use super::{InlineBlock, InlineDocument, UiNode, UiState, inline_reference_tool_id};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InlinePhase {
    #[default]
    Working,
    Complete,
    Error,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InlineSnapshot {
    pub reference: String,
    pub revision: u64,
    pub phase: InlinePhase,
    pub updated_at_ms: i64,
    pub last_call_id: i64,
    pub document: InlineDocument,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InlinePanelPatch {
    pub block_id: String,
    #[serde(default)]
    pub data: UiState,
    /// Whole-node upserts preserve identity and allow incremental layout growth.
    #[serde(default)]
    pub nodes: Vec<UiNode>,
    /// Explicitly remove a subtree. Dangling/duplicate/cyclic references reject
    /// the entire transaction, leaving the previous valid revision visible.
    #[serde(default)]
    pub remove: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InlineUpdate {
    pub reference: String,
    pub expected_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<InlineDocument>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<InlinePanelPatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<InlinePhase>,
}
impl InlineSnapshot {
    pub fn validate(&self) -> Result<(), String> {
        if inline_reference_tool_id(&self.reference).is_none() || self.revision == 0 {
            return Err("invalid live reference/revision".into());
        }
        self.document.validate()
    }
    pub fn apply(&self, update: &InlineUpdate, call_id: i64, now: i64) -> Result<Self, String> {
        if update.reference != self.reference {
            return Err("reference mismatch".into());
        }
        if self.last_call_id == call_id && call_id > 0 {
            return Ok(self.clone());
        }
        if update.expected_revision != self.revision {
            return Err(format!(
                "revision conflict: expected {}, current {}; read the current UI and retry",
                update.expected_revision, self.revision
            ));
        }
        if update.document.is_some() && update.patch.is_some() {
            return Err("provide document OR patch, not both".into());
        }
        if update.document.is_none() && update.patch.is_none() && update.phase.is_none() {
            return Err("update has no changes".into());
        }
        let mut next = self.clone();
        if let Some(document) = &update.document {
            next.document = document.clone();
        }
        if let Some(patch) = &update.patch {
            if patch.nodes.len() > 128 || patch.remove.len() > 128 || patch.data.len() > 64 {
                return Err("patch complexity exceeds budget".into());
            }
            let block = next
                .document
                .blocks
                .iter_mut()
                .find(|b| b.id() == patch.block_id)
                .ok_or("panel block not found")?;
            let InlineBlock::Panel { program, .. } = block else {
                return Err("patch target must be a panel".into());
            };
            program.data.extend(patch.data.clone());
            let mut removed = patch
                .remove
                .iter()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>();
            loop {
                let before = removed.len();
                for node in &program.nodes {
                    if removed.contains(&node.id) {
                        removed.extend(node.children.iter().cloned());
                    }
                }
                if before == removed.len() {
                    break;
                }
            }
            if removed.contains(&program.root) {
                return Err("cannot remove panel root".into());
            }
            program.nodes.retain(|n| !removed.contains(&n.id));
            for node in &mut program.nodes {
                node.children.retain(|id| !removed.contains(id));
            }
            let mut ids = std::collections::BTreeSet::new();
            for node in &patch.nodes {
                if !ids.insert(&node.id) {
                    return Err("duplicate upsert id".into());
                }
                if let Some(existing) = program.nodes.iter_mut().find(|n| n.id == node.id) {
                    *existing = node.clone();
                } else {
                    program.nodes.push(node.clone());
                }
            }
        }
        next.document.validate()?;
        next.revision = next.revision.checked_add(1).ok_or("revision exhausted")?;
        next.updated_at_ms = now;
        next.last_call_id = call_id;
        if let Some(phase) = update.phase {
            next.phase = phase;
        }
        Ok(next)
    }
}
