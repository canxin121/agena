use std::path::PathBuf;

use agena_domain::{RawOutput, StructuredObject, ToolInvocation, ViewBlock};
use agena_runtime_tools::tool::human_view::BuiltinHumanRenderer;
use agena_tool::{RenderContext, ToolHumanRenderer, completed_tool_title, initial_tool_title};
use serde_json::{Value, json};

fn render_context() -> RenderContext {
    RenderContext {
        workspace_root: PathBuf::from("/tmp/agena-rendering-test"),
        command: None,
    }
}

fn sample_payload(tool: &str) -> Value {
    let cloud = agena_tool::provider_tools::cloud_tool(tool);
    if cloud.is_some_and(|tool| {
        matches!(
            tool.operation,
            "image_understanding" | "document_understanding"
        )
    }) {
        let cloud = cloud.unwrap();
        return json!({"provider":cloud.provider,"tool":cloud.operation,"model":"fixture","outcome":"completed","input_sent":true,"media_inputs":[{"filename":"input.png","mime":"image/png","size_bytes":80}],"text":"Understood fixture"});
    }
    if cloud.is_some_and(|tool| {
        matches!(
            tool.operation,
            "file_upload" | "file_status" | "file_delete"
        )
    }) {
        let cloud = cloud.unwrap();
        return json!({"provider":cloud.provider,"tool":cloud.operation,"state":"ready","handle":"media_fixture","filename":"input.png","mime":"image/png","size_bytes":80,"remote_file_id":"file_fixture"});
    }

    // Feed one canonical current operation shape through every registered identity spelling.
    let tool = agena_tool::provider_tools::canonical_cloud_identity(tool);
    let job = json!({
        "id": "job-1",
        "kind": "cron",
        "expression": "*/5 * * * *",
        "timezone": "UTC",
        "prompt": "check status",
        "next_fire_at": "2026-08-22T12:05:00Z",
        "paused": false,
        "completed": false,
        "misfire_policy": "skip",
        "retry_max_attempts": 1,
        "last_run_status": "completed"
    });
    let task = json!({
        "task_id": "task-1",
        "description": "Example delegated task",
        "status": "completed",
        "session_id": 42,
        "model_id": "gpt-5",
        "started_at_ms": 1,
        "finished_at_ms": 2
    });

    match tool {
        "fs.read" => json!({
            "preview": "fn main() {}",
            "loaded_paths": ["src/main.rs"],
            "truncated": false
        }),
        "fs.read_many" => json!({
            "files": [{
                "path": "src/lib.rs",
                "bytes": 128,
                "returned_bytes": 128,
                "truncated": false,
                "sha256": "abc123"
            }],
            "max_total_bytes": 1048576,
            "remaining_bytes": 1048448,
            "truncated": false
        }),
        "fs.write" => json!({
            "path": "src/lib.rs",
            "kind": "updated",
            "bytes": 128,
            "sha256": "abc123"
        }),
        "fs.replace" => json!({
            "path": "src/lib.rs",
            "replacements": 2,
            "before_sha256": "before",
            "after_sha256": "after"
        }),
        "fs.stat" => json!({
            "path": "src/lib.rs",
            "kind": "file",
            "size": 128,
            "modified_at_ms": 1000,
            "readonly": false,
            "sha256": "abc123",
            "hash_skipped": false
        }),
        "fs.glob" => json!({
            "count": 2,
            "paths": ["src/lib.rs", "src/main.rs"],
            "truncated": false
        }),
        "fs.grep" => json!({
            "matches": 2,
            "results": ["src/lib.rs:1: fn main", "src/main.rs:2: fn run"],
            "truncated": false
        }),
        "fs.apply_patch" => json!({
            "operation_id": "patch-1",
            "inverse_patch": "*** Begin Patch\n*** End Patch",
            "changes": [{"path": "src/lib.rs", "kind": "updated"}],
            "diff": "@@ -1 +1 @@\n-old\n+new"
        }),
        "code.search_ast" => json!({
            "language": "rust",
            "pattern": "fn $NAME()",
            "scanned_files": 4,
            "matches": [{"path": "src/lib.rs", "line": 7, "text": "fn render()"}]
        }),
        "code.syntax_tree" => json!({
            "path": "src/lib.rs",
            "language": "rust",
            "root_kind": "source_file",
            "has_error": false,
            "tree": {"kind": "source_file", "children": 3}
        }),
        "cron.create" => json!({"id": "job-1", "next_fire_at": "2026-08-22T12:05:00Z"}),
        "cron.list" => json!({"jobs": [job.clone()]}),
        "cron.delete" => json!({"id": "job-1", "removed": true}),
        "cron.update" => json!({"job": job.clone()}),
        "cron.pause" => json!({"job": {
            "id": "job-1",
            "kind": "cron",
            "expression": "*/5 * * * *",
            "timezone": "UTC",
            "prompt": "check status",
            "next_fire_at": "2026-08-22T12:05:00Z",
            "paused": true,
            "completed": false,
            "misfire_policy": "skip",
            "retry_max_attempts": 1,
                "last_run_status": "completed"
        }}),
        "cron.resume" => json!({"job": job.clone()}),
        "cron.history" => json!({
            "entries": [{
                "job_id": "job-1",
                "triggered_at": "2026-08-22T12:00:00Z",
                "finished_at": "2026-08-22T12:00:01Z",
                "status": "completed",
                "scheduled_for": "2026-08-22T12:00:00Z",
                "attempt": 1,
                "session_id": 42
            }]
        }),
        "interaction.ask" => json!({
            "questions": [{"question": "Continue?"}],
            "answers": {"0": ["yes"]},
            "timed_out": false
        }),
        "interaction.notify" => json!({
            "title": "Build",
            "level": "success",
            "body_markdown": "**Done**"
        }),
        "lsp.servers" => json!({
            "servers": [{"name": "rust-analyzer", "command": "rust-analyzer", "args": [], "file_extensions": ["rs"]}]
        }),
        "lsp.definition" => json!({"locations": ["src/lib.rs:7:1"]}),
        "lsp.references" => json!({"locations": ["src/main.rs:3:1"]}),
        "lsp.hover" => json!({"contents": "**render**"}),
        "lsp.diagnostics" => json!({"entries": ["src/lib.rs:1 warning"]}),
        "mcp.resources.list" => json!({
            "server": "demo",
            "resources": [{"name": "README", "uri": "mcp://demo/readme", "mime_type": "text/markdown", "description": "Docs"}],
            "next_cursor": "cursor-2"
        }),
        "mcp.resources.templates.list" => json!({
            "server": "demo",
            "resource_templates": [{"name": "User", "uri_template": "users://{id}", "mime_type": "application/json", "description": "Profile"}],
            "next_cursor": "cursor-2"
        }),
        "mcp.resources.read" => json!({
            "server": "demo",
            "uri": "mcp://demo/readme",
            "contents": [{"uri": "mcp://demo/readme", "mime_type": "text/markdown", "text": "# Hello"}]
        }),
        "mcp.prompts.list" => json!({
            "server": "demo",
            "prompts": [{"name": "review", "description": "Review code", "arguments": []}],
            "next_cursor": "cursor-2"
        }),
        "mcp.prompts.get" => json!({
            "server": "demo",
            "prompt": "review",
            "messages": [{"role": "user", "content": "Review this"}]
        }),
        "mcp.tools.call" => json!({
            "server": "demo",
            "tool": "search",
            "content": [{"type": "text", "text": "3 matches"}],
            "structured_content": {"matches": 3},
            "mcp_meta": {"request_id": "mcp-1"}
        }),
        "mcp.tools.search" => json!({
            "server": "demo",
            "results": [{"name": "search", "description": "Search docs", "server": "demo"}]
        }),
        "mcp.servers.status" => json!({
            "servers": [{"name": "demo", "connected": true, "tool_count": 3, "url": "https://mcp.test"}]
        }),
        "mcp.servers.reconnect" => json!({
            "server": "demo", "connected": true, "tool_count": 3
        }),
        "memory.search" => json!({
            "query": "release",
            "results": [{"name": "release-notes", "score": 0.9, "snippet": "..."}]
        }),
        "memory.get" => json!({"name": "release-notes", "body": "Ship safely."}),
        "memory.list" => json!({"memories": [{"name": "release-notes", "size": 128}]}),
        "memory.write" => json!({"name": "release-notes", "saved": true, "bytes": 128}),
        "memory.delete" => json!({"name": "release-notes", "deleted": true}),
        "monitor.start" => json!({
            "action": "start", "monitor_id": "mon-1", "status": "running", "processes": []
        }),
        "monitor.stop" => json!({
            "action": "stop", "monitor_id": "mon-1", "status": "stopped", "processes": []
        }),
        "notebook.edit_cell" => json!({
            "path": "demo.ipynb", "action": "replace", "cell_index": 0, "cell_count": 2,
            "before_sha256": "before", "after_sha256": "after", "changed": true
        }),
        "plan.clear" => json!({"cleared": true}),
        "plan.edit" | "plan.get" | "plan.phase" | "plan.review" | "plan.set" => json!({
            "plan": {
                "title": "Release",
                "objective": "Ship safely",
                "phase": "active",
                "steps": [{"title": "Test", "status": "completed", "checkpoints": [{"text": "CI", "status": "passed"}]}]
            },
            "current_step": {"title": "Test", "status": "completed"},
            "current_step_index": 0,
            "decision": "approved"
        }),
        "report.findings" => json!({
            "summary": "Review complete",
            "findings": [{"severity": "high", "file": "src/lib.rs", "line": 7, "title": "Example finding", "body": "Fix this", "confidence": 0.9}],
            "counts": {"high": 1}
        }),
        "session.environment" => json!({
            "workspace_root": "/workspace", "git_branch": "main", "git_short_sha": "abc123", "git_dirty": true,
            "shell": "/bin/zsh", "os": "macos", "arch": "aarch64"
        }),
        "session.get" | "session.rename" => json!({
            "session": {"id": 42, "title": "Release", "parent_id": null, "root_id": 42, "is_subagent": false}
        }),
        "session.model" => json!({
            "model_provider_id": "openai", "model_adapter_id": "responses", "model_id": "gpt-5",
            "thinking_mode": "high", "model_context_window_tokens": 128000
        }),
        "session.tokens" => json!({
            "current_tokens": 12000, "measured_prompt_tokens": 11000, "projected_tokens": 13000,
            "limit_tokens": 16000, "remaining_tokens": 4000, "reserved_tokens": 1000, "usage_ratio": 0.75
        }),
        "settings.get" => {
            json!({"path": "providers.openai.model", "layer": "workspace", "value": "gpt-5"})
        }
        "settings.list" => {
            json!({"items": [{"path": "providers.openai.model", "value": "gpt-5"}], "count": 1})
        }
        "settings.inspect" => json!({
            "path": "providers.openai", "global": {"defined": true}, "workspace": {"defined": false},
            "layers": [{"name": "global", "active": true}]
        }),
        "settings.validate" => {
            json!({"valid": true, "warnings": [{"path": "model", "message": "uses default"}]})
        }
        "settings.set" | "settings.patch" => json!({
            "path": "providers.openai.model", "layer": "workspace", "changed": true, "validated": true,
            "current": "gpt-5", "updated_paths": ["providers.openai.model"]
        }),
        "settings.delete" => {
            json!({"path": "providers.openai.model", "deleted": true, "changed": true})
        }
        "shell.run" => json!({
            "action": "run", "shell": "bash", "background": false, "status": "exited",
            "output": "all tests passed", "exit_code": 0, "process_id": "p-1"
        }),
        "shell.write" | "shell.resize" | "shell.signal" => json!({
            "action": tool.split('.').next_back().unwrap(), "background": true,
            "status": "running", "process_id": "p-1", "output": "PROMPT> ",
            "terminal": { "rows": 24, "cols": 80, "cursor_row": 0, "cursor_col": 8,
                "cursor_visible": true, "alternate_screen": false, "bracketed_paste": false,
                "application_cursor": false, "text": "PROMPT> ", "truncated": false }
        }),
        "shell.list" => {
            json!({"action": "list", "processes": [{
                "process_id": "p-1",
                "command": "cargo test",
                "description": "Run the test suite",
                "status": "running",
                "background": true,
                "monitored": false,
                "started_at_ms": 1,
                "ended_at_ms": null,
                "buffered_lines": 2,
                "last_seq": 2,
                "dropped_lines": 0,
                "exit_code": null,
                "completion_reason": null
            }]})
        }
        "shell.logs" => {
            json!({"action": "logs", "process_id": "p-1", "events": [{"seq": 1, "stream": "stdout", "ts_ms": 1, "line": "ok"}], "last_seq": 1})
        }
        "shell.stop" => {
            json!({"action": "stop", "process_id": "p-1", "status": "stopped", "exit_code": 143})
        }
        "commands.list" => {
            json!({"packages": [{"name": "review", "summary": "Review changes", "source": "workspace", "editable": true}], "returned": 1, "total": 1, "offset": 0})
        }
        "commands.get" => {
            json!({"name": "review", "source": "workspace", "body": "Review changes.", "content_hash": "hash-1", "editable": true, "revision": "rev-1"})
        }
        "commands.install" | "commands.remove" => {
            json!({"operation": if tool.ends_with("remove") { "removed" } else { "installed" }, "name": "review", "path": ".agena/skills/review/SKILL.md", "catalog_generation": 3, "catalog_changed": true, "editable": true})
        }
        "commands.read_resource" => {
            json!({"name": "review", "path": "references/checklist.md", "source": "workspace", "bytes": 42, "content_hash": "hash-2", "content": "Checklist"})
        }
        "commands.refresh" => {
            json!({"changed": true, "generation": 3, "declared": 14, "external": 0})
        }
        "snapshot.enter" => {
            json!({"path": "/tmp/snapshot", "branch": "snapshot/main", "backend": "git", "note": "before release"})
        }
        "snapshot.exit" => json!({"action": "restore", "path": "/tmp/snapshot"}),
        "snapshot.status" => {
            json!({"snapshots": [{"session_id": 42, "path": "/tmp/snapshot", "branch": "snapshot/main", "created_here": true}]})
        }
        "tasks.run" => json!({
            "task_id": "task-1", "session_id": 42, "parent_session_id": 0, "status": "completed",
            "resumed": false, "final_text": "Task completed.", "model_provider_id": "openai", "model_id": "gpt-5",
            "input_tokens": 10, "output_tokens": 20, "reasoning_tokens": 5, "cache_write_tokens": 0, "cache_read_tokens": 0,
            "total_cost_microusd": 12
        }),
        "tasks.list" => json!({"tasks": [task.clone()]}),
        "tasks.get" | "tasks.cancel" | "tasks.followup" | "tasks.message" => {
            json!({"task": task.clone()})
        }
        "tasks.output" => {
            json!({"task": task.clone(), "chunks": [{"role": "assistant", "text": "done"}], "next_cursor": 1, "has_more": true})
        }
        "web.fetch" => {
            json!({"url": "https://example.test", "status": 200, "cached": false, "truncated": false, "summary": "Example page", "markdown": "# Example"})
        }
        "web.search" => {
            json!({"query": "Agena", "backend": "default", "results": [{"title": "Guide", "url": "https://example.test", "snippet": "Docs"}]})
        }
        "web.crawl" => json!({
            "start_url": "https://example.test", "engine": "spider", "stored_count": 2, "cached_count": 1, "failure_count": 0,
            "documents": [{"title": "Home", "url": "https://example.test", "depth": 0, "chunk_count": 3, "fetched_at": "2026-08-22T10:00:00Z"}]
        }),
        "web.browser_list" => {
            json!({"sessions": [{"session_id": "s-1", "title": "Agena docs", "url": "https://example.test", "attached": true}], "browser_running": true})
        }
        "web.browser_open"
        | "web.browser_snapshot"
        | "web.browser_click"
        | "web.browser_type"
        | "web.browser_wait" => json!({
            "session_id": "s-1", "condition": "ready", "elapsed_ms": 20,
            "snapshot": {"title": "Agena docs", "url": "https://example.test/docs", "text": "Welcome", "elements": [{"ref": "e1", "role": "link", "name": "API", "selector": "#api"}]}
        }),
        "web.browser_close" => json!({"session_id": "s-1", "closed": true}),
        "web.browser_shutdown" => json!({"closed": true}),
        "web.browser_screenshot" => {
            json!({"session_id": "s-1", "path": "/tmp/page.png", "size_bytes": 1024})
        }
        "web.browser_download" => {
            json!({"session_id": "s-1", "url": "https://example.test/a.zip", "path": "/tmp/a.zip", "size_bytes": 2048})
        }
        _ if tool.starts_with("chatgpt.")
            || tool.starts_with("claude.")
            || tool.starts_with("gemini.") =>
        {
            provider_sample_payload(tool)
        }
        _ => {
            json!({"status": "completed", "message": "The operation completed.", "count": 1, "details": {"available": true}})
        }
    }
}

