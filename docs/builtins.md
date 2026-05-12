# Agena in-tree plugin-entry inventory

- **Document version:** plugin-entry-only runtime
- **Language:** English
- **Mirror:** `docs/builtins.zh-CN.md`

This document records the extension surfaces that ship **in-tree** with the `agena` binary.

It focuses on compile-time surfaces that are part of the product itself:

- first-party plugins registered by the runtime
- compile-time model-visible plugin entries
- bundled workflow markdown compiled into the binary
- in-tree TUI slash commands
- in-tree provider implementations

It does **not** try to inventory runtime-generated surfaces whose names depend on user files or config, such as:

- plugins loaded from config at runtime (`cdylib`, `stdio`, `http`, `wasm`)
- entries generated from `.agena/skills`, `~/.agena/skills`, or `~/.claude/skills`
- entries generated from `.agena/commands` or `~/.agena/commands`
- MCP server tools/resources/prompts generated from configured MCP servers
- provider additions injected by plugins
- shell hooks configured by the user

## Source of truth

The main source files for this document are:

- `crates/agena/src/config/registry.rs` — registers first-party static plugins into the runtime
- `crates/agena/src/entry/catalog.rs` — projects compile-time model-visible entries from first-party plugin manifests
- `crates/agena/src/plugins/bundled/` — first-party plugin implementations
- `crates/agena/src/plugins/bundled/skills_fs.rs` — discovery plugin that registers dynamic entries from skills/commands markdown
- `crates/agena-skills/src/bundled/` — bundled workflow markdown compiled with `include_str!`
- `apps/agena-tui/src/commands.rs` — in-tree slash command list
- `crates/agena/src/provider/` — in-tree provider implementations

---

## 1. In-tree first-party plugins

These plugins are compiled into the binary and registered statically by `ResolvedConfig::build_plugin_host_with_previous_and_mcp()` in `crates/agena/src/config/registry.rs`.

### 1.1 Always-registered first-party plugins

| Plugin ID | Role | Model-visible surface |
|---|---|---|
| `agena.lsp` | LSP observability and navigation plugin | Compile-time entries |
| `agena.cron` | Scheduler plugin | Compile-time entries |
| `agena.fs` | Filesystem plugin | Compile-time entries |
| `agena.shell` | Shell and monitor plugin | Compile-time entries |
| `agena.web` | Web-fetch and web-search plugin | Compile-time entries |
| `agena.workflow` | Workflow/orchestration plugin | Compile-time entries |
| `agena.skills_fs` | Discovery plugin for markdown skills and commands | Dynamic entries at runtime |
| `agena-memory` | In-tree memory subsystem plugin | No model-visible entries |
| `agena-shell-hooks` | In-tree shell hook bridge | No model-visible entries |

### 1.2 Conditionally-registered first-party plugins

| Plugin ID | Condition | Notes |
|---|---|---|
| `agena.mcp` | Registered when an MCP manager is present | Model-visible entries are generated from the live MCP server snapshot, so this document does not inventory the runtime-generated names |

### 1.3 What no longer exists as a parallel runtime substrate

The runtime no longer treats these as separate first-class extension layers:

- the old built-in executor substrate
- the old skills substrate
- the old `skill_run` compatibility bridge
- a standalone skill registry for model-visible workflows
- a standalone command registry for runtime markdown commands

Model-visible capability now comes from plugin entries:

- compile-time entries declared by first-party plugin manifests
- dynamic entries registered through the plugin host entry registry

---

## 2. Compile-time model-visible entries

The compile-time model-visible catalog is assembled in `crates/agena/src/entry/catalog.rs` by merging entry declarations from first-party plugins.

### 2.1 Filesystem entries (`agena.fs`)

Defined in `crates/agena/src/plugins/bundled/fs.rs`.

| Entry | Behavior | Purpose |
|---|---|---|
| `read` | Read-only | Read a text file |
| `view_file` | Read-only | Attach a local file back into the conversation as multimodal input |
| `glob` | Read-only | Search files by glob pattern |
| `grep` | Read-only | Search file contents by regex |
| `apply_patch` | Mutating | Apply structured file patches |
| `notebook_edit` | Mutating | Edit a Jupyter notebook cell |

### 2.2 Shell entries (`agena.shell`)

Defined in `crates/agena/src/plugins/bundled/shell.rs`.

