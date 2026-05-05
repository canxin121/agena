//! `skill_run` builtin tool.
//!
//! Looks up the named skill via [`SkillsManager`], renders its body as
//! the tool output (the model is then expected to follow the inlined
//! instructions on the next turn).  The tool surface itself is a thin
//! router; full skill execution (allowed_tools constraint, model
//! switch) is opportunistic — we expose the metadata so callers can act
//! on it but do not currently rebuild the catalog mid-turn.

use std::sync::Arc;

use agena_skills::SkillsManager;

use crate::message::{BuiltinToolOutput, SkillRunToolInput};

use super::{BuiltinExecution, ToolError, ToolExecutionView, ToolExecutor};

pub(super) fn execute(
    executor: &ToolExecutor,
    input: &SkillRunToolInput,
) -> Result<BuiltinExecution, ToolError> {
    let manager = executor
        .skills_manager()
        .ok_or_else(|| ToolError::Plugin("skill_run: skill manager not configured".to_string()))?;
    let skill = manager
        .get(input.name.trim())
        .map_err(|e| ToolError::Plugin(format!("skill_run: {e}")))?;

    let mut body = skill.body.clone();
    if let Some(args) = input.args.as_deref() {
        let trimmed = args.trim();
        if !trimmed.is_empty() {
            body.push_str("\n\n# User-supplied arguments\n\n");
            body.push_str(trimmed);
            body.push('\n');
        }
    }

    let view = ToolExecutionView::simple(
        format!("skill_run: {}", skill.frontmatter.name),
        body.clone(),
    );
    let output = BuiltinToolOutput::SkillRun {
        name: skill.frontmatter.name.clone(),
        body_chars: body.chars().count(),
        allowed_tools: skill.frontmatter.allowed_tools.clone(),
        model: skill.frontmatter.model.clone(),
    };
    Ok(BuiltinExecution::new(output, view))
}

#[allow(dead_code)]
pub type SkillsManagerHandle = Arc<SkillsManager>;
