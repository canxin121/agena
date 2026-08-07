//! Built-in human renderer for activity v2 (design 07 \u00a74.3 / 08 \u00a72).
//!
//! Every built-in tool renders its human-facing `ViewBlock` stream through
//! this one renderer: it maps the structured [`ToolPayloadOutput`] variant to
//! first-class blocks (command card, file changes, diff, search results,
//! path lists) and falls back to the raw output (`Json` payload + `Log` text)
//! for tools without a structured presentation. This is the v2 counterpart of
//! the legacy `operation_blocks_from_tool_output` projection, owned by the
//! tools crate so renderers live next to the tools that produce the output.

use agena_domain::{CommandOutputStream, RawOutput, ToolOutput, ViewBlock};
use agena_tool::{RenderContext, RenderError, ToolHumanRenderer};

use crate::tool::payload::ToolPayloadOutput;
use agena_domain::WebSearchResult;

/// A renderer for one built-in tool result. `tool_name` follows the same
/// resolution rules as [`ToolPayloadOutput::from_tool_output`]; `command` and
/// `cwd` let shell/process executions render a `$ command` card instead of a
/// bare output card.
#[derive(Debug, Clone)]
pub struct BuiltinHumanRenderer {
    pub tool_name: String,
    /// The shell command line from the invocation input, when known.
    pub command: Option<String>,
    /// The working directory from the invocation input, when known.
    pub cwd: Option<String>,
}