| Entry | Behavior | Purpose |
|---|---|---|
| `bash` | Mutating | Run a shell command in the workspace |
| `powershell` | Mutating | Run a PowerShell command |
| `monitor` | Mutating | Start, list, read, and stop long-running monitored processes |

### 2.3 Web entries (`agena.web`)

Defined in `crates/agena/src/plugins/bundled/web.rs`.

| Entry | Behavior | Purpose |
|---|---|---|
| `web_fetch` | Read-only | Fetch a URL and return markdown content |
| `web_search` | Read-only | Search the web using the configured backend |

### 2.4 Workflow and orchestration entries (`agena.workflow`)

Defined in `crates/agena/src/plugins/bundled/workflow.rs`.

| Entry | Behavior | Purpose |
|---|---|---|
| `init` | Read-only | Generate the bundled init workflow prompt |
| `review` | Read-only | Generate the bundled review workflow prompt |
| `security-review` | Read-only | Generate the bundled security-review workflow prompt |
| `task` | Task | Create or resume a delegated subtask session |
| `tool_search` | Read-only | Search the tool catalog and optionally load deferred tools |
| `todo_write` | Read-only | Replace the session todo list |
| `ask_user` | Read-only | Ask short questions and wait for answers |
| `enter_plan_mode` | Read-only | Enter plan mode |
| `exit_plan_mode` | Read-only | Exit plan mode |
| `enter_worktree` | Mutating | Create or attach to a worktree |
| `exit_worktree` | Mutating | Leave the current worktree |

### 2.5 LSP entries (`agena.lsp`)

Defined in `crates/agena/src/plugins/bundled/lsp.rs`.

| Entry | Behavior | Purpose |
|---|---|---|
| `lsp_servers` | Read-only | List configured LSP servers |
| `lsp_definition` | Read-only | Jump to symbol definition |
| `lsp_references` | Read-only | Find references to a symbol |
| `lsp_hover` | Read-only | Read hover/type information |
| `lsp_diagnostics` | Read-only | Read diagnostics for a file |

### 2.6 Scheduler entries (`agena.cron`)

Defined in `crates/agena/src/plugins/bundled/cron.rs`.

| Entry | Behavior | Purpose |
|---|---|---|
| `cron_create` | Read-only | Create a recurring scheduled job |
| `cron_list` | Read-only | List scheduled jobs |
| `cron_delete` | Read-only | Delete a scheduled job |
| `schedule_wakeup` | Read-only | Schedule a one-shot wake-up prompt |

### 2.7 Complete compile-time entry list

For quick reference, the compile-time model-visible entry names are:

- `read`
- `view_file`
- `glob`
- `grep`
- `apply_patch`
- `notebook_edit`
- `bash`
- `powershell`
- `monitor`
- `web_fetch`
- `web_search`
- `init`
- `review`
- `security-review`
- `task`
- `tool_search`
- `todo_write`
- `ask_user`
- `enter_plan_mode`
- `exit_plan_mode`
- `enter_worktree`
- `exit_worktree`
- `lsp_servers`
- `lsp_definition`
- `lsp_references`
- `lsp_hover`
- `lsp_diagnostics`
- `cron_create`
- `cron_list`
- `cron_delete`
- `schedule_wakeup`

That is **31 compile-time model-visible entries**.

---

## 3. Runtime-generated entry surfaces

### 3.1 `agena.skills_fs`

Defined in `crates/agena/src/plugins/bundled/skills_fs.rs`.

`agena.skills_fs` is a first-party discovery plugin. It scans these roots:

- `.agena/skills`
- `~/.agena/skills`
- `~/.claude/skills`
- `.agena/commands`
- `~/.agena/commands`

It also loads bundled markdown content from `crates/agena-skills/src/bundled/`.

Everything it discovers is registered through `host/entry.register` as **dynamic plugin entries**. That means markdown skills and markdown commands are no longer a separate runtime substrate; they become ordinary plugin entries in the shared registry.

These names are runtime-dependent, so they are intentionally excluded from the fixed compile-time list above.

### 3.2 `agena.mcp`

Defined in `crates/agena/src/plugins/bundled/mcp.rs`.

`agena.mcp` is the first-party adapter plugin for configured MCP servers. It projects server state into model-visible entries such as:

- `mcp:<server>:tool:<tool>`
- `mcp:<server>:resources:list`
- `mcp:<server>:resources:read`
- `mcp:<server>:prompts:list`
- `mcp:<server>:prompts:get`