fn sample_input(tool: &str) -> Value {
    match tool {
        "fs.read" => json!({"file_path": "src/main.rs"}),
        "fs.read_many" => json!({"paths": ["src/lib.rs", "src/main.rs"]}),
        "fs.write" | "fs.replace" | "fs.stat" => {
            json!({"path": "src/lib.rs"})
        }
        "fs.apply_patch" => {
            json!({"patch": "*** Begin Patch\n*** Update File: src/lib.rs\n*** End Patch"})
        }
        "fs.glob" | "fs.grep" => json!({"pattern": "TODO", "path": "src"}),
        "code.search_ast" => json!({"pattern": "fn $NAME()", "path": "src", "language": "rust"}),
        "code.syntax_tree" => json!({"path": "src/lib.rs", "language": "rust"}),
        "shell.run" => json!({"command": "cargo test"}),
        "shell.logs" | "shell.stop" => json!({"process_id": "p-1"}),
        "shell.write" => {
            json!({"process_id": "p-1", "chars": "hello\r", "reads": [], "writes": [], "network": []})
        }
        "shell.resize" => json!({"process_id": "p-1", "rows": 30, "cols": 100}),
        "shell.signal" => json!({"process_id": "p-1", "signal": "interrupt"}),
        "monitor.start" => json!({"command": "cargo watch"}),
        "monitor.stop" => json!({"monitor_id": "mon-1"}),
        "interaction.ask" => json!({"questions": [{"question": "Continue?"}]}),
        "interaction.notify" => json!({"title": "Build", "body": "Done"}),
        "lsp.definition" | "lsp.references" | "lsp.hover" | "lsp.diagnostics" => json!({
            "position": {"file_path": "src/lib.rs", "line": 9, "character": 3}
        }),
        "mcp.tools.call" => json!({"server": "demo", "name": "search"}),
        "mcp.tools.search" => json!({"server": "demo", "query": "search"}),
        value if value.starts_with("mcp.") => json!({"server": "demo"}),
        "memory.search" => json!({"query": "release"}),
        "memory.get" | "memory.write" | "memory.delete" => {
            json!({"name": "release-notes"})
        }
        "plan.set" | "plan.update" => json!({"title": "Release"}),
        "plan.phase" => json!({"phase": "implementation"}),
        "plan.review" => json!({"decision": "approve"}),
        value if value.starts_with("plan.") => json!({}),
        "tasks.run" => json!({"description": "Run release checks"}),
        value if value.starts_with("tasks.") => json!({"task_id": "task-1"}),
        "commands.install" | "commands.remove" | "commands.get" => {
            json!({"name": "review"})
        }
        "commands.read_resource" => json!({"name": "review", "path": "references/checklist.md"}),
        value if value.starts_with("commands.") => json!({}),
        value if value.starts_with("settings.") => json!({"path": "providers.openai.model"}),
        "session.rename" => json!({"title": "Release"}),
        value if value.starts_with("session.") => json!({}),
        "snapshot.enter" => json!({"path": "/tmp/snapshot"}),
        "snapshot.exit" => json!({"path": "/tmp/snapshot"}),
        value if value.starts_with("snapshot.") => json!({}),
        "notebook.edit_cell" => json!({"notebook_path": "demo.ipynb", "cell": 0}),
        "report.findings" => json!({"summary": "Review release"}),
        "web.search" => json!({"query": "Agena"}),
        "web.fetch" | "web.crawl" => json!({"url": "https://example.test"}),
        "web.browser_open" => json!({"url": "https://example.test/docs"}),
        "web.browser_click" => json!({"selector": "#submit"}),
        "web.browser_type" => json!({"selector": "#query"}),
        "web.browser_wait" => json!({"condition": "ready"}),
        "web.browser_screenshot" => json!({"session_id": "s-1"}),
        "web.browser_download" => json!({"url": "https://example.test/a.zip"}),
        "web.browser_close" => json!({"session_id": "s-1"}),
        "web.browser_list" | "web.browser_shutdown" => json!({}),
        value
            if value.starts_with("tools.plugins_search") || value.starts_with("plugins.search") =>
        {
            json!({"query": "filesystem"})
        }
        value if value.starts_with("tools.plugins_tags") || value.starts_with("plugins.tags") => {
            json!({"tag": "filesystem"})
        }
        value if value.starts_with("tools.search") || value.starts_with("tools_search") => {
            json!({"query": "filesystem"})
        }
        value if value.starts_with("tools.") || value.starts_with("plugins.") => json!({}),
        value
            if value.starts_with("chatgpt.")
                || value.starts_with("claude.")
                || value.starts_with("gemini.")
                || value.starts_with("openai.") =>
        {
            provider_sample_input(value)
        }
        _ => json!({}),
    }
}

