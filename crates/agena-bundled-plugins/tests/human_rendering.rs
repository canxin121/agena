use std::path::PathBuf;

use agena_domain::{RawOutput, ViewBlock};
use agena_runtime_tools::tool::human_view::BuiltinHumanRenderer;
use agena_tool::{RenderContext, ToolHumanRenderer};
use serde_json::{Value, json};

fn render_context() -> RenderContext {
    RenderContext {
        workspace_root: PathBuf::from("/tmp/agena-rendering-test"),
        command: None,
    }
}

fn sample_payload(tool: &str) -> Value {
    let mut payload = json!({
        "status": "completed",
        "message": "The operation completed.",
        "count": 1,
    });
    let Some(object) = payload.as_object_mut() else {
        return payload;
    };

    if tool.contains("findings") {
        object.insert(
            "findings".to_owned(),
            json!([{
                "severity": "high",
                "file": "src/lib.rs",
                "line": 7,
                "title": "Example finding",
                "body": "Example finding body",
                "confidence": 0.9,
            }]),
        );
    } else if tool.contains("servers") {
        object.insert(
            "servers".to_owned(),
            json!([{"name": "example", "connected": true, "tool_count": 2}]),
        );
    } else if tool.contains("tasks") || tool.contains("process") || tool.contains("sessions") {
        object.insert(
            "items".to_owned(),
            json!([{"id": "item-1", "status": "completed", "title": "Example item"}]),
        );
    } else if tool.contains("tools") || tool.contains("resources") || tool.contains("prompts") {
        object.insert(
            "results".to_owned(),
            json!([{"name": "example", "description": "Example result", "server": "demo"}]),
        );
    } else {
        object.insert(
            "details".to_owned(),
            json!({"key": "example", "available": true}),
        );
    }
    payload
}

#[test]
fn every_bundled_execution_tool_has_a_non_json_human_fallback() {
    let manifest = agena_bundled_plugins::bundled_capability_manifest();
    let context = render_context();
    let mut checked = 0;

    for plugin in manifest.plugins {
        for tool in plugin.tools {
            if tool.gateway {
                continue;
            }
            checked += 1;
            let compact_name = tool
                .canonical_name
                .strip_prefix("agena.")
                .unwrap_or(tool.canonical_name.as_str());
            let raw = RawOutput {
                payload: Some(sample_payload(compact_name)),
                ..RawOutput::default()
            };
            let blocks = BuiltinHumanRenderer::new(compact_name)
                .render_human(&context, &raw)
                .expect("bundled renderer should not fail");
            assert!(
                blocks
                    .iter()
                    .any(|block| !matches!(block, ViewBlock::Json { .. })),
                "{compact_name} rendered only an opaque JSON block: {blocks:?}"
            );
        }
    }

    assert_eq!(checked, 137);
}

