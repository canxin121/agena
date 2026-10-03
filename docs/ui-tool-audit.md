# UI and bundled tool audit — 2026-10-04

This audit covers the Web application, the terminal presentation and pointer
routing, and the complete source-level bundled plugin catalog: 22 plugins,
135 tool definitions (131 execution tools and four discovery gateways).

## Shared interface behavior

| Surface | Implementation and verification |
| --- | --- |
| Web dialogs and forms | `DialogHeader` owns the left close control, title, description, wrapping and touch sizing. `Dialog` and `FormDialog` share this header, compact spacing and semantic colors. Headers remain visible while content scrolls. Existing mobile keyboard and safe-area handling is retained. |
| Web plan viewer | One scrolling Markdown body; left controls; session-scoped, abortable requests; repeated refresh/toggle coalescing; stale reads and mutation failures cannot update a different session or reopened dialog. Existing content survives refresh failure. |
| Web composer, help, MCP and stash diff | Composer status uses flex rows instead of overlapping absolute corners. Plan progress opens the viewer. Help uses the shared scroll owner; the diff editor has a bounded viewport. MCP refresh sits beside its summary on the left. |
| TUI shared frames and pickers | Frame-local targets are registered from actual rendered rectangles. The topmost surface clears underlying targets. Shared close controls, shortcut buttons, list rows, search inputs and preview scrolling reuse keyboard reducers. |
| TUI settings, providers and permissions | Clicks focus the correct pane before selecting or activating rows. Settings label/value rows omit repeated inspector descriptions. Permission navigation handles nonselectable headings explicitly. Permission prompts reserve space for choices and scroll long request details independently. |
| TUI plan, activities and usage | Plan rendering scrolls wrapped display rows and clamps after resize. Plan requests retain the originating session and coalesce concurrent controls. Activity rows and usage rows use their displayed offsets; controls are clickable. |
| TUI plugin workbench | Plugin lists, configuration sections, fields and nested objects follow selection. Table widths are allocated across every column. Visible rows and all detail tabs have matching pointer targets; the config page retains its tabs. Toolbar actions reuse existing validation, confirmation and save paths. |
| TUI composer and suggestions | Model, thinking, speed, usage, plan and activity targets open their existing surfaces. File/command suggestions use the shared picker pointer path. Composer clicks place the cursor at Unicode display cells while dragging preserves selection. Transcript clicks focus the transcript. |

Pointer targets are rebuilt during rendering, without backend calls or timers.
Key input, pointer actions and terminal resize invalidate old targets. Wheel
events on modal surfaces cannot scroll the underlying chat. Permission body
scrolling leaves the selected decision unchanged. Displayed list targets are
computed after Ratatui adjusts its list offset, including multiline rows.

The TUI test constructor now allocates independent temporary draft and history
files. Its normal shutdown save also stays inside that temporary directory.

## Bundled tools

Every identity below runs through its actual `Plugin::tool_invoke` entrypoint
with an array, a boolean and an unknown-field object. All 405 calls must return
`InvalidParams` before initialization, host callbacks or service execution.
The test uses an isolated workspace and no configured external services.