fn provider_sample_input(tool: &str) -> Value {
    let operation = agena_tool::provider_tools::cloud_tool(tool)
        .map(|tool| tool.operation)
        .unwrap_or_else(|| tool.rsplit('.').next().unwrap_or_default());
    match operation {
        "web_search" | "google_search" | "google_maps" | "file_search" => {
            json!({"query": "release policy"})
        }
        "web_fetch" | "url_context" => json!({"url": "https://example.test"}),
        "code_interpreter" | "code_execution" => json!({"command": "cargo test"}),
        "shell" => json!({"command": "cargo test"}),
        "image_generation" | "image_edit" => json!({"prompt": "A polished release diagram"}),
        "advisor" => json!({"prompt": "Review this change"}),
        "image_understanding" | "document_understanding" => {
            json!({"prompt": "Analyze this fixture"})
        }
        "file_upload" => json!({"path": "input.png"}),
        "file_status" | "file_delete" => json!({"handle": "media_fixture"}),
        _ => json!({}),
    }
}

fn provider_sample_payload(tool: &str) -> Value {
    let cloud = agena_tool::provider_tools::cloud_tool(tool);
    let provider = cloud
        .map(|tool| tool.provider)
        .unwrap_or_else(|| tool.split('.').next().unwrap_or("provider"));
    let operation = cloud
        .map(|tool| tool.operation)
        .unwrap_or_else(|| tool.rsplit('.').next().unwrap_or("operation"));
    let mut payload = json!({
        "provider": provider,
        "tool": operation,
        "model": "example-model",
        "response_id": "response-1"
    });
    let object = payload.as_object_mut().expect("provider payload object");
    match operation {
        "file_search" => {
            object.insert("query".into(), json!("rendering"));
            object.insert(
                "results".into(),
                json!([
                    {"file_name": "README.md", "score": 0.9, "snippet": "Rendering guide"},
                    {"file_name": "guide.md", "score": 0.8, "snippet": "Examples"}
                ]),
            );
        }
        "web_search" | "google_search" => {
            object.insert("sources".into(), json!([
                {"title": "Agena guide", "url": "https://example.test/guide", "domain": "example.test", "snippet": "Guide"}
            ]));
            object.insert(
                "assistant_content".into(),
                json!([{"type": "text", "text": "A concise answer."}]),
            );
        }
        "google_maps" => {
            object.insert("query".into(), json!("cafes near me"));
            object.insert("places".into(), json!([
                {"name": "Cafe One", "address": "1 Main St", "rating": 4.8, "url": "https://example.test/cafe"}
            ]));
        }
        "url_context" | "web_fetch" => {
            object.insert("url".into(), json!("https://example.test"));
            object.insert("status".into(), json!(200));
            object.insert("fetched_urls".into(), json!(["https://example.test"]));
        }
        "code_execution" | "code_interpreter" => {
            object.insert("status".into(), json!("completed"));
            object.insert("exit_code".into(), json!(0));
            object.insert(
                "outputs".into(),
                json!([{"type": "text", "text": "passed"}]),
            );
        }
        "shell" => {
            object.insert(
                "pending_calls".into(),
                json!([{
                    "type": format!("{operation}_call"),
                    "id": "call-1",
                    "status": "in_progress",
                    "action": {"type": "exec", "command": "cargo test"}
                }]),
            );
            object.insert("continuation_required".into(), json!(true));
        }
        "advisor" => {
            object.insert(
                "assistant_content".into(),
                json!([{"type": "text", "text": "Use a small patch."}]),
            );
        }
        "image_generation" | "image_edit" => {
            object.insert("path".into(), json!("/tmp/image.png"));
            object.insert("mime".into(), json!("image/png"));
            object.insert("image_count".into(), json!(1));
            object.insert("size_bytes".into(), json!(4096));
            object.insert("sha256".into(), json!("abc123"));
            object.insert("revised_prompt".into(), json!("A polished image"));
        }
        _ => {
            object.insert(
                "assistant_content".into(),
                json!([{"type": "text", "text": "Response received."}]),
            );
        }
    }
    payload
}