impl BuiltinHumanRenderer {
    pub fn new(tool_name: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            command: None,
            cwd: None,
        }
    }

    pub fn with_command(mut self, command: impl Into<String>) -> Self {
        self.command = Some(command.into());
        self
    }

    pub fn with_cwd(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    /// Render the raw output directly (same floor as the runtime fallback).
    fn fallback(raw: &RawOutput) -> Vec<ViewBlock> {
        let mut blocks = Vec::new();
        if let Some(payload) = raw.payload.as_ref() {
            blocks.push(ViewBlock::Json {
                id: Some("payload".into()),
                value: payload.clone(),
            });
        }
        if !raw.text.is_empty() {
            blocks.push(ViewBlock::Log {
                id: Some("text".into()),
                stream: CommandOutputStream::Stdout,
                text: raw.text.clone(),
            });
        }
        blocks
    }

    fn markdown_block(text: &str) -> ViewBlock {
        ViewBlock::Markdown {
            id: Some("detail".into()),
            text: text.to_owned(),
        }
    }

    fn structured_blocks(
        tool_name: &str,
        raw: &RawOutput,
        output: &ToolOutput,
        command: Option<&str>,
        cwd: Option<&str>,
    ) -> Vec<ViewBlock> {
        let mut blocks = Vec::new();
        // Shell/process executions render as a command card: the command line
        // and its output, exit code, and stderr as a distinct human block.
        if let Some(parsed) = ToolPayloadOutput::from_tool_output(tool_name, output) {
            if let ToolPayloadOutput::Shell {
                exit_code,
                output: shell_output,
                ..
            } = &parsed
                && let Some(command) = command.filter(|c| !c.trim().is_empty())
            {
                blocks.push(ViewBlock::Command {
                    id: Some("command".into()),
                    command: command.to_owned(),
                    cwd: cwd.map(ToOwned::to_owned),
                    exit_code: *exit_code,
                    stdout: shell_output
                        .as_deref()
                        .unwrap_or(raw.text.as_str())
                        .to_owned(),
                    stderr: String::new(),
                });
                return blocks;
            }
        }

        match ToolPayloadOutput::from_tool_output(tool_name, output) {
            Some(ToolPayloadOutput::ApplyPatch { changes, diff, .. }) => {
                if !changes.is_empty() {
                    blocks.push(ViewBlock::FileChanges {
                        id: Some("changes".into()),
                        changes,
                    });
                }
                if !diff.trim().is_empty() {
                    blocks.push(ViewBlock::Diff {
                        id: Some("diff".into()),
                        diff,
                        language: Some("diff".into()),
                    });
                }
            }
            Some(ToolPayloadOutput::Read {
                preview,
                loaded_paths,
                truncated,
                ..
            }) => {
                let mut lines = Vec::new();
                if !loaded_paths.is_empty() {
                    lines.push("### loaded paths".to_owned());
                    lines.extend(loaded_paths.iter().map(|p| format!("- {p}")));
                }
                if let Some(preview) = preview {
                    lines.push("### preview".to_owned());
                    lines.push(format!("```text\n{preview}\n```"));
                }
                if truncated {
                    lines.push("_truncated_".to_owned());
                }
                if !lines.is_empty() {
                    blocks.push(Self::markdown_block(&lines.join("\n")));
                }
            }
            Some(ToolPayloadOutput::Glob {
                paths,
                count,
                truncated,
                ..
            }) => {
                let mut lines = Vec::new();
                if let Some(count) = count {
                    lines.push(format!("### {count} matches"));
                }
                lines.extend(paths.iter().map(|p| format!("- {p}")));
                if truncated {
                    lines.push("_truncated_".to_owned());
                }
                if !lines.is_empty() {
                    blocks.push(Self::markdown_block(&lines.join("\n")));
                }
            }
            Some(ToolPayloadOutput::Grep {
                results, matches, ..
            }) => {
                let mut lines = Vec::new();
                if let Some(matches) = matches {
                    lines.push(format!("### {matches} matches"));
                }
                lines.extend(results.iter().map(|r| format!("- {r}")));
                if !lines.is_empty() {
                    blocks.push(Self::markdown_block(&lines.join("\n")));
                }
            }
            Some(ToolPayloadOutput::ToolSearch { results }) => {
                let mut lines = vec!["### tool results".to_owned()];
                lines.extend(results.iter().map(|r| format!("- {r}")));
                blocks.push(Self::markdown_block(&lines.join("\n")));
            }
            Some(ToolPayloadOutput::WebSearch { query, results, .. }) => {
                let items: Vec<WebSearchResult> = results;
                blocks.push(ViewBlock::SearchResults {
                    id: Some("search".into()),
                    total: Some(items.len() as u64),
                    items,
                });
                let _ = query;
            }
            Some(ToolPayloadOutput::WebFetch {
                url,
                markdown,
                summary,
                truncated,
                ..
            }) => {
                let mut lines = vec![format!("### {url}")];
                if let Some(summary) = summary {
                    lines.push(summary.clone());
                }
                if let Some(markdown) = markdown {
                    lines.push(format!("```markdown\n{markdown}\n```"));
                }
                if truncated {
                    lines.push("_truncated_".to_owned());
                }
                blocks.push(Self::markdown_block(&lines.join("\n")));
            }
            Some(ToolPayloadOutput::LspDiagnostics { entries }) => {
                let mut lines = vec!["### diagnostics".to_owned()];
                lines.extend(entries.iter().map(|e| format!("- {e}")));
                blocks.push(Self::markdown_block(&lines.join("\n")));
            }
            Some(ToolPayloadOutput::LspDefinition { locations })
            | Some(ToolPayloadOutput::LspReferences { locations }) => {
                let mut lines = vec!["### locations".to_owned()];
                lines.extend(locations.iter().map(|l| format!("- {l}")));
                blocks.push(Self::markdown_block(&lines.join("\n")));
            }
            Some(ToolPayloadOutput::CronList { jobs }) => {
                let mut lines = vec!["### cron jobs".to_owned()];
                lines.extend(jobs.iter().map(|j| format!("- {j:?}")));
                blocks.push(Self::markdown_block(&lines.join("\n")));
            }
            Some(ToolPayloadOutput::Task {
                status, final_text, ..
            }) => {
                let mut lines = vec![format!("### task {status}")];
                if let Some(final_text) = final_text {
                    lines.push(final_text.clone());
                }
                blocks.push(Self::markdown_block(&lines.join("\n")));
            }
            _ => {}
        }
        blocks
    }
}