Those names come from the live MCP snapshot, so they are also runtime-generated rather than compile-time fixed.

---

## 4. Bundled workflow markdown compiled into the binary

Bundled workflow markdown lives in `crates/agena-skills/src/bundled/` and is compiled with `include_str!` in `crates/agena-skills/src/bundled/mod.rs`.

This content is **source material used by first-party plugins**, not a separate runtime registry.

### 4.1 `init`

Source: `crates/agena-skills/src/bundled/init.md`

- **Name:** `init`
- **Alias:** `bootstrap`
- **Description:** initialise an `AGENTS.md` or `CLAUDE.md` describing the codebase
- **Purpose:** bootstrap a project instruction / memory file for the repository

### 4.2 `review`

Source: `crates/agena-skills/src/bundled/review.md`

- **Name:** `review`
- **Description:** review the current branch as a senior code reviewer
- **Allowed tools:** `read`, `glob`, `grep`, `view_file`
- **Purpose:** perform a structured review of the current branch

### 4.3 `security-review`

Source: `crates/agena-skills/src/bundled/security_review.md`

- **Name:** `security-review`
- **Description:** audit the current branch for security regressions
- **Allowed tools:** `read`, `glob`, `grep`
- **Purpose:** perform a security-focused review of the current branch

The canonical compile-time runtime entries for these workflows come from `agena.workflow` (`init`, `review`, `security-review`).

---

## 5. In-tree TUI slash commands

The in-tree slash commands are defined in `apps/agena-tui/src/commands.rs`.

These are compile-time commands implemented by the TUI itself. Local UI commands stay local, while runtime workflow commands such as `/review` dispatch through the runtime entry registry.

For the current command set, treat `apps/agena-tui/src/commands.rs` as the source of truth.

### 5.1 Session and navigation commands

| Command | Aliases | Summary |
|---|---|---|
| `/help` | `/?` | Show help |
| `/commands` | `/palette` | Open the command palette |
| `/new` | `/clear` | Create a new session |
| `/sessions` |  | Focus sessions or switch view with all, roots, or subtree |
| `/resume` | `/switch`, `/recent` | Open a global session switcher to resume recent work |
| `/lineage` | `/branch-history`, `/branches` | Open a branch-history picker for the current or selected session |
| `/rewind` | `/backtrack` | Open a message picker to rewind the current session |
| `/search` |  | Search sessions or open the session search dialog |
| `/find` |  | Search transcript or open the transcript search dialog |
| `/rename` | `/title` | Rename the current or selected session |
| `/timeline` | `/events` | Open a searchable event timeline for the current or selected session |
| `/continue` | `/resume-run` | Continue the current blocked or pending session |
| `/fork` | `/branch` | Create a child session from the current session |
| `/children` | `/child` | Open the child-session picker |
| `/parent` |  | Jump to the parent session |
| `/queue` | `/q` | Inspect or manage the pending message queue |
| `/btw` | `/aside`, `/side` | Ask a side-question in a child session without disturbing the current turn |

### 5.2 Runtime inspectors and operational views

| Command | Aliases | Summary |
|---|---|---|
| `/plugins` | `/plugin` | Open a searchable plugin inspector with runtime status and recent logs |
| `/mcp` |  | Open a searchable MCP inspector |
| `/lsp` |  | Open a searchable LSP inspector |
| `/skills` | `/skill` | Open a searchable skills inspector |
| `/runtime` | `/operator` | Open a searchable runtime summary inspector |
| `/cost` | `/usage` | Inspect session token and cost usage |
| `/permissions` | `/perm` | Inspect persisted permission rules |
| `/config` |  | Inspect resolved config and runtime configuration |
| `/worktree` | `/wt` | Inspect active and managed worktrees |
| `/git` | `/repo` | Inspect git branch, diff, and worktree status |
| `/diagnostics` | `/feedback` | Show a sanitized diagnostics summary for feedback |
| `/status` |  | Show the current runtime override summary |
| `/memory` | `/mem` | Browse, edit, or remove saved memory files |

### 5.3 Git and review workflow commands

| Command | Aliases | Summary |
|---|---|---|
| `/review` |  | Dispatch the runtime `review` entry in the current session |
| `/commit` |  | Create a commit from staged changes |
| `/pr` |  | Create a pull request with minimal gh-backed options |

### 5.4 Provider and model selection commands

