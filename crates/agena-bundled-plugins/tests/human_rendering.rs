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
        "run_count": 2,
        "last_run_status": "completed"
    });
    let task = json!({
        "task_id": "task-1",
        "description": "Example delegated task",
        "status": "completed",
        "access": "read",
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
        "fs.view_image" => json!({
            "path": "assets/chart.png",
            "detail": "high",
            "mime": "image/png",
            "size_bytes": 4096,
            "sha256": "abc123"
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
            "run_count": 2,
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
        "skills.list" => {
            json!({"tools": [{"name": "review", "kind": "skill", "summary": "Review changes", "source": "workspace", "editable": true}], "returned": 1, "total": 1, "offset": 0})
        }
        "skills.get" => {
            json!({"name": "review", "kind": "skill", "source": "workspace", "body": "Review changes.", "content_hash": "hash-1", "editable": true})
        }
        "skills.create" | "skills.update" | "skills.delete" => {
            json!({"operation": if tool.ends_with("delete") { "deleted" } else { "updated" }, "name": "review", "path": "skills/review.md", "catalog_generation": 3, "catalog_changed": true, "editable": true})
        }
        "skills.read_resource" => {
            json!({"name": "review", "path": "references/checklist.md", "source": "workspace", "bytes": 42, "content_hash": "hash-2", "body": "Checklist"})
        }
        "skills.refresh" => {
            json!({"changed": true, "generation": 3, "tools": [{"name": "review"}]})
        }
        "snapshot.enter" => {
            json!({"path": "/tmp/snapshot", "branch": "snapshot/main", "backend": "git", "note": "before release"})
        }
        "snapshot.exit" => json!({"action": "restore", "path": "/tmp/snapshot"}),
        "snapshot.status" => {
            json!({"snapshots": [{"session_id": 42, "path": "/tmp/snapshot", "branch": "snapshot/main", "created_here": true}]})
        }
        "tasks.run" => json!({
            "task_id": "task-1", "session_id": 42, "parent_session_id": 0, "access": "read", "status": "completed",
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
            || tool.starts_with("gemini.")
            || tool.starts_with("openai.") =>
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
        "fs.write" | "fs.replace" | "fs.stat" | "fs.view_image" => {
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
        "skills.create" | "skills.update" | "skills.delete" | "skills.get" => {
            json!({"name": "review"})
        }
        "skills.read_resource" => json!({"name": "review", "path": "references/checklist.md"}),
        value if value.starts_with("skills.") => json!({}),
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
    let operation = tool.rsplit('.').next().unwrap_or_default();
    match operation {
        "web_search"
        | "web_search_preview"
        | "google_search"
        | "google_maps"
        | "retrieval"
        | "file_search"
        | "tool_search"
        | "tool_search_bm25"
        | "tool_search_regex"
        | "tool_search_tool_bm25"
        | "tool_search_tool_regex" => {
            json!({"query": "release policy"})
        }
        "web_fetch" | "url_context" => json!({"url": "https://example.test"}),
        "code_interpreter" | "code_execution" => json!({"command": "cargo test"}),
        "local_shell" | "shell" | "bash" => json!({"command": "cargo test"}),
        "mcp" | "mcp_server" | "mcp_toolset" => json!({"server_label": "docs"}),
        "memory" => json!({"operation": "save", "name": "release-notes"}),
        "text_editor" | "str_replace_based_edit_tool" => {
            json!({"path": "src/lib.rs", "operation": "replace"})
        }
        "apply_patch" => {
            json!({"patch": "*** Begin Patch\n*** Update File: src/lib.rs\n*** End Patch"})
        }
        "image_generation" | "image_edit" => json!({"prompt": "A polished release diagram"}),
        "computer" | "computer_use_preview" | "computer_use" => {
            json!({"url": "https://example.test/docs"})
        }
        "function" | "custom" | "namespace" => json!({"name": "search"}),
        "programmatic_tool_calling" => json!({"prompt": "Find the test tool"}),
        "advisor" => json!({"prompt": "Review this change"}),
        _ => json!({}),
    }
}

fn provider_sample_payload(tool: &str) -> Value {
    let provider = tool.split('.').next().unwrap_or("provider");
    let operation = tool.rsplit('.').next().unwrap_or("operation");
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
        "tool_search" | "tool_search_bm25" | "tool_search_regex" => {
            object.insert("query".into(), json!("find a search tool"));
            object.insert(
                "results".into(),
                json!([
                    {"name": "web.search", "description": "Search web", "server": "builtin"}
                ]),
            );
        }
        "web_search" | "web_search_preview" | "google_search" => {
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
        "retrieval" => {
            object.insert("query".into(), json!("release policy"));
            object.insert(
                "retrieved".into(),
                json!([
                    {"title": "Policy", "url": "https://example.test/policy", "snippet": "..."}
                ]),
            );
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
        "computer" | "computer_use_preview" | "computer_use" => {
            object.insert("action".into(), json!({"type": "click", "x": 20, "y": 30}));
            object.insert("page_title".into(), json!("Agena docs"));
            object.insert("url".into(), json!("https://example.test/docs"));
        }
        "local_shell" | "shell" | "bash" => {
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
        "mcp" | "mcp_server" | "mcp_toolset" => {
            object.insert("server_label".into(), json!("docs"));
            object.insert("server_url".into(), json!("https://mcp.test"));
            object.insert("connected".into(), json!(true));
            object.insert("status".into(), json!("ready"));
            object.insert("tool_count".into(), json!(3));
        }
        "memory" => {
            object.insert("operation".into(), json!("save"));
            object.insert("saved".into(), json!(true));
            object.insert("status".into(), json!("completed"));
        }
        "text_editor" => {
            object.insert("operation".into(), json!("str_replace"));
            object.insert("path".into(), json!("src/lib.rs"));
            object.insert("changed".into(), json!(true));
            object.insert("replacements".into(), json!(1));
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
        "apply_patch" => {
            object.insert(
                "pending_calls".into(),
                json!([{
                    "type": "apply_patch_call", "id": "call-1", "action": {"type": "update_file"}
                }]),
            );
            object.insert("continuation_required".into(), json!(true));
        }
        "function" | "custom" | "namespace" | "programmatic_tool_calling" => {
            object.insert("pending_calls".into(), json!([{
                "type": "function_call", "id": "call-1", "action": {"type": "invoke", "name": "search"}
            }]));
            object.insert("continuation_required".into(), json!(true));
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
    RawOutput {
        text: sample_text(tool),
        payload: (tool != "memory.delete").then(|| sample_payload(tool)),
        ..RawOutput::default()
    }
}

fn has_tool_specific_projection(tool: &str, blocks: &[ViewBlock]) -> bool {
    let ids = blocks.iter().filter_map(ViewBlock::block_id);
    if tool.starts_with("chatgpt.")
        || tool.starts_with("claude.")
        || tool.starts_with("gemini.")
        || tool.starts_with("openai.")
    {
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

    assert_eq!(checked, 137);
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

    assert_eq!(checked, 137);
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

    assert_eq!(checked, 137);
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

    for tool in ["chatgpt.apply_patch", "chatgpt.function", "chatgpt.shell"] {
        let ids = render_ids(tool, sample_raw(tool));
        assert_has(tool, &ids, "provider-calls");
        assert_has(tool, &ids, "provider-call-operation-0");
        assert!(
            ids.iter().all(|id| !id.starts_with("result-")),
            "{tool} must not use generic nested result blocks: {ids:?}"
        );
    }

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
                attachments: vec![agena_domain::AttachmentItem {
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
                attachments: vec![agena_domain::AttachmentItem {
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