fn sample_text(tool: &str) -> String {
    match tool {
        "tools.plugins_list" => {
            "Available plugins: returned 1 of 1 starting at offset 0.\n- agena.fs [filesystem, execute] (v0.1.0): Filesystem tools · tools: fs.read, fs.write"
                .into()
        }
        "tools.plugins_search" => {
            "Matching plugins for \"file\": returned 1 of 1 starting at offset 0.\n- agena.fs [filesystem]: Filesystem tools"
                .into()
        }
        "tools.plugins_tags" => {
            "Available plugin tags: returned 2 of 2 starting at offset 0.\n- filesystem: 3\n- execute: 2"
                .into()
        }
        "memory.delete" => "Deleted memory 'release-notes'.".into(),
        _ => String::new(),
    }
}

fn sample_raw(tool: &str) -> RawOutput {
    let text = sample_text(tool);
    RawOutput::from_parts(
        text.is_empty().then(|| sample_payload(tool)),
        text,
        Vec::new(),
        Vec::new(),
        false,
    )
}

fn has_tool_specific_projection(tool: &str, blocks: &[ViewBlock]) -> bool {
    let ids = blocks.iter().filter_map(ViewBlock::block_id);
    if tool.starts_with("chatgpt.") || tool.starts_with("claude.") || tool.starts_with("gemini.") {
        return ids.into_iter().any(|id| {
            id.starts_with("provider-")
                && !matches!(
                    id,
                    "provider-meta"
                        | "provider-usage"
                        | "provider-receipt"
                        | "provider-content"
                        | "provider-error"
                )
        });
    }
    if tool.starts_with("tools.") || tool.starts_with("plugins.") {
        return ids.into_iter().any(|id| id.starts_with("discovery-"));
    }
    ids.into_iter()
        .any(|id| id != "result" && !id.starts_with("result-"))
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
            let raw = sample_raw(compact_name);
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

    assert_eq!(checked, 131);
}

#[test]
fn every_bundled_execution_tool_has_a_tool_specific_human_projection() {
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
            let blocks = BuiltinHumanRenderer::new(compact_name)
                .render_human(&context, &sample_raw(compact_name))
                .expect("bundled renderer should not fail");
            assert!(
                !blocks
                    .iter()
                    .any(|block| matches!(block, ViewBlock::Json { .. })),
                "{compact_name} must not expose an opaque JSON presentation: {blocks:?}"
            );
            assert!(
                blocks
                    .iter()
                    .all(|block| { !block.block_id().is_some_and(|id| id.starts_with("result-")) }),
                "{compact_name} must not use generic nested result blocks: {blocks:?}"
            );
            assert!(
                has_tool_specific_projection(compact_name, &blocks),
                "{compact_name} only produced generic result blocks: {blocks:?}"
            );
        }
    }

    assert_eq!(checked, 131);
}

#[test]
fn every_bundled_execution_tool_has_a_typed_empty_state_projection() {
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
            let blocks = BuiltinHumanRenderer::new(compact_name)
                .render_human(&context, &RawOutput::default())
                .expect("bundled renderer should not fail for an empty result");
            assert!(
                !blocks
                    .iter()
                    .any(|block| matches!(block, ViewBlock::Json { .. })),
                "{compact_name} must not expose JSON for an empty result"
            );
            assert!(
                has_tool_specific_projection(compact_name, &blocks),
                "{compact_name} has no typed empty-state presentation"
            );
        }
    }

    assert_eq!(checked, 131);
}

#[test]
fn high_risk_tool_families_use_stable_operation_blocks() {
    let context = render_context();
    let render_ids = |tool: &str, raw: RawOutput| {
        BuiltinHumanRenderer::new(tool)
            .render_human(&context, &raw)
            .expect("renderer should not fail")
            .into_iter()
            .filter_map(|block| block.block_id().map(str::to_owned))
            .collect::<Vec<_>>()
    };
    let assert_has = |tool: &str, ids: &[String], expected: &str| {
        assert!(
            ids.iter().any(|id| id == expected),
            "{tool} omitted {expected}; got {ids:?}"
        );
    };

    let ids = render_ids("shell.run", sample_raw("shell.run"));
    assert_has("shell.run", &ids, "command");
    assert_has("shell.run", &ids, "process-meta");

    let ids = render_ids("shell.list", sample_raw("shell.list"));
    assert_has("shell.list", &ids, "processes");

    let ids = render_ids("memory.delete", sample_raw("memory.delete"));
    assert_has("memory.delete", &ids, "memory-delete");

    let ids = render_ids("memory.write", sample_raw("memory.write"));
    assert_has("memory.write", &ids, "memory-write");

    let tool = "chatgpt.cloud_shell";
    let ids = render_ids(tool, sample_raw(tool));
    assert!(!ids.iter().any(|id| id == "provider-calls"));
    assert_has(tool, &ids, "provider-call-operation-0");
    assert!(
        ids.iter().all(|id| !id.starts_with("result-")),
        "{tool} must not use generic nested result blocks: {ids:?}"
    );

    let ids = render_ids("tools.plugins_list", sample_raw("tools.plugins_list"));
    assert_has("tools.plugins_list", &ids, "discovery-plugins");
    let ids = render_ids("tools.plugins_tags", sample_raw("tools.plugins_tags"));
    assert_has("tools.plugins_tags", &ids, "discovery-tags");

    for tool in ["mcp.prompts.list", "mcp.prompts.get"] {
        let ids = render_ids(tool, sample_raw(tool));
        assert!(
            ids.iter().all(|id| !id.starts_with("result-")),
            "{tool} must not use generic nested prompt blocks: {ids:?}"
        );
    }
}