#[test]
fn representative_plugin_payloads_render_complete_readable_facts() {
    let fixtures = vec![
        (
            "mcp.resources.list",
            RawOutput {
                text: "- README (mcp://demo/readme) [text/markdown]: Project documentation".into(),
                payload: Some(json!({
                    "server": "demo",
                    "resources": [{
                        "uri": "mcp://demo/readme",
                        "name": "README",
                        "description": "Project documentation",
                        "mime_type": "text/markdown"
                    }],
                    "next_cursor": "resources-cursor-2"
                })),
                ..RawOutput::default()
            },
            vec!["demo", "README", "resources-cursor-2"],
            true,
        ),
        (
            "mcp.resources.templates.list",
            RawOutput {
                text: "- User profile (users://{id}) [application/json]: A user profile".into(),
                payload: Some(json!({
                    "server": "demo",
                    "resource_templates": [{
                        "uri_template": "users://{id}",
                        "name": "User profile",
                        "description": "A user profile",
                        "mime_type": "application/json"
                    }],
                    "next_cursor": "templates-cursor-2"
                })),
                ..RawOutput::default()
            },
            vec!["demo", "User profile", "templates-cursor-2"],
            true,
        ),
        (
            "mcp.resources.read",
            RawOutput {
                text: "mcp://demo/readme [text/markdown]\n# Hello from MCP".into(),
                payload: Some(json!({
                    "server": "demo",
                    "uri": "mcp://demo/readme",
                    "contents": [{
                        "uri": "mcp://demo/readme",
                        "mime_type": "text/markdown",
                        "text": "# Hello from MCP"
                    }]
                })),
                ..RawOutput::default()
            },
            vec!["demo", "# Hello from MCP"],
            true,
        ),
        (
            "mcp.prompts.list",
            RawOutput {
                text: "- summarize (document*): Summarize a document".into(),
                payload: Some(json!({
                    "server": "demo",
                    "prompts": [{
                        "name": "summarize",
                        "description": "Summarize a document",
                        "arguments": [{"name": "document", "required": true}]
                    }],
                    "next_cursor": null
                })),
                ..RawOutput::default()
            },
            vec!["demo", "summarize"],
            true,
        ),
        (
            "mcp.prompts.get",
            RawOutput {
                text: "user: Summarize this document".into(),
                payload: Some(json!({
                    "server": "demo",
                    "prompt": "summarize",
                    "description": "Summarize a document",
                    "messages": [{
                        "role": "user",
                        "content": {"type": "text", "text": "Summarize this document"}
                    }]
                })),
                ..RawOutput::default()
            },
            vec!["demo", "summarize", "Summarize this document"],
            true,
        ),
        (
            "mcp.tools.call",
            RawOutput {
                text: "Search result: 3 matching documents".into(),
                payload: Some(json!({
                    "server": "demo",
                    "tool": "search",
                    "content": [{"type": "text", "text": "Search result: 3 matching documents"}],
                    "structured_content": {"matches": 3},
                    "mcp_meta": {"request_id": "mcp-17"}
                })),
                ..RawOutput::default()
            },
            vec!["demo", "search", "mcp-17"],
            true,
        ),
        (
            "mcp.tools.search",
            RawOutput {
                text: "Found 1 matching MCP tool.".into(),
                payload: Some(json!({
                    "query": "search",
                    "results": [{
                        "server": "demo",
                        "name": "search",
                        "description": "Search documents",
                        "risk": "low"
                    }],
                    "total": 1,
                    "index_fingerprint": "abc123"
                })),
                ..RawOutput::default()
            },
            vec!["search", "demo", "abc123"],
            true,
        ),
        (
            "mcp.servers.status",
            RawOutput {
                text: "MCP server status refreshed.".into(),
                payload: Some(json!({
                    "servers": [{
                        "name": "demo",
                        "connected": true,
                        "status": "ready",
                        "tool_count": 4,
                        "resource_count": 2,
                        "prompt_count": 1
                    }],
                    "checked_at": "2026-08-22T10:00:00Z"
                })),
                ..RawOutput::default()
            },
            vec!["demo", "ready", "2026-08-22T10:00:00Z"],
            true,
        ),
        (
            "mcp.servers.reconnect",
            RawOutput {
                text: "Reconnected MCP server 'demo'.".into(),
                payload: Some(json!({
                    "server": "demo",
                    "reconnected": true,
                    "status": "connected",
                    "message": "Handshake completed",
                    "attempt": 2
                })),
                ..RawOutput::default()
            },
            vec!["demo", "connected", "Handshake completed"],
            false,
        ),
        (
            "settings.inspect",
            RawOutput {
                text: "Inspected global, workspace, and effective settings values.".into(),
                payload: Some(json!({
                    "path": "providers.openai",
                    "global": {"defined": true, "value": "redacted"},
                    "workspace": {"defined": false},
                    "effective": {"enabled": true},
                    "applied_layers": ["global", "environment"],
                    "layers": [
                        {"name": "global", "active": true},
                        {"name": "environment", "active": false}
                    ]
                })),
                ..RawOutput::default()
            },
            vec!["providers.openai", "global", "environment"],
            true,
        ),
        (
            "settings.list",
            RawOutput {
                text: "- providers.openai.model = gpt-5".into(),
                payload: Some(json!({
                    "source": "effective",
                    "config_found": true,
                    "items": [{
                        "path": "providers.openai.model",
                        "value": "gpt-5",
                        "source": "global"
                    }],
                    "count": 1
                })),
                ..RawOutput::default()
            },
            vec!["effective", "providers.openai.model", "global"],
            true,
        ),
        (
            "settings.get",
            RawOutput {
                text: "providers.openai.model = gpt-5 (source: global)".into(),
                payload: Some(json!({
                    "path": "providers.openai.model",
                    "value": "gpt-5",
                    "source": "global",
                    "layer": "global",
                    "config_path": "/workspace/.agena/agena.json"
                })),
                ..RawOutput::default()
            },
            vec!["providers.openai.model", "gpt-5", "agena.json"],
            false,
        ),
        (
            "settings.set",
            RawOutput {
                text: "Updated providers.openai.model in the workspace settings.".into(),
                payload: Some(json!({
                    "path": "providers.openai.model",
                    "layer": "workspace",
                    "value": "gpt-5",
                    "changed": true,
                    "validated": true,
                    "config_path": "/workspace/.agena/agena.json"
                })),
                ..RawOutput::default()
            },
            vec!["providers.openai.model", "workspace", "Validated"],
            false,
        ),
        (
            "settings.validate",
            RawOutput {
                text: "Settings validation completed with one warning.".into(),
                payload: Some(json!({
                    "valid": true,
                    "warnings": [{"path": "providers.openai.model", "message": "Uses a preview model"}],
                    "files": ["/workspace/.agena/agena.json", "/workspace/.agena/agena.local.json"]
                })),
                ..RawOutput::default()
            },
            vec!["preview model", "agena.local.json"],
            true,
        ),
        (
            "settings.delete",
            RawOutput {
                text: "Deleted providers.openai.model from the workspace settings.".into(),
                payload: Some(json!({
                    "path": "providers.openai.model",
                    "layer": "workspace",
                    "deleted": true,
                    "validated": true,
                    "config_path": "/workspace/.agena/agena.json"
                })),
                ..RawOutput::default()
            },
            vec!["providers.openai.model", "Deleted", "true"],
            false,
        ),
        (
            "settings.patch",
            RawOutput {
                text: "Patched workspace settings and validated the merged configuration.".into(),
                payload: Some(json!({
                    "layer": "workspace",
                    "changed": true,
                    "validated": true,
                    "updated_paths": ["providers.openai.model", "providers.openai.timeout"],
                    "config_path": "/workspace/.agena/agena.json"
                })),
                ..RawOutput::default()
            },
            vec!["workspace", "providers.openai.timeout", "Validated"],
            false,
        ),
        (
            "tools.list",
            RawOutput {
                text: "Available tools: returned 2 of 2 starting at offset 0.\n- fs.read [filesystem] (agena.fs): Read a file\n- browser_snapshot [browser] (agena.web): Inspect a page".into(),
                ..RawOutput::default()
            },
            vec!["fs.read", "browser_snapshot", "filesystem"],
            false,
        ),
        (
            "tools.search",
            RawOutput {
                text: "Matching tools for \"image\": returned 1 of 1 starting at offset 0.\n- openai.image_generation [network]: Generate an image".into(),
                ..RawOutput::default()
            },
            vec!["openai.image_generation", "Generate an image"],
            false,
        ),
        (
            "tools.help",
            RawOutput {
                text: "Tool: fs.read\nTags: filesystem, query\nUsage:\n- `file_path` (string, required)\nExamples:\n- {\"file_path\":\"README.md\"}\nHelp:\nRead a bounded file preview.".into(),
                ..RawOutput::default()
            },
            vec!["fs.read", "file_path", "README.md"],
            false,
        ),
        (
            "tools.tags",
            RawOutput {
                text: "Available tool tags: returned 2 of 2 starting at offset 0.\n- filesystem: 14\n- discovery: 37".into(),
                ..RawOutput::default()
            },
            vec!["filesystem", "discovery", "37"],
            false,
        ),
        (
            "plugins.list",
            RawOutput {
                text: "Available plugins: returned 1 of 1 starting at offset 0.\n- agena.web [network] (v0.1.0): Browser and web tools · tools: browser_open, browser_snapshot".into(),
                ..RawOutput::default()
            },
            vec!["agena.web", "browser_snapshot", "0.1.0"],
            false,
        ),
        (
            "plugins.search",
            RawOutput {
                text: "Matching plugins for \"memory\": returned 1 of 1 starting at offset 0.\n- agena.memory [filesystem, discovery]: Durable workspace memory".into(),
                ..RawOutput::default()
            },
            vec!["agena.memory", "Durable workspace memory"],
            false,
        ),
        (
            "plugins.tags",
            RawOutput {
                text: "Available plugin tags: returned 2 of 2 starting at offset 0.\n- filesystem: 4\n- interactive: 6".into(),
                ..RawOutput::default()
            },
            vec!["filesystem", "interactive", "6"],
            false,
        ),
        (
            "skills.list",
            RawOutput {
                text: "- renderer-notes [skill]: Rendering conventions\n- review [command]: Review a change".into(),
                payload: Some(json!({
                    "tools": [
                        {"name": "renderer-notes", "kind": "skill", "summary": "Rendering conventions", "source": "workspace", "content_hash": "skillhash1", "editable": true},
                        {"name": "review", "kind": "command", "summary": "Review a change", "source": "builtin", "content_hash": "skillhash2", "editable": false}
                    ],
                    "diagnostics": [],
                    "total": 2,
                    "offset": 0,
                    "returned": 2,
                    "kind": null
                })),
                ..RawOutput::default()
            },
            vec!["renderer-notes", "skillhash1", "workspace"],
            true,
        ),
        (
            "skills.get",
            RawOutput {
                text: "Name: renderer-notes\nKind: skill\nSummary: Rendering conventions\n\nBody:\nKeep human output concise.".into(),
                payload: Some(json!({
                    "name": "renderer-notes",
                    "kind": "skill",
                    "summary": "Rendering conventions",
                    "body": "Keep human output concise.",
                    "aliases": ["rendering"],
                    "source_path": ".agena/skills/renderer-notes/SKILL.md",
                    "source": "workspace",
                    "content_hash": "skillhash1",
                    "document": "---\nname: renderer-notes\n---\nKeep human output concise.",
                    "editable": true
                })),
                ..RawOutput::default()
            },
            vec!["renderer-notes", "Keep human output concise", "SKILL.md"],
            false,
        ),
        (
            "session.environment",
            RawOutput {
                text: "Workspace: /workspace\nGit: main @ abc123\nShell: /bin/zsh\nOS: macos arm64".into(),
                payload: Some(json!({
                    "workspace_root": "/workspace",
                    "git_branch": "main",
                    "git_short_sha": "abc123",
                    "git_dirty": true,
                    "shell": "/bin/zsh",
                    "os": "macos",
                    "arch": "arm64"
                })),
                ..RawOutput::default()
            },
            vec!["/workspace", "abc123", "arm64"],
            false,
        ),
        (
            "session.model",
            RawOutput {
                text: "Model: openai/responses/gpt-5; thinking: high; verbosity: concise".into(),
                payload: Some(json!({
                    "session_id": 42,
                    "model_provider_id": "openai",
                    "model_adapter_id": "responses",
                    "model_id": "gpt-5",
                    "thinking_mode": "high",
                    "speed_mode": "fast",
                    "verbosity": "concise",
                    "model_context_window_tokens": 200000,
                    "model_max_input_tokens": 180000,
                    "model_max_output_tokens": 32000
                })),
                ..RawOutput::default()
            },
            vec!["responses", "gpt-5", "200000"],
            false,
        ),
        (
            "session.tokens",
            RawOutput {
                text: "Tokens: 12000 used; measured 11000; projected 15000; limit 20000; remaining 5000; reserved 1000.".into(),
                payload: Some(json!({
                    "session_id": 42,
                    "current_tokens": 12000,
                    "measured_prompt_tokens": 11000,
                    "projected_tokens": 15000,
                    "limit_tokens": 20000,
                    "remaining_tokens": 5000,
                    "usage_ratio": 0.6,
                    "reserved_tokens": 1000
                })),
                ..RawOutput::default()
            },
            vec!["12000", "5000", "0.6"],
            false,
        ),
        (
            "tasks.list",
            RawOutput {
                text: "2 delegated tasks: t-1 completed, t-2 running".into(),
                payload: Some(json!({
                    "tasks": [
                        {"task_id": "t-1", "parent_session_id": 42, "status": "completed", "description": "Inspect files", "access": "read_only", "model_id": "gpt-5"},
                        {"task_id": "t-2", "parent_session_id": 42, "status": "running", "description": "Run tests", "access": "read_write", "model_id": "gpt-5"}
                    ],
                    "timed_out": false
                })),
                ..RawOutput::default()
            },
            vec!["t-1", "t-2", "read_only"],
            true,
        ),
        (
            "tasks.output",
            RawOutput {
                text: "[assistant] The renderer is ready.\n[tool] 2 tests passed.".into(),
                payload: Some(json!({
                    "task": {"task_id": "t-1", "status": "completed"},
                    "chunks": [
                        {"role": "assistant", "text": "The renderer is ready."},
                        {"role": "tool", "text": "2 tests passed."}
                    ],
                    "next_cursor": 8,
                    "has_more": false
                })),
                ..RawOutput::default()
            },
            vec!["t-1", "tests passed", "Next Cursor"],
            true,
        ),
        (
            "code.search_ast",
            RawOutput {
                text: "2 structural matches in 4 files\nsrc/lib.rs:7\nsrc/main.rs:12".into(),
                payload: Some(json!({
                    "language": "rust",
                    "pattern": "fn $NAME() { $$$ }",
                    "scanned_files": 4,
                    "matches": [
                        {"path": "src/lib.rs", "line": 7, "column": 1, "text": "fn render() {}"},
                        {"path": "src/main.rs", "line": 12, "column": 1, "text": "fn main() {}"}
                    ]
                })),
                ..RawOutput::default()
            },
            vec!["src/lib.rs", "src/main.rs", "rust"],
            true,
        ),
        (
            "code.syntax_tree",
            RawOutput {
                text: "Syntax tree · src/lib.rs\nroot source_file\nno parse errors".into(),
                payload: Some(json!({
                    "path": "src/lib.rs",
                    "language": "rust",
                    "root_kind": "source_file",
                    "has_error": false,
                    "tree": {"kind": "function_item", "name": "render", "children": ["identifier", "block"]}
                })),
                ..RawOutput::default()
            },
            vec!["src/lib.rs", "function_item", "render"],
            false,
        ),
        (
            "shell.list",
            RawOutput {
                text: "1 managed process.".into(),
                payload: Some(json!({
                    "action": "list",
                    "processes": [{
                        "process_id": "proc-1",
                        "command": "cargo test",
                        "description": "Test run",
                        "status": "running",
                        "background": true,
                        "monitored": false,
                        "started_at_ms": 10,
                        "buffered_lines": 3,
                        "last_seq": 3,
                        "dropped_lines": 0
                    }],
                    "last_seq": 3,
                    "has_more": false,
                    "dropped_lines": 0
                })),
                ..RawOutput::default()
            },
            vec!["proc-1", "cargo test", "running"],
            true,
        ),
        (
            "shell.logs",
            RawOutput {
                text: "test result: ok".into(),
                payload: Some(json!({
                    "action": "logs",
                    "process_id": "proc-1",
                    "events": [{"seq": 4, "stream": "stdout", "ts_ms": 20, "line": "test result: ok"}],
                    "last_seq": 4,
                    "has_more": false,
                    "dropped_lines": 0
                })),
                ..RawOutput::default()
            },
            vec!["proc-1", "test result: ok", "Last event"],
            false,
        ),
        (
            "report.findings",
            RawOutput {
                text: "- [high] src/lib.rs:7 — Example finding (confidence 0.90)\n  Example finding body".into(),
                payload: Some(json!({
                    "summary": "Review complete",
                    "findings": [{
                        "severity": "high",
                        "file": "src/lib.rs",
                        "line": 7,
                        "title": "Example finding",
                        "body": "Example finding body",
                        "confidence": 0.9
                    }],
                    "counts": {"high": 1, "medium": 0}
                })),
                ..RawOutput::default()
            },
            vec!["Review complete", "src/lib.rs", "high"],
            true,
        ),
        (
            "plan.get",
            RawOutput {
                text: "# Renderer cleanup\n\nImprove tool presentation.\n\n## Steps\n\n1. **Implement renderer** — in progress\n   Use shared blocks".into(),
                payload: Some(json!({
                    "plan": {
                        "title": "Renderer cleanup",
                        "objective": "Improve tool presentation",
                        "phase": "planning",
                        "autorun": true,
                        "steps": [{
                            "title": "Implement renderer",
                            "status": "in_progress",
                            "note": "Use shared blocks",
                            "checkpoints": [{"text": "Add tests", "status": "pending"}]
                        }]
                    },
                    "view": "full",
                    "current_step": 1
                })),
                ..RawOutput::default()
            },
            vec!["Renderer cleanup", "planning", "Add tests"],
            true,
        ),
        (
            "plan.review",
            RawOutput {
                text: "Plan review decision: approve.\n\nRenderer cleanup is now active.".into(),
                payload: Some(json!({
                    "decision": "approve",
                    "plan": {
                        "title": "Renderer cleanup",
                        "phase": "active",
                        "steps": [{"title": "Implement renderer", "status": "in_progress"}]
                    }
                })),
                ..RawOutput::default()
            },
            vec!["approve", "Renderer cleanup", "active"],
            true,
        ),
        (
            "fs.view_image",
            RawOutput {
                text: "Attached 'assets/diagram.png' for visual inspection (detail=high, 4096 bytes).".into(),
                payload: Some(json!({
                    "path": "assets/diagram.png",
                    "detail": "high",
                    "mime": "image/png",
                    "size_bytes": 4096,
                    "sha256": "deadbeef"
                })),
                ..RawOutput::default()
            },
            vec!["assets/diagram.png", "image/png", "deadbeef"],
            false,
        ),
        (
            "openai.image_generation",
            RawOutput {
                text: "Saved OpenAI image artifact to '/workspace/generated.png'.".into(),
                payload: Some(json!({
                    "provider": "openai",
                    "model": "gpt-image-1",
                    "path": "/workspace/generated.png",
                    "mime": "image/png",
                    "size_bytes": 8192,
                    "sha256": "imagehash",
                    "revised_prompt": "A watercolor map of a floating city"
                })),
                ..RawOutput::default()
            },
            vec!["gpt-image-1", "/workspace/generated.png", "floating city"],
            false,
        ),
        (
            "browser_snapshot",
            RawOutput {
                text: "Title: Agena docs\nURL: https://example.test/docs\nInteractive elements: 1\n\nWelcome to the docs".into(),
                payload: Some(json!({
                    "session_id": "session-1",
                    "snapshot": {
                        "title": "Agena docs",
                        "url": "https://example.test/docs",
                        "text": "Welcome to the docs",
                        "elements": [{"ref": "e1", "role": "link", "name": "API reference"}]
                    }
                })),
                ..RawOutput::default()
            },
            vec!["session-1", "API reference", "https://example.test/docs"],
            true,
        ),
        (
            "browser_list",
            RawOutput {
                text: "1 managed browser page target(s).".into(),
                payload: Some(json!({
                    "browser_running": true,
                    "sessions": [{
                        "session_id": "session-1",
                        "title": "Agena docs",
                        "url": "https://example.test/docs",
                        "attached": true
                    }]
                })),
                ..RawOutput::default()
            },
            vec!["session-1", "Agena docs", "https://example.test/docs"],
            true,
        ),
        (
            "browser_open",
            RawOutput {
                text: "Opened https://example.test/docs in browser session session-1.".into(),
                payload: Some(json!({
                    "session_id": "session-1",
                    "snapshot": {"title": "Agena docs", "url": "https://example.test/docs", "elements": []},
                    "preflight_redirects": [],
                    "document_requests_intercepted": true
                })),
                ..RawOutput::default()
            },
            vec!["session-1", "Agena docs", "Document Requests Intercepted"],
            false,
        ),
        (
            "browser_click",
            RawOutput {
                text: "Completed browser click in browser session session-1.".into(),
                payload: Some(json!({
                    "session_id": "session-1",
                    "result": {
                        "action": {"ok": true, "method": "css"},
                        "snapshot": {
                            "title": "Agena docs",
                            "url": "https://example.test/docs/api",
                            "elements": [{"ref": "e2", "role": "button", "name": "Run"}]
                        }
                    }
                })),
                ..RawOutput::default()
            },
            vec!["session-1", "https://example.test/docs/api", "Run"],
            true,
        ),
        (
            "browser_type",
            RawOutput {
                text: "Completed browser type in browser session session-1.".into(),
                payload: Some(json!({
                    "session_id": "session-1",
                    "result": {"ok": true, "value": "agena", "method": "ref"},
                    "snapshot": {"title": "Search", "url": "https://example.test/search?q=agena", "elements": []}
                })),
                ..RawOutput::default()
            },
            vec!["session-1", "agena", "https://example.test/search"],
            false,
        ),
        (
            "browser_wait",
            RawOutput {
                text: "Completed browser wait in browser session session-1.".into(),
                payload: Some(json!({
                    "session_id": "session-1",
                    "condition": "text:Ready",
                    "elapsed_ms": 250,
                    "snapshot": {"title": "Ready", "url": "https://example.test/ready", "elements": []}
                })),
                ..RawOutput::default()
            },
            vec!["text:Ready", "250", "https://example.test/ready"],
            false,
        ),
        (
            "browser_screenshot",
            RawOutput {
                text: "Saved browser screenshot to '/workspace/.agena/artifacts/browser/screen.png'.".into(),
                payload: Some(json!({
                    "session_id": "session-1",
                    "path": "/workspace/.agena/artifacts/browser/screen.png",
                    "size_bytes": 8192
                })),
                ..RawOutput::default()
            },
            vec!["session-1", "screen.png", "8192"],
            false,
        ),
        (
            "browser_download",
            RawOutput {
                text: "Saved browser download to '/workspace/downloads/report.pdf'.".into(),
                payload: Some(json!({
                    "session_id": "session-1",
                    "url": "https://example.test/report.pdf",
                    "path": "/workspace/downloads/report.pdf",
                    "size_bytes": 4096,
                    "preflight_redirects": ["https://cdn.example.test/report.pdf"]
                })),
                ..RawOutput::default()
            },
            vec!["report.pdf", "cdn.example.test", "4096"],
            false,
        ),
        (
            "chatgpt.web_search",
            RawOutput {
                text: "OpenAI found the latest Agena rendering guide.".into(),
                payload: Some(json!({
                    "provider": "openai",
                    "tool": "web_search",
                    "model": "gpt-5",
                    "request_id": "req-openai-1",
                    "response_id": "resp-1",
                    "pending_calls": [],
                    "assistant_content": [{"type": "output_text", "text": "Latest Agena rendering guide"}],
                    "sources": [{"title": "Agena rendering guide", "url": "https://example.test/guide", "domain": "example.test"}],
                    "usage": {"input_tokens": 100, "output_tokens": 40},
                    "response_receipt": {"path": ".agena/receipts/resp-1.json", "sha256": "receipt-hash", "binary_payloads_redacted": true},
                    "continuation_required": false
                })),
                ..RawOutput::default()
            },
            vec!["req-openai-1", "Agena rendering guide", "receipt-hash"],
            true,
        ),
        (
            "claude.bash",
            RawOutput {
                text: "Claude returned a bash tool result for the requested command.".into(),
                payload: Some(json!({
                    "provider": "claude",
                    "tool": "bash",
                    "model": "claude-sonnet",
                    "request_id": "req-claude-1",
                    "response_id": "msg-1",
                    "pending_calls": [{"type": "bash_20250124", "name": "bash", "id": "toolu-1"}],
                    "assistant_content": [{"type": "tool_use", "name": "bash", "input": {"command": "pwd"}}],
                    "sources": [],
                    "usage": {"input_tokens": 80, "output_tokens": 30},
                    "response_receipt": {"path": ".agena/receipts/msg-1.json", "sha256": "claude-receipt"},
                    "continuation_required": true
                })),
                ..RawOutput::default()
            },
            vec!["req-claude-1", "toolu-1", "claude-receipt"],
            true,
        ),
        (
            "gemini.google_search",
            RawOutput {
                text: "Gemini returned grounded search context.".into(),
                payload: Some(json!({
                    "provider": "gemini",
                    "tool": "google_search",
                    "model": "gemini-2.5-pro",
                    "request_id": "req-gemini-1",
                    "response_id": "int-1",
                    "pending_calls": [],
                    "assistant_content": [{"type": "text", "text": "Grounded result"}],
                    "sources": [{"title": "Agena docs", "url": "https://example.test/docs", "domain": "example.test"}],
                    "usage": {"input_tokens": 90, "output_tokens": 20},
                    "response_receipt": {"path": ".agena/receipts/int-1.json", "sha256": "gemini-receipt"},
                    "continuation_required": false
                })),
                ..RawOutput::default()
            },
            vec!["req-gemini-1", "gemini-2.5-pro", "gemini-receipt"],
            true,
        ),
        (
            "memory.search",
            RawOutput {
                text: "Found 1 memory item(s) matching 'renderer'.\n- renderer-notes [project]: Rendering conventions".into(),
                payload: Some(json!({
                    "query": "renderer",
                    "limit": 5,
                    "results": [{
                        "id": "memory-1",
                        "name": "renderer-notes",
                        "description": "Rendering conventions",
                        "memory_type": "project",
                        "body": "Keep human output concise and complete.",
                        "path": ".agena/memory/renderer-notes.md",
                        "searchable_text": "renderer rendering conventions"
                    }]
                })),
                ..RawOutput::default()
            },
            vec!["memory-1", "renderer-notes", ".agena/memory/renderer-notes.md"],
            true,
        ),
        (
            "repo.status",
            RawOutput {
                text: "Repository on branch main with one changed file.".into(),
                payload: Some(json!({
                    "root": "/workspace",
                    "branch": "main",
                    "head": "abc123",
                    "dirty": true,
                    "changes": [{"path": "src/lib.rs", "kind": "modified", "additions": 4, "deletions": 1}]
                })),
                ..RawOutput::default()
            },
            vec!["/workspace", "abc123", "src/lib.rs"],
            true,
        ),
        (
            "memory.list",
            RawOutput {
                text: "- renderer-notes [project]: Rendering conventions".into(),
                payload: Some(json!({
                    "limit": 50,
                    "memories": [{
                        "name": "renderer-notes",
                        "description": "Rendering conventions",
                        "memory_type": "project",
                        "path": ".agena/memory/renderer-notes.md",
                        "content_hash": "memoryhash"
                    }]
                })),
                ..RawOutput::default()
            },
            vec!["renderer-notes", "memoryhash", "50"],
            true,
        ),
        (
            "memory.get",
            RawOutput {
                text: "# Renderer notes\n\nKeep human output concise and complete.".into(),
                payload: Some(json!({
                    "name": "renderer-notes",
                    "description": "Rendering conventions",
                    "memory_type": "project",
                    "body": "Keep human output concise and complete.",
                    "path": ".agena/memory/renderer-notes.md",
                    "content_hash": "memoryhash"
                })),
                ..RawOutput::default()
            },
            vec!["renderer-notes", "Keep human output concise", "memoryhash"],
            false,
        ),
    ];

    let context = render_context();
    for (tool, raw, expected_fragments, expects_table) in fixtures {
        let blocks = BuiltinHumanRenderer::new(tool)
            .render_human(&context, &raw)
            .unwrap_or_else(|error| panic!("{tool} renderer failed: {error}"));
        assert!(
            blocks
                .iter()
                .any(|block| !matches!(block, ViewBlock::Json { .. })),
            "{tool} rendered only opaque JSON: {blocks:?}"
        );
        let serialized = serde_json::to_string(&blocks).expect("serialize human blocks");
        for fragment in expected_fragments {
            assert!(
                serialized.contains(fragment),
                "{tool} omitted expected fact {fragment:?}: {serialized}"
            );
        }
        if expects_table {
            assert!(
                blocks
                    .iter()
                    .any(|block| matches!(block, ViewBlock::Table { .. })),
                "{tool} should use a table for its repeated records: {blocks:?}"
            );
        }
    }
}