| Plugin | Tools | Additional exercised behavior |
| --- | ---: | --- |
| `agena.chatgpt` | 11 | Hosted service routing, adapter gates, media handles, error/result projection |
| `agena.claude` | 9 | Hosted service routing, pause/resume, callback quarantine, media handles |
| `agena.gemini` | 12 | Hosted service routing, adapter gates, media and result projection |
| `agena.code` | 2 | Structured search contracts and human rendering |
| `agena.commands` | 6 | Workspace package discovery, install/read/remove, refresh generations, read-only catalog entries |
| `agena.cron` | 7 | Input contracts, execution permission boundaries and human rendering |
| `agena.fs` | 8 | Exact Unicode/CRLF edits, revision checks, ambiguous patch rejection, multi-file preflight, move/inverse operations, bounded reads, symlink identity |
| `agena.interaction` | 2 | Interaction contracts, effect permissions and human rendering |
| `agena.lsp` | 5 | Navigation/observability contracts and human rendering |
| `agena.mcp` | 9 | Bridge/discovery contracts, host lifecycle and failure boundaries |
| `agena.memory` | 5 | Revision-safe writes, index failure handling, exact names, YAML scalar quoting |
| `agena.monitor` | 2 | Stream contracts, ownership and human rendering |
| `agena.notebook` | 1 | Cell conversion, execution invalidation, metadata preservation, unique/duplicate cell IDs |
| `agena.plan` | 6 | Edit/phase validation, review cancellation/timeouts, autorun state, display output |
| `agena.report` | 1 | Invalid line range rejection and empty-result presentation |
| `agena.session` | 5 | Session/environment facts, nongit workspace semantics and human rendering |
| `agena.settings` | 7 | Explicit scopes, field diagnostics, secret redaction, commit facts after reload failure |
| `agena.shell` | 7 | Process/terminal lifecycle, cancellation, permissions, bounded output and human rendering |
| `agena.snapshot` | 3 | Snapshot contracts, effect permissions and human rendering |
| `agena.tasks` | 7 | Concurrent admission, cancellation, rollback after persistence failure, completion notifications |
| `agena.tools` | 7 | Discovery/help, exact tool identity, collision rejection and gateway/execution separation |
| `agena.web` | 13 | Partial search failures, browser ownership, response timeouts, UTF-8 log budgets, download completion and terminal rendering |

The additional column describes automated fixtures and contract checks, not
end-to-end testing against every live service. Existing human-rendering tests
exercise registered tool identities and file-edit diff output. Provider tests
use local fake services; credentials are not required. Dynamically installed
third-party plugins and arbitrary external MCP servers are outside this catalog.

### Fixed parameter defects

Serde permits positional sequences for named structs, so object-shaped tool
contracts could accept `[]` and execute defaults. The shared SDK parser now
checks JSON shapes before deserialization, including nested records, local
schema references and unions. Genuine arrays and unrestricted JSON fields stay
valid. Two command tools had a separate parser and now use the same SDK path.

`cron.list` also accepted unknown object fields. Its input now rejects them;
the generated capability identity snapshot and tool reference were regenerated
from the real manifests.

## Validation

The affected plugin/host/SDK/runtime suites pass 796 tests. The final TUI suites
pass 551 tests, and Web passes 577 tests plus its production build (including
import-boundary checks, settings translations and Vue type checking).
The CLI/server binary passes another 101 tests; the runtime capability gate
also verifies both AWS and reqwest HTTPS client construction.

```sh
cargo test -p agena-plugin-sdk -p agena-bundled-plugins \
  -p agena-plugin-host -p agena-runtime-contracts -p agena-runtime-tools \
  --tests --locked --no-fail-fast
cargo test -p agena-tui-components -p agena-tui -p agena-tui-app \
  -p agena-tui-plugin-workbench --tests --locked --no-fail-fast
cargo test -p agena --bin agena --locked
cargo clippy -p agena-plugin-sdk -p agena-bundled-plugins \
  -p agena-runtime-contracts -p agena-tui-components -p agena-tui \
  -p agena-tui-app -p agena-tui-plugin-workbench --all-targets --locked -- -D warnings
python3 scripts/ci/verify-runtime-capabilities.py
cd packages/agena-web
bun test
bun run build
```

TUI fixtures verify rendered buffers and the actions at their actual cells,
including short terminals, long lists, wrapped CJK plans, Unicode cursor
placement, modal precedence, refresh coalescing and stale-target invalidation.
Web lifecycle tests verify delayed responses, abort/disposal and mutation
ordering. The in-app browser was unavailable in this environment, so this audit
does not claim interactive browser screenshots or device-specific visual QA.