impl ToolHumanRenderer for BuiltinHumanRenderer {
    fn render_human(
        &self,
        _ctx: &RenderContext,
        raw: &RawOutput,
    ) -> Result<Vec<ViewBlock>, RenderError> {
        let output = ToolOutput::from_json_payload(raw.payload.as_ref())
            .map_err(|error| RenderError::Failed(error))?;
        let blocks = Self::structured_blocks(
            &self.tool_name,
            raw,
            &output,
            self.command.as_deref(),
            self.cwd.as_deref(),
        );
        if blocks.is_empty() {
            return Ok(Self::fallback(raw));
        }
        Ok(blocks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_tool::RenderContext as ToolRenderContext;
    use serde_json::json;
    use std::path::PathBuf;

    fn ctx() -> ToolRenderContext {
        ToolRenderContext {
            workspace_root: PathBuf::from("/tmp"),
            command: None,
        }
    }

    #[test]
    fn apply_patch_renders_file_changes_and_diff() {
        let renderer = BuiltinHumanRenderer::new("apply_patch");
        let raw = RawOutput {
            payload: Some(json!({
                "operation_id": "op-1",
                "inverse_patch": "",
                "changes": [{"path": "a.txt", "kind": "updated"}],
                "diff": "--- a\n+++ b\n"
            })),
            text: String::new(),
            ..RawOutput::default()
        };
        let blocks = renderer.render_human(&ctx(), &raw).expect("render");
        assert!(
            blocks
                .iter()
                .any(|b| matches!(b, ViewBlock::FileChanges { .. }))
        );
        assert!(blocks.iter().any(|b| matches!(b, ViewBlock::Diff { .. })));
    }

    #[test]
    fn shell_renders_command_card_with_output() {
        let renderer = BuiltinHumanRenderer::new("shell")
            .with_command("cargo test")
            .with_cwd("/tmp");
        let raw = RawOutput {
            payload: Some(json!({
                "action": "run",
                "exit_code": 0,
                "output": "ok\n"
            })),
            text: "ok\n".into(),
            ..RawOutput::default()
        };
        let blocks = renderer.render_human(&ctx(), &raw).expect("render");
        match blocks.as_slice() {
            [
                ViewBlock::Command {
                    command,
                    cwd,
                    exit_code,
                    stdout,
                    ..
                },
            ] => {
                assert_eq!(command, "cargo test");
                assert_eq!(cwd.as_deref(), Some("/tmp"));
                assert_eq!(*exit_code, Some(0));
                assert_eq!(stdout, "ok\n");
            }
            other => panic!("expected command card, got {other:?}"),
        }
    }

    #[test]
    fn glob_renders_path_list_and_fallback_is_used_for_opaque_payloads() {
        let renderer = BuiltinHumanRenderer::new("glob");
        let raw = RawOutput {
            payload: Some(json!({ "paths": ["a.rs", "b.rs"], "count": 2 })),
            text: String::new(),
            ..RawOutput::default()
        };
        let blocks = renderer.render_human(&ctx(), &raw).expect("render");
        assert!(
            blocks
                .iter()
                .any(|b| matches!(b, ViewBlock::Markdown { .. }))
        );

        let opaque = BuiltinHumanRenderer::new("unknown_tool");
        let raw = RawOutput {
            payload: Some(json!({ "x": 1 })),
            text: "line\n".into(),
            ..RawOutput::default()
        };
        let blocks = opaque.render_human(&ctx(), &raw).expect("render");
        assert!(blocks.iter().any(|b| matches!(b, ViewBlock::Json { .. })));
        assert!(blocks.iter().any(|b| matches!(b, ViewBlock::Log { .. })));
    }

    #[test]
    fn web_search_renders_search_results_block() {
        let renderer = BuiltinHumanRenderer::new("web.search");
        let raw = RawOutput {
            payload: Some(json!({
                "query": "rust",
                "backend": "test",
                "results": [{"title": "Rust", "url": "https://rust-lang.org", "snippet": "lang"}]
            })),
            text: String::new(),
            ..RawOutput::default()
        };
        let blocks = renderer.render_human(&ctx(), &raw).expect("render");
        match blocks.as_slice() {
            [ViewBlock::SearchResults { items, .. }] => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].title, "Rust");
            }
            other => panic!("expected search results, got {other:?}"),
        }
    }
}