| Command | Aliases | Summary |
|---|---|---|
| `/providers` |  | List configured providers |
| `/provider` |  | Select a provider or clear the current provider/model override |
| `/models` |  | List models for a provider |
| `/model` |  | Select a model or clear the current model override |
| `/temperature` | `/temp` | Set or clear the temperature override |
| `/max-output` | `/max-tokens` | Set or clear the max output token override |
| `/system` |  | Set or clear the system prompt override |

### 5.5 Input, approval, and editor commands

| Command | Aliases | Summary |
|---|---|---|
| `/user-input` | `/reply` | Reply to the first pending user-input request |
| `/allow` |  | Allow the first pending permission request once |
| `/allow-always` |  | Always allow the first pending permission request |
| `/deny` |  | Deny the first pending permission request once |
| `/deny-always` |  | Always deny the first pending permission request |
| `/attach` | `/file` | Open the file attach overlay |
| `/editor` | `/edit` | Open the external editor for the composer |
| `/image` | `/paste-image` | Attach an image from the clipboard |
| `/copy` | `/yank` | Copy the loaded transcript |
| `/copy-visible` |  | Copy the visible transcript viewport |
| `/export` | `/save` | Export the loaded transcript as markdown and open it in the editor |
| `/pager` | `/view`, `/less` | Open the loaded transcript in a terminal pager |

---

## 6. In-tree provider implementations

Provider implementations are compiled into `crates/agena/src/provider/` and assembled by the provider registry in `crates/agena/src/config/registry.rs` and `crates/agena/src/provider/registry.rs`.

These are not model tools, but they are in-tree extension surfaces because they define the model backends the runtime can instantiate.

### 6.1 In-tree provider backends

| Provider implementation | Source file |
|---|---|
| Anthropic | `crates/agena/src/provider/anthropic.rs` |
| OpenAI | `crates/agena/src/provider/openai.rs` |
| OpenAI-compatible | `crates/agena/src/provider/openai_compatible.rs` |
| Amazon Bedrock | `crates/agena/src/provider/amazon_bedrock.rs` |
| Gemini | `crates/agena/src/provider/gemini.rs` |
| Google Vertex | `crates/agena/src/provider/google_vertex.rs` |
| Ollama | `crates/agena/src/provider/ollama.rs` |
| GitHub Copilot | `crates/agena/src/provider/copilot.rs` |
| Codex | `crates/agena/src/provider/codex.rs` |
| GitLab | `crates/agena/src/provider/gitlab.rs` |
| Opencode | `crates/agena/src/provider/opencode.rs` |
| Cloudflare AI Gateway | `crates/agena/src/provider/cloudflare_ai_gateway.rs` |

### 6.2 Provider registry vs runtime-added providers

This document only covers the in-tree provider implementations above.

It does **not** cover:

- provider removals/additions injected by plugins through `provider_list`
- remote or custom providers added at runtime

---

## 7. Explicit exclusions

To keep this document focused on compile-time in-tree surfaces, the following are intentionally excluded.

### 7.1 Disk-discovered markdown skills and commands

The markdown discovery system feeds `agena.skills_fs`, which registers those files as dynamic plugin entries through the entry registry.

They are runtime content, not fixed compile-time entry names.

### 7.2 MCP-generated entries

`agena.mcp` is an in-tree first-party plugin implementation, but its exposed entry names depend on configured MCP servers at runtime.

### 7.3 Configured shell hooks

`agena-shell-hooks` is in-tree, but the hook commands it executes come from user config. They are runtime configuration, not fixed compile-time surfaces.

---

## 8. Maintenance checklist

When updating this document, verify these places:

1. `crates/agena/src/config/registry.rs` for the statically registered first-party plugins
2. `crates/agena/src/entry/catalog.rs` for the compile-time entry catalog projection
3. `crates/agena/src/plugins/bundled/*.rs` for model-visible entry names
4. `crates/agena/src/plugins/bundled/skills_fs.rs` for dynamic discovery behavior
5. `crates/agena-skills/src/bundled/` for bundled workflow markdown
6. `apps/agena-tui/src/commands.rs` for in-tree slash commands
7. `apps/agena-tui/locales/en-US/main.ftl` for command summaries
8. `crates/agena/src/provider/mod.rs` and `crates/agena/src/provider/` for in-tree provider implementations

If a new first-party plugin is added, decide whether it belongs in:

- compile-time model-visible entries
- runtime-generated entry surfaces
- bundled workflow content
- in-tree slash commands
- in-tree providers

and update this document accordingly.