#[test]
fn every_bundled_execution_tool_has_a_human_initial_and_completed_title() {
    let manifest = agena_bundled_plugins::bundled_capability_manifest();
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
            let input = StructuredObject::try_from(sample_input(compact_name))
                .expect("sample structured input");
            let invocation = ToolInvocation::new(tool.canonical_name.clone(), input);
            let initial = initial_tool_title(&invocation);
            assert!(
                !initial.trim().is_empty(),
                "{} must have a visible initial title",
                tool.canonical_name
            );
            assert_ne!(
                initial, tool.canonical_name,
                "{} must not expose its registry identity as the human title",
                tool.canonical_name
            );

            let completed = completed_tool_title(&invocation, &sample_raw(compact_name));
            assert!(
                completed.starts_with(initial.as_str()),
                "{} completed title should retain its action",
                tool.canonical_name
            );
            assert!(
                completed != initial,
                "{} completed title should expose a terminal result fact: initial={initial}, completed={completed}",
                tool.canonical_name,
            );
        }
    }

    assert_eq!(checked, 131);
}

#[test]
fn representative_plugin_payloads_render_complete_readable_facts() {
    let fixtures = vec![
        (
            "mcp.resources.list",
            RawOutput::from_parts(
                Some(json!({
                    "server": "demo",
                    "resources": [{
                        "uri": "mcp://demo/readme",
                        "name": "README",
                        "description": "Project documentation",
                        "mime_type": "text/markdown"
                    }],
                    "next_cursor": "resources-cursor-2"
                })),
                "- README (mcp://demo/readme) [text/markdown]: Project documentation",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["demo", "README", "resources-cursor-2"],
            true,
        ),
        (
            "mcp.resources.templates.list",
            RawOutput::from_parts(
                Some(json!({
                    "server": "demo",
                    "resource_templates": [{
                        "uri_template": "users://{id}",
                        "name": "User profile",
                        "description": "A user profile",
                        "mime_type": "application/json"
                    }],
                    "next_cursor": "templates-cursor-2"
                })),
                "- User profile (users://{id}) [application/json]: A user profile",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["demo", "User profile", "templates-cursor-2"],
            true,
        ),
        (
            "mcp.resources.read",
            RawOutput::from_parts(
                Some(json!({
                    "server": "demo",
                    "uri": "mcp://demo/readme",
                    "contents": [{
                        "uri": "mcp://demo/readme",
                        "mime_type": "text/markdown",
                        "text": "# Hello from MCP"
                    }]
                })),
                "mcp://demo/readme [text/markdown]\n# Hello from MCP",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["demo", "# Hello from MCP"],
            false,
        ),
        (
            "mcp.prompts.list",
            RawOutput::from_parts(
                Some(json!({
                    "server": "demo",
                    "prompts": [{
                        "name": "summarize",
                        "description": "Summarize a document",
                        "arguments": [{"name": "document", "required": true}]
                    }],
                    "next_cursor": null
                })),
                "- summarize (document*): Summarize a document",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["demo", "summarize"],
            true,
        ),
        (
            "mcp.prompts.get",
            RawOutput::from_parts(
                Some(json!({
                    "server": "demo",
                    "prompt": "summarize",
                    "description": "Summarize a document",
                    "messages": [{
                        "role": "user",
                        "content": {"type": "text", "text": "Summarize this document"}
                    }]
                })),
                "user: Summarize this document",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["demo", "summarize", "Summarize this document"],
            false,
        ),
        (
            "mcp.tools.call",
            RawOutput::from_parts(
                Some(json!({
                    "server": "demo",
                    "tool": "search",
                    "content": [{"type": "text", "text": "Search result: 3 matching documents"}],
                    "structured_content": {"matches": 3},
                    "mcp_meta": {"request_id": "mcp-17"}
                })),
                "Search result: 3 matching documents",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["demo", "search", "3 matching documents"],
            false,
        ),
        (
            "mcp.tools.search",
            RawOutput::from_parts(
                Some(json!({
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
                "Found 1 matching MCP tool.",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["search", "demo", "abc123"],
            true,
        ),
        (
            "mcp.servers.status",
            RawOutput::from_parts(
                Some(json!({
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
                "MCP server status refreshed.",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["demo", "ready", "2026-08-22T10:00:00Z"],
            true,
        ),
        (
            "mcp.servers.reconnect",
            RawOutput::from_parts(
                Some(json!({
                    "server": "demo",
                    "reconnected": true,
                    "status": "connected",
                    "message": "Handshake completed",
                    "attempt": 2
                })),
                "Reconnected MCP server 'demo'.",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["demo", "connected", "Handshake completed"],
            false,
        ),
        (
            "settings.inspect",
            RawOutput::from_parts(
                Some(json!({
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
                "Inspected global, workspace, and effective settings values.",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["providers.openai", "global", "environment"],
            true,
        ),
        (
            "settings.list",
            RawOutput::from_parts(
                Some(json!({
                    "source": "effective",
                    "config_found": true,
                    "items": [{
                        "path": "providers.openai.model",
                        "value": "gpt-5",
                        "source": "global"
                    }],
                    "count": 1
                })),
                "- providers.openai.model = gpt-5",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["effective", "providers.openai.model", "global"],
            true,
        ),
        (
            "settings.get",
            RawOutput::from_parts(
                Some(json!({
                    "path": "providers.openai.model",
                    "value": "gpt-5",
                    "source": "global",
                    "layer": "global",
                    "config_path": "/workspace/.agena/agena.json"
                })),
                "providers.openai.model = gpt-5 (source: global)",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["providers.openai.model", "gpt-5", "agena.json"],
            false,
        ),
        (
            "settings.set",
            RawOutput::from_parts(
                Some(json!({
                    "path": "providers.openai.model",
                    "layer": "workspace",
                    "value": "gpt-5",
                    "changed": true,
                    "validated": true,
                    "config_path": "/workspace/.agena/agena.json"
                })),
                "Updated providers.openai.model in the workspace settings.",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["providers.openai.model", "workspace", "Validated"],
            false,
        ),
        (
            "settings.validate",
            RawOutput::from_parts(
                Some(json!({
                    "valid": true,
                    "warnings": [{"path": "providers.openai.model", "message": "Uses a preview model"}],
                    "files": ["/workspace/.agena/agena.json", "/workspace/.agena/agena.local.json"]
                })),
                "Settings validation completed with one warning.",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["preview model", "agena.local.json"],
            true,
        ),
        (
            "settings.delete",
            RawOutput::from_parts(
                Some(json!({
                    "path": "providers.openai.model",
                    "layer": "workspace",
                    "deleted": true,
                    "validated": true,
                    "config_path": "/workspace/.agena/agena.json"
                })),
                "Deleted providers.openai.model from the workspace settings.",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["providers.openai.model", "Deleted", "true"],
            false,
        ),
        (
            "settings.patch",
            RawOutput::from_parts(
                Some(json!({
                    "layer": "workspace",
                    "changed": true,
                    "validated": true,
                    "updated_paths": ["providers.openai.model", "providers.openai.timeout"],
                    "config_path": "/workspace/.agena/agena.json"
                })),
                "Patched workspace settings and validated the merged configuration.",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["workspace", "providers.openai.timeout", "Validated"],
            false,
        ),
        (
            "tools.list",
            RawOutput::from_parts(
                None,
                "Available tools: returned 2 of 2 starting at offset 0.\n- fs.read [filesystem] (agena.fs): Read a file\n- browser_snapshot [browser] (agena.web): Inspect a page",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["fs.read", "browser_snapshot", "filesystem"],
            false,
        ),
        (
            "tools.search",
            RawOutput::from_parts(
                None,
                "Matching tools for \"image\": returned 1 of 1 starting at offset 0.\n- chatgpt.cloud_image_generation [network]: Generate an image in OpenAI cloud",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["chatgpt.cloud_image_generation", "Generate an image"],
            false,
        ),
        (
            "tools.help",
            RawOutput::from_parts(
                None,
                "Tool: fs.read\nTags: filesystem, query\nUsage:\n- `file_path` (string, required)\nExamples:\n- {\"file_path\":\"README.md\"}\nHelp:\nRead a bounded file preview.",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["fs.read", "file_path", "README.md"],
            false,
        ),
        (
            "tools.tags",
            RawOutput::from_parts(
                None,
                "Available tool tags: returned 2 of 2 starting at offset 0.\n- filesystem: 14\n- discovery: 37",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["filesystem", "discovery", "37"],
            false,
        ),
        (
            "plugins.list",
            RawOutput::from_parts(
                None,
                "Available plugins: returned 1 of 1 starting at offset 0.\n- agena.web [network] (v0.1.0): Browser and web tools · tools: browser_open, browser_snapshot",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["agena.web", "browser_snapshot", "0.1.0"],
            false,
        ),
        (
            "plugins.search",
            RawOutput::from_parts(
                None,
                "Matching plugins for \"memory\": returned 1 of 1 starting at offset 0.\n- agena.memory [filesystem, discovery]: Durable workspace memory",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["agena.memory", "Durable workspace memory"],
            false,
        ),
        (
            "plugins.tags",
            RawOutput::from_parts(
                None,
                "Available plugin tags: returned 2 of 2 starting at offset 0.\n- filesystem: 4\n- interactive: 6",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["filesystem", "interactive", "6"],
            false,
        ),
        (
            "commands.list",
            RawOutput::from_parts(
                Some(json!({
                    "packages": [
                        {"name": "renderer-notes", "summary": "Rendering conventions", "source": "workspace", "content_hash": "skillhash1", "editable": true},
                        {"name": "review", "summary": "Review a change", "source": "builtin", "content_hash": "skillhash2", "editable": false}
                    ],
                    "diagnostics": [],
                    "total": 2,
                    "offset": 0,
                    "returned": 2
                })),
                "- renderer-notes: Rendering conventions\n- review: Review a change",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["renderer-notes", "Rendering conventions", "workspace"],
            true,
        ),
        (
            "commands.get",
            RawOutput::from_parts(
                Some(json!({
                    "name": "renderer-notes",
                    "summary": "Rendering conventions",
                    "body": "Keep human output concise.",
                    "aliases": ["rendering"],
                    "source_path": ".agena/skills/renderer-notes/SKILL.md",
                    "source": "workspace",
                    "content_hash": "skillhash1",
                    "document": "---\nname: renderer-notes\n---\nKeep human output concise.",
                    "revision": "rev-1",
                    "editable": true
                })),
                "Name: renderer-notes\nRevision: rev-1\nSummary: Rendering conventions\n\nBody:\nKeep human output concise.",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["renderer-notes", "Keep human output concise", "SKILL.md"],
            false,
        ),
        (
            "session.environment",
            RawOutput::from_parts(
                Some(json!({
                    "workspace_root": "/workspace",
                    "git_branch": "main",
                    "git_short_sha": "abc123",
                    "git_dirty": true,
                    "shell": "/bin/zsh",
                    "os": "macos",
                    "arch": "arm64"
                })),
                "Workspace: /workspace\nGit: main @ abc123\nShell: /bin/zsh\nOS: macos arm64",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["/workspace", "abc123", "arm64"],
            false,
        ),
        (
            "session.model",
            RawOutput::from_parts(
                Some(json!({
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
                "Model: openai/responses/gpt-5; thinking: high; verbosity: concise",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["responses", "gpt-5", "200000"],
            false,
        ),
        (
            "session.tokens",
            RawOutput::from_parts(
                Some(json!({
                    "session_id": 42,
                    "current_tokens": 12000,
                    "measured_prompt_tokens": 11000,
                    "projected_tokens": 15000,
                    "limit_tokens": 20000,
                    "remaining_tokens": 5000,
                    "usage_ratio": 0.6,
                    "reserved_tokens": 1000
                })),
                "Tokens: 12000 used; measured 11000; projected 15000; limit 20000; remaining 5000; reserved 1000.",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["12000", "5000", "0.6"],
            false,
        ),
        (
            "tasks.list",
            RawOutput::from_parts(
                Some(json!({
                    "tasks": [
                        {"task_id": "t-1", "parent_session_id": 42, "status": "completed", "description": "Inspect files", "model_id": "gpt-5"},
                        {"task_id": "t-2", "parent_session_id": 42, "status": "running", "description": "Run tests", "model_id": "gpt-5"}
                    ],
                    "timed_out": false
                })),
                "2 delegated tasks: t-1 completed, t-2 running",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["t-1", "t-2", "Inspect files"],
            true,
        ),
        (
            "tasks.output",
            RawOutput::from_parts(
                Some(json!({
                    "task": {"task_id": "t-1", "status": "completed"},
                    "chunks": [
                        {"role": "assistant", "text": "The renderer is ready."},
                        {"role": "tool", "text": "2 tests passed."}
                    ],
                    "next_cursor": 8,
                    "has_more": false
                })),
                "[assistant] The renderer is ready.\n[tool] 2 tests passed.",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["t-1", "tests passed", "Next Cursor"],
            true,
        ),
        (
            "code.search_ast",
            RawOutput::from_parts(
                Some(json!({
                    "language": "rust",
                    "pattern": "fn $NAME() { $$$ }",
                    "scanned_files": 4,
                    "matches": [
                        {"path": "src/lib.rs", "line": 7, "column": 1, "text": "fn render() {}"},
                        {"path": "src/main.rs", "line": 12, "column": 1, "text": "fn main() {}"}
                    ]
                })),
                "2 structural matches in 4 files\nsrc/lib.rs:7\nsrc/main.rs:12",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["src/lib.rs", "src/main.rs", "rust"],
            true,
        ),
        (
            "code.syntax_tree",
            RawOutput::from_parts(
                Some(json!({
                    "path": "src/lib.rs",
                    "language": "rust",
                    "root_kind": "source_file",
                    "has_error": false,
                    "tree": {"kind": "function_item", "name": "render", "children": ["identifier", "block"]}
                })),
                "Syntax tree · src/lib.rs\nroot source_file\nno parse errors",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["src/lib.rs", "function_item", "render"],
            false,
        ),
        (
            "shell.list",
            RawOutput::from_parts(
                Some(json!({
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
                "1 managed process.",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["proc-1", "cargo test", "running"],
            true,
        ),
        (
            "shell.logs",
            RawOutput::from_parts(
                Some(json!({
                    "action": "logs",
                    "process_id": "proc-1",
                    "events": [{"seq": 4, "stream": "stdout", "ts_ms": 20, "line": "test result: ok"}],
                    "last_seq": 4,
                    "has_more": false,
                    "dropped_lines": 0
                })),
                "test result: ok",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["proc-1", "test result: ok"],
            false,
        ),
        (
            "report.findings",
            RawOutput::from_parts(
                Some(json!({
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
                "- [high] src/lib.rs:7 — Example finding (confidence 0.90)\n  Example finding body",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["Review complete", "src/lib.rs", "high"],
            true,
        ),
        (
            "plan.get",
            RawOutput::from_parts(
                Some(json!({
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
                "# Renderer cleanup\n\nImprove tool presentation.\n\n## Steps\n\n1. **Implement renderer** — in progress\n   Use shared blocks",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["Renderer cleanup", "planning", "Add tests"],
            true,
        ),
        (
            "plan.review",
            RawOutput::from_parts(
                Some(json!({
                    "decision": "approve",
                    "plan": {
                        "title": "Renderer cleanup",
                        "phase": "active",
                        "steps": [{"title": "Implement renderer", "status": "in_progress"}]
                    }
                })),
                "Plan review decision: approve.\n\nRenderer cleanup is now active.",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["approve", "Renderer cleanup", "active"],
            true,
        ),
        (
            "chatgpt.cloud_image_generation",
            RawOutput::from_parts(
                Some(json!({
                    "provider": "chatgpt",
                    "tool": "image_generation",
                    "model": "gpt-image-1",
                    "path": "/workspace/generated.png",
                    "mime": "image/png",
                    "size_bytes": 8192,
                    "sha256": "imagehash",
                    "revised_prompt": "A watercolor map of a floating city"
                })),
                "Saved OpenAI cloud image artifact to '/workspace/generated.png'.",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["gpt-image-1", "/workspace/generated.png", "floating city"],
            false,
        ),
        (
            "browser_snapshot",
            RawOutput::from_parts(
                Some(json!({
                    "session_id": "session-1",
                    "snapshot": {
                        "title": "Agena docs",
                        "url": "https://example.test/docs",
                        "text": "Welcome to the docs",
                        "elements": [{"ref": "e1", "role": "link", "name": "API reference"}]
                    }
                })),
                "Title: Agena docs\nURL: https://example.test/docs\nInteractive elements: 1\n\nWelcome to the docs",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["session-1", "API reference", "https://example.test/docs"],
            true,
        ),
        (
            "browser_list",
            RawOutput::from_parts(
                Some(json!({
                    "browser_running": true,
                    "sessions": [{
                        "session_id": "session-1",
                        "title": "Agena docs",
                        "url": "https://example.test/docs",
                        "attached": true
                    }]
                })),
                "1 managed browser page target(s).",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["session-1", "Agena docs", "https://example.test/docs"],
            true,
        ),
        (
            "browser_open",
            RawOutput::from_parts(
                Some(json!({
                    "session_id": "session-1",
                    "snapshot": {"title": "Agena docs", "url": "https://example.test/docs", "elements": []},
                    "preflight_redirects": [],
                    "document_requests_intercepted": true
                })),
                "Opened https://example.test/docs in browser session session-1.",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["session-1", "Agena docs", "Document Requests Intercepted"],
            false,
        ),
        (
            "browser_click",
            RawOutput::from_parts(
                Some(json!({
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
                "Completed browser click in browser session session-1.",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["session-1", "https://example.test/docs/api", "Run"],
            true,
        ),
        (
            "browser_type",
            RawOutput::from_parts(
                Some(json!({
                    "session_id": "session-1",
                    "result": {"ok": true, "value": "agena", "method": "ref"},
                    "snapshot": {"title": "Search", "url": "https://example.test/search?q=agena", "elements": []}
                })),
                "Completed browser type in browser session session-1.",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["session-1", "agena", "https://example.test/search"],
            false,
        ),
        (
            "browser_wait",
            RawOutput::from_parts(
                Some(json!({
                    "session_id": "session-1",
                    "condition": "text:Ready",
                    "elapsed_ms": 250,
                    "snapshot": {"title": "Ready", "url": "https://example.test/ready", "elements": []}
                })),
                "Completed browser wait in browser session session-1.",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["text:Ready", "250", "https://example.test/ready"],
            false,
        ),
        (
            "browser_screenshot",
            RawOutput::from_parts(
                Some(json!({
                    "session_id": "session-1",
                    "path": "/workspace/.agena/artifacts/browser/screen.png",
                    "size_bytes": 8192
                })),
                "Saved browser screenshot to '/workspace/.agena/artifacts/browser/screen.png'.",
                vec![agena_domain::AttachmentItem {
                    kind: agena_domain::AttachmentKind::Image,
                    mime: "image/png".into(),
                    source: agena_domain::AttachmentSource::LocalPath {
                        path: "/workspace/.agena/artifacts/browser/screen.png".into(),
                    },
                    filename: Some("screen.png".into()),
                    title: Some("Browser screenshot session-1".into()),
                    size_bytes: Some(8192),
                    sha256: None,
                    width: None,
                    height: None,
                    duration_ms: None,
                    page_count: None,
                }],
                Vec::new(),
                false,
            ),
            vec!["session-1", "screen.png", "8192"],
            false,
        ),
        (
            "browser_download",
            RawOutput::from_parts(
                Some(json!({
                    "session_id": "session-1",
                    "url": "https://example.test/report.pdf",
                    "path": "/workspace/downloads/report.pdf",
                    "size_bytes": 4096,
                    "preflight_redirects": ["https://cdn.example.test/report.pdf"]
                })),
                "Saved browser download to '/workspace/downloads/report.pdf'.",
                vec![agena_domain::AttachmentItem {
                    kind: agena_domain::AttachmentKind::Pdf,
                    mime: "application/pdf".into(),
                    source: agena_domain::AttachmentSource::LocalPath {
                        path: "/workspace/downloads/report.pdf".into(),
                    },
                    filename: Some("report.pdf".into()),
                    title: Some("Browser download".into()),
                    size_bytes: Some(4096),
                    sha256: None,
                    width: None,
                    height: None,
                    duration_ms: None,
                    page_count: Some(1),
                }],
                Vec::new(),
                false,
            ),
            vec!["report.pdf", "cdn.example.test", "4096"],
            false,
        ),
        (
            "chatgpt.cloud_web_search",
            RawOutput::from_parts(
                Some(json!({
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
                "OpenAI found the latest Agena rendering guide.",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec![
                "OpenAI cloud",
                "Agena rendering guide",
                "example.test/guide",
            ],
            true,
        ),
        (
            "chatgpt.cloud_shell",
            RawOutput::from_parts(
                Some(json!({
                    "provider": "chatgpt",
                    "tool": "shell",
                    "model": "gpt-5",
                    "request_id": "req-shell-1",
                    "response_id": "resp-shell-1",
                    "pending_calls": [{"type": "shell_call", "id": "call-shell-1", "command": "pwd"}],
                    "sources": [],
                    "usage": {"input_tokens": 80, "output_tokens": 30},
                    "response_receipt": {"path": ".agena/receipts/resp-shell-1.json", "sha256": "shell-receipt"},
                    "continuation_required": true
                })),
                "OpenAI returned a hosted shell result for the requested command.",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["OpenAI cloud", "call-shell-1", "pwd"],
            false,
        ),
        (
            "gemini.cloud_google_search",
            RawOutput::from_parts(
                Some(json!({
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
                "Gemini returned grounded search context.",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["Google cloud", "gemini-2.5-pro", "Grounded result"],
            true,
        ),
        (
            "memory.search",
            RawOutput::from_parts(
                Some(json!({
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
                "Found 1 memory item(s) matching 'renderer'.\n- renderer-notes [project]: Rendering conventions",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec![
                "memory-1",
                "renderer-notes",
                ".agena/memory/renderer-notes.md",
            ],
            true,
        ),
        (
            "repo.status",
            RawOutput::from_parts(
                Some(json!({
                    "root": "/workspace",
                    "branch": "main",
                    "head": "abc123",
                    "dirty": true,
                    "changes": [{"path": "src/lib.rs", "kind": "modified", "additions": 4, "deletions": 1}]
                })),
                "Repository on branch main with one changed file.",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["/workspace", "abc123", "src/lib.rs"],
            true,
        ),
        (
            "memory.list",
            RawOutput::from_parts(
                Some(json!({
                    "limit": 50,
                    "memories": [{
                        "name": "renderer-notes",
                        "description": "Rendering conventions",
                        "memory_type": "project",
                        "path": ".agena/memory/renderer-notes.md",
                        "content_hash": "memoryhash"
                    }]
                })),
                "- renderer-notes [project]: Rendering conventions",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["renderer-notes", "50"],
            true,
        ),
        (
            "memory.get",
            RawOutput::from_parts(
                Some(json!({
                    "name": "renderer-notes",
                    "description": "Rendering conventions",
                    "memory_type": "project",
                    "body": "Keep human output concise and complete.",
                    "path": ".agena/memory/renderer-notes.md",
                    "content_hash": "memoryhash"
                })),
                "# Renderer notes\n\nKeep human output concise and complete.",
                Vec::new(),
                Vec::new(),
                false,
            ),
            vec!["renderer-notes", "Keep human output concise"],
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
        if matches!(tool, "browser_screenshot" | "browser_download") {
            assert!(
                blocks
                    .iter()
                    .any(|block| matches!(block, ViewBlock::Media { .. })),
                "{tool} should expose its returned artifact as media: {blocks:?}"
            );
        }
    }
}

#[test]
fn every_cloud_tool_keeps_its_location_visible_in_titles_and_result_views() {
    for tool in agena_tool::provider_tools::CLOUD_TOOLS {
        let raw = sample_raw(tool.name);
        for name in [
            tool.name.to_owned(),
            format!("agena.{}", tool.name),
            format!("agena_{}", tool.name.replacen('.', "_", 1)),
        ] {
            let invocation = ToolInvocation::new(
                name.clone(),
                StructuredObject::try_from(json!({"prompt":"fixture","model":"test-model"}))
                    .unwrap(),
            );
            let initial = initial_tool_title(&invocation);
            let completed = completed_tool_title(&invocation, &raw);
            assert!(initial.contains("cloud"), "{name}: {initial}");
            assert!(initial.contains(tool.provider_label));
            assert!(completed.starts_with(&initial));
            assert!(completed.contains("cloud"));
            assert_ne!(initial, completed);
            let blocks = BuiltinHumanRenderer::new(&name)
                .render_human(&render_context(), &raw)
                .unwrap();
            assert!(
                has_tool_specific_projection(tool.name, &blocks),
                "{name} lost operation details"
            );
            let metadata = blocks
                .iter()
                .find(|block| block.block_id() == Some("provider-meta"))
                .and_then(ViewBlock::text_value)
                .unwrap();
            assert!(metadata.contains(&format!("{} cloud", tool.provider_label)));
            assert!(!metadata.contains("Local project access"));
            let empty = BuiltinHumanRenderer::new(&name)
                .render_human(&render_context(), &RawOutput::default())
                .unwrap();
            assert!(
                empty
                    .iter()
                    .filter_map(ViewBlock::text_value)
                    .any(|text| text.contains("cloud"))
            );
        }
    }
    for name in ["shell.run", "fs.read", "web.search"] {
        let invocation = ToolInvocation::new(name, StructuredObject::default());
        assert!(!initial_tool_title(&invocation).contains("cloud"));
    }
}

#[test]
fn cloud_continuation_never_looks_like_missing_local_callbacks() {
    let raw = RawOutput {
        payload: Some(
            json!({"provider":"claude","tool":"advisor","pending_calls":[],"continuation_required":true}),
        ),
        ..Default::default()
    };
    let blocks = BuiltinHumanRenderer::new("claude.cloud_advisor")
        .render_human(&render_context(), &raw)
        .unwrap();
    let text = blocks
        .iter()
        .filter_map(ViewBlock::text_value)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Cloud continuation"));
    assert!(text.contains("No local command"));
    assert!(!text.contains("No pending calls could be decoded"));
}

#[test]
fn every_tool_has_compact_tables_and_preserves_its_raw_payload() {
    for plugin in agena_bundled_plugins::bundled_capability_manifest().plugins {
        for tool in plugin.tools.into_iter().filter(|tool| !tool.gateway) {
            let name = tool
                .canonical_name
                .strip_prefix("agena.")
                .unwrap_or(&tool.canonical_name);
            let raw = sample_raw(name);
            let original = serde_json::to_value(&raw).unwrap();
            let blocks = BuiltinHumanRenderer::new(name)
                .render_human(&render_context(), &raw)
                .unwrap();
            assert_eq!(
                serde_json::to_value(&raw).unwrap(),
                original,
                "{name} mutated its durable output"
            );
            let mut texts = std::collections::BTreeSet::new();
            for block in &blocks {
                if let Some(text) = block.text_value() {
                    assert!(
                        texts.insert(text.trim()),
                        "{name} repeats a human block: {text}"
                    );
                }
                if let ViewBlock::Table { columns, rows, .. } = block {
                    for (index, column) in columns.iter().enumerate() {
                        assert!(
                            !matches!(column.as_str(), "SHA-256" | "Hash" | "Revision"),
                            "{name}: technical column {column}"
                        );
                        assert!(
                            rows.iter()
                                .any(|row| row.get(index).is_some_and(
                                    |value| !value.is_null() && value.as_str() != Some("")
                                )),
                            "{name}: empty column {column}"
                        );
                    }
                    assert!(
                        rows.iter().all(|row| row.len() == columns.len()),
                        "{name}: malformed table"
                    );
                }
            }
        }
    }
}

#[test]
fn file_mutations_show_diffs_and_keep_checksums_in_raw_details() {
    for name in ["fs.write", "fs.replace", "notebook.edit_cell"] {
        let preview =
            agena_runtime_tools::file_diff_preview("src/main.rs", Some("old\n"), Some("new\n"));
        let raw = RawOutput {
            payload: Some(json!({
                "path": "src/main.rs", "kind": "updated", "replacements": 1,
                "diff": preview.diff, "additions": 1, "deletions": 1,
                "before_sha256": "private-before-checksum", "after_sha256": "private-after-checksum"
            })),
            ..RawOutput::default()
        };
        let blocks = BuiltinHumanRenderer::new(name)
            .render_human(&render_context(), &raw)
            .unwrap();
        assert!(blocks.iter().any(
            |block| matches!(block, ViewBlock::Diff { diff, .. } if diff.contains("-old\n+new\n"))
        ));
        let human = serde_json::to_string(&blocks).unwrap();
        assert!(!human.contains("private-before-checksum"));
        assert!(!human.contains("private-after-checksum"));
        assert_eq!(
            raw.payload.as_ref().unwrap()["before_sha256"],
            "private-before-checksum"
        );
    }
}

#[test]
fn cloud_execution_keeps_the_actual_output_and_shell_logs_do_not_claim_to_be_empty() {
    for name in [
        "chatgpt.cloud_code_interpreter",
        "claude.cloud_code_execution",
        "gemini.cloud_code_execution",
    ] {
        let blocks = BuiltinHumanRenderer::new(name)
            .render_human(&render_context(), &sample_raw(name))
            .unwrap();
        assert!(
            blocks
                .iter()
                .filter_map(ViewBlock::text_value)
                .any(|text| text.contains("passed")),
            "{name} hid execution output"
        );
    }
    let logs = BuiltinHumanRenderer::new("shell.logs")
        .render_human(&render_context(), &sample_raw("shell.logs"))
        .unwrap();
    assert!(
        !serde_json::to_string(&logs)
            .unwrap()
            .contains("No process events")
    );
    for name in ["shell.resize", "shell.signal", "shell.write"] {
        let blocks = BuiltinHumanRenderer::new(name)
            .render_human(&render_context(), &sample_raw(name))
            .unwrap();
        assert_eq!(
            blocks
                .iter()
                .filter_map(ViewBlock::text_value)
                .filter(|text| text.contains("PROMPT>"))
                .count(),
            1
        );
    }
}
