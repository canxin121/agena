# Final Acceptance Checklist (R6)

Single reference for final acceptance once the migration chain merges. Produced by the
R6 FINAL-ACCEPTANCE pre-audit (read-only, worktree `db-design-audit`, HEAD 07ba6828 = merged R6-T5).

Status legend:
- **DONE** — verified in this tree at pre-audit time.
- **IN-FLIGHT** — partial work present; completion expected from the in-flight migration.
- **BLOCKED-on-T6** / **BLOCKED-on-T7** — cannot complete until `agena-runtime-session`
  v1-bridge removal (T6) / v1 contracts deletion (T7) land (parallel worktree `db-w3-t6`).
- **PENDING** — run after the chain is green.

---

## 1. Baseline build

| # | Command | Status | Notes |
|---|---|---|---|
| 1.1 | `cargo check --workspace --all-targets` | **DONE** | 2026-08-10 — first tree-wide green after T9-B merge. |
| 1.2 | `cargo test --workspace` | **DONE** | 2026-08-10, exit 0 (full tree, post T9-B). Key libs: contracts 22, session-core 15, session 106, provider 83, adapters 61, application 17, cli 9. |
| 1.3 | `cargo clippy --workspace --all-targets -- -D warnings` | **DONE** | 2026-08-10, exit 0. |
| 1.4 | `cargo fmt --all --check` | **DONE** | 2026-08-10, exit 0. |
| 1.5 | `git diff --check` | **DONE** | 2026-08-10, exit 0. |
| 1.6 | `cargo bench -p agena-storage-sqlite --bench v2_store` | **DONE** | 2026-08-10 — **improved** vs P7 baseline: read/warm_facade 161.6µs (was 221.7µs), usage/indexed 561.0µs (was 845.6µs), streaming 12.2ms (was 21.1ms). No regression. |

### 1a. Baseline failure detail (captured 2026-08-10)

`cargo check --workspace --all-targets` log (`/tmp/baseline_check.log`):

- Crates that compiled green: `agena-runtime-contracts`, `agena-runtime-tools`,
  `agena-runtime-config`, `agena-runtime-session-core`, `agena-runtime-notifications`,
  `agena-runtime-plugins`, `agena-runtime-provider`, `agena-bundled-plugins`,
  `agena-runtime-provider-adapters`.
- Failing: `agena-runtime-session` (lib + lib test), 9 errors:
  - `session/manager/compact.rs:462` — `.map(project_completion_input)` over v1 `Message` iter (E0631 + E0599).
  - `session/manager/mod.rs:60` — `.map(project_completion_input)` over v1 `Message` iter (E0631 + E0599).
  - `session/prompt_window.rs:310` — `project_session_parts(message)` passes `&Message`, needs `&[Part]` (E0308).
  - `session/prompt_window.rs:429` — same (E0308).
  - `session/prompt_window.rs:875` — `.map(project_session_text_lossy)` over v1 `Message` iter (E0631 + E0599×2).
  - `session/prompt_window.rs:924` — `project_session_text_lossy(message)` passes `&Message`, needs `&[Part]` (E0308).
- **Caveat**: crates that depend on `agena-runtime-session` (agena-api, agena-api-server,
  agena-application, agena-client, agena-tui-*, agena-web, agena-cli, apps/agena, …) were
  **skipped** (not compiled) because their dependency failed. They are verified only after T6
  fixes session. The baseline proves every crate that *was* compiled is green; it does not
  prove downstream crates compile.

---

## 2. SQL / schema seals

| # | Item | Command / evidence | Status | Notes |
|---|---|---|---|---|
| 2.1 | v7 schema seal | `crates/agena-storage-sqlite/src/schema_lifecycle.rs:31` — `CURRENT_SCHEMA_VERSION: i64 = 7`; `schema.rs` rejects any version ≠ 7 without mutation; lifecycle tests `database_with_version(CURRENT_SCHEMA_VERSION+1)` rejected | **DONE** | v7 rejects v6 (matches memory: "chat schema v7 rejects v6"). |
| 2.2 | D10 streaming deltas | `agena-storage/src/store/facade.rs:43` — `STREAMING_FLUSH_DELTA_COUNT = 8`; `types.rs:228` — `content_text_delta`; buffered coalescing + mandatory tail flush + notification-after-flush (facade.rs:819-876, 1132-1138) | **DONE** | Storage side complete; engine side streams through it in T6. |
| 2.3 | Run marker persistence | `agena-storage-sqlite/src/engine.rs:551` — `kind: "run"`; `is_run_marker()`; `schema_invariants.rs:71-75` — `agena_parts_run_marker_is_batch_root` + `agena_parts_run_marker_root_immutable` triggers | **DONE** | |
| 2.4 | Compaction part shape | `facade.rs:267-274` / `:1377-1398` — `compact_session` writes `run_kind="compaction"` run marker whose content is `CompactionContent{summary, window}`; typed struct at `crates/agena-runtime-contracts/src/part_content.rs:386`; facade tests `compact_session_starts_a_compaction_run_and_clears_anchors` + `compaction_part_content_records_the_checkpoint_summary` (:2054, :2081) | **DONE** (storage) | R4b selection (`prompt_window.active_prompt_messages_for_model` picks parts after last compaction part) is **IN-FLIGHT** — prompt_window.rs is mid-T6 rewrite. |
| 2.5 | Gate-10 sealed SQL boundary | `rg -l 'agena_(parts\|session_parts\|sessions\|execution_leases\|sequences\|usage\|idempotency)' crates apps tools packages examples scripts --glob '*.rs'` → only `agena-storage-sqlite` (8 files incl. model_catalog_repository) + `agena-storage/src/store/in_memory.rs` (comment naming the trigger) | **DONE** | No manager/application/API/TUI/CLI module issues chat SQL. |
| 2.6 | Typed content layer | `crates/agena-runtime-contracts/src/part_content.rs` — 13 typed kinds (run/text/think/tool_call/tool_result/file_ref/paste_ref/skill_ref/notice/hook/compaction/error/interaction), `#[serde(flatten)] extra` lossless bucket, lenient decode + 9 round-trip tests | **DONE** | Lives at contracts top level (out of `message/`), so it survives T7's message deletion. |

---

## 3. Identifier audit (v1-name residue)

R6 gate: `rg -n 'Message\b|MessagePart|PartContent|RuntimeActivity|MessageMetadata|TranscriptSnapshot|TranscriptPatch|TurnSnapshot|ExecutionStatus|PartKind\b|reserve_message_ids|reserve_processor_ids|OPERATION_ID_METADATA_KEY' crates apps tools --glob '*.rs'` → expected **zero** after T6+T7.

**T8/T9 add-on tokens** (added 2026-08-10, tracked in §8): T9 name kills — `SessionProjectedMessage`, `list_projected_messages`, `submit_user_message`, `SessionUserMessageRequest`, `MessageProviderState`, `MessageResource`, `MessagePartResource`, `DebugMessageOutput`, `MAX_COMPACTOR_MESSAGE_CHARS`; T8 PartContent kills — `PartContent`, `RuntimeActivity`, `decode_part_content`, `part_content_from_typed` (contracts deletion), `PartContent::Activity` construction sites. Wire names that MUST SURVIVE (do not report): `ChatMessage`, `InboundMessage`/`ClientMessage`/`ServerMessage`, SSE `message` event, MCP/LSP terms, `MessageSubtaskRequest`, domain `MessageSource`, DB `message_count`/`last_message_at`/`source_message_id`, wire `message/submit`+`messages/list`, REST `/api/v1/sessions/{id}/messages`, `turns` JSON key (serde-renamed `messages`), `PromptCompactionMessage`/`recent_messages`.

| Crate | `message::` path files | v1 `Message*` struct hits | Verdict / notes |
|---|---|---|---|
| `agena-runtime-session` | 17 | 18 | **Transitional (T6 in flight)** — the whole v1 bridge. Expected; do NOT report as new. |
| `agena-runtime-contracts` | 3 | 6 | **Transitional (T7 target)** — `message/{message,metadata,part}*.rs` + `part/mod.rs:3-4` stale comment. Deleted by T7. |
| `agena-application` | 1 | 0 | `application.rs:1034-1057` — part types only (`agena_runtime::message::{PartContent, RuntimeActivity, NoticePart}`). T7 §2c rename `message::`→`part::`. |
| `agena-bundled-plugins` | 8 | 2 | 8 files = part-type imports (cron/fs/interaction/lsp/mcp/shell/tasks/workflow). T7 §2c rename. `Message*` hits = `tokio_tungstenite::Message` (web/plugin.rs CDP sink) — not v1. |
| `agena-runtime-tools` | 22 | 0 | 22 files = tool-input part types (`crate::message::{Shell*,Read,Grep,…}ToolInput`). T7 §2c rename. No v1 structs. |
| `agena-runtime-provider` | 3 | 2 | `registry/mod.rs`, `wire_message.rs`, `chat_wire.rs` — part-type imports (T7 §2c). `Message*` hits = own `ChatMessage`/`ChatDeltaOrMessage` + doc comments; **no v1 `Message` struct remains in production** (T5 landed: `project_persisted(&[Part])` at wire_message.rs:124). Remaining v1 `Message` is test-only (T3). |
| `agena-runtime-provider-adapters` | 6 | 6 | part-type imports (T7 §2c) + **T3 test fixtures still build v1 `Message`** (`gemini_adapter.rs:870`, `openai_response_builders.rs:881` tool_api_history_tests). T3 rewrites them off v1. |
| `agena-runtime` | 2 | 1 | `lib.rs:37` `pub use …::message;` (T7 §2b) + `runtime/host_client/mod.rs:11` part types (T7 §2c). `Message*` hit = `MessageProviderState` (kept). |
| `agena-api` | 0 | 2 | Own resource types `MessageResource`/`MessageMetadata`/`MessagePart*` — **NOT contracts v1** (T7 §3d; their track = T4). |
| `agena-api-server` | 0 | 3 | `tungstenite::Message` / axum `ws::Message` — unrelated. |
| `agena-client` | 0 | 1 | `tungstenite::Message` — unrelated. |
| `agena-tui-app` / `agena-tui-transcript` / `agena-tui-session` / `agena-tui-backend` | 0 / 0 / 0 / 0 | 7 / 4 / 2 / 3 | agena-api resource types + own/plugin/tungstenite types. **Zero `agena_runtime_contracts::message` imports** (verified). T7 §3d lookalikes. |
| `agena-runtime-session-core` | 0 | 0 | Clean. |
| `agena-storage` / `agena-storage-sqlite` | 0 / 0 | 0 / 0 | Clean. |
| `agena-web` | 0 | 0 | Clean. |

Additional v1 tokens:

- `runtime_activity` (snake): **0 hits** workspace-wide.
- `segment_id` / `activity_id` / `operation_id`: die with contracts `MessagePart` (T7). Mirror fields remain in **other** type families, out of T7 scope:
  - `agena-api/src/message_part.rs:36` (`segment_id`) and `session/session_query_service.rs:89` (`SessionProjectedMessagePart.segment_id`) — agena-api/session own types.
  - `agena-domain/activity.rs`, `agena-macro-core` — unrelated domain/macro activity ids.

---

## 4. JSON-RPC v2 (redefined, NOT deleted) — `crates/agena-api-server/src/jsonrpc/`

| # | Item | Status | Notes |
|---|---|---|---|
| 4.1 | `message/submit` → run+parts | **DONE** | `protocol.rs` `SubmitMessageResult{run_id, parts: Vec<SessionTranscriptPart>}`; comment states v1 `status`/`text` replaced by run marker state + text parts. |
| 4.2 | `messages/list` → parts list | **DONE** | `ReadMessagesResult{parts: Vec<SessionTranscriptPart>}`. |
| 4.3 | `events/subscribe` removed | **DONE** | `protocol.rs:89-92` — replaced by SessionChange part-patch notifications. |
| 4.4 | Notifications = part patches | **DONE** | `AppServerNotification` = `PartAdded/PartUpdated/PartRemoved/SessionMetaUpdated/PermissionRequest/SessionStateChanged`. v1 `MessageDelta`/`ToolEvent` gone (`protocol.rs:229-234`). |
| 4.5 | Server dispatch + broadcast | **DONE** | `server.rs` dispatches all 6 v2 methods; `message/submit` publishes `SessionStateChanged` + `PartAdded` per part. |
| 4.6 | No v1 message shapes in wire | **DONE** | `"type":"activity"`, `MessageDelta`, `ToolEvent` → 0 hits in jsonrpc. No `MessageResource`-from-v1-contracts construction anywhere in api-server/api/client. |
| 4.7 | VS Code client adapted to v2 | **IN-FLIGHT** | `packages/agena-vscode/src/extension.ts` reads `result.parts` (kind/role/state/content) instead of v1 `text`. **FLAG**: extension spawns `agena app-server --transport stdio` (extension.ts:34) but the CLI subcommand is `rpc-server` (clap `RpcServer`, `crates/agena-cli/src/cli/mod.rs:122`, help text :83) — `app-server` does not resolve today. Verify the parallel A9/B1 worktree aligns the name (CLI rename or extension change) before the VS Code smoke. |

---

## 5. REST + Web

| # | Item | Status | Notes |
|---|---|---|---|
| 5.1 | REST `SessionExecutionResource.transcript` → v2 parts | **DONE** | `crates/agena-api/src/resource.rs:1196` — `parts: Vec<SessionTranscriptPart>`; comment "Replaces the v1 TranscriptSnapshot aggregate". |
| 5.2 | `crates/agena-web` imports no contracts `message` | **DONE** | `rg 'contracts::message|::message::|agena_runtime::message' crates/agena-web` → 0. |
| 5.3 | `packages/agena-web-ui` imports no contracts `message` | **DONE** | 0 hits (TS). Chat render path is v2-derived: `SessionPart` + `partsToMessages` (chatRenderModel.test.ts) + SSE `streamSessionChanges`/`applySessionChange`. |
| 5.4 | Web-ui R5 gate: `TranscriptSnapshot`/`TurnSnapshot` gone | **DONE** | 0 hits in web-ui. |
| 5.5 | Web-ui R5 gate: `getSessionState`/`DomainEventRecord`/`fetchEvents` gone | **IN-FLIGHT** | Still present: `agenaApi.ts:2161` `getSessionState`, `:1195` `DomainEventRecord`, `:2214-2251` `/api/v1/events` polling. Plan gate wants zero; these are the B2 web-ui migration remainder. |
| 5.6 | `agena-tui-*` transcript on v2 | **DONE** (A7 merged) | TUI main transcript rendering already migrated (recent commit `d7dc06c9 feat(tui): migrate main transcript rendering to v2 parts (A7)`). tui crates import no contracts `message`. |

---

## 6. E2E smoke inventory (manual, post-merge)

Per plan: send / stream / cancel / compact / fork over the v2 parts face, in all three clients.

| # | Surface | Launch command | What to verify | Status |
|---|---|---|---|---|
| 6.1 | TUI | `agena` (default) or `agena tui [--session N]` (bin `agena`, `apps/agena`) | user send streams assistant text with part states (in_progress→completed), run marker per turn, cancel stops a run, compact collapses to a `compaction` marker with summary, fork creates a shared-prefix child — all rendered from v2 parts | **PENDING** (needs green build, post-T6) |
| 6.2 | Web UI | `cd packages/agena-web-ui && bun run dev` (vite) against `agena server` (REST, 127.0.0.1:3210) | chat send/stream/cancel/compact over v2 REST parts + SSE `SessionChange` part patches (no polling regression) | **PENDING** |
| 6.3 | VS Code | Extension `agena.start` → `agena app-server --transport stdio` | prompt submit streams assistant text derived from `text` parts; permission replies round-trip | **PENDING** — blocked until 4.7 command-name mismatch resolved |
| 6.4 | Provider probe suite (optional) | `cargo run -p agena-e2e --bin dsv4f_tool_api_suite` | tool API against real provider (not a v2-parts assertion; regression guard only) | **PENDING** |

---

## 7. Final R6 audit gates (post-merge)

| # | Gate | Status |
|---|---|---|
| 7.1 | R6 identifier audit (§3 regex) returns zero | **PENDING** (today: residue only in session [T6] + contracts [T7] + `message::` part-type renames [T7 §2c]) |
| 7.2 | Gate-10 sealed SQL audit still only hits `agena-storage-sqlite` | **DONE** (re-verified this tree, §2.5) |
| 7.3 | `cargo test --workspace` green | **DONE** | 2026-08-10, exit 0 (see §1.2). |
| 7.4 | clippy -D warnings / fmt / diff --check green | **DONE** | 2026-08-10 (§1.3-1.5). |
| 7.5 | Bench `v2_store` no regression | **DONE** | 2026-08-10 — improved (§1.6). |

---

## 8. T8 / T9 — PartContent elimination + message-name elimination (R6 add-ons)

Added 2026-08-10 per user directive ("PartContent彻底清除，以及message名字消灭…全部完成"). Specs: `docs/t8-partcontent-elimination.md`, `docs/t9-message-name-elimination.md`.

| # | Item | Worktree / branch | Status |
|---|---|---|---|
| 8.1 | **T9-A** — api `MessageResource`→`RunResource` etc., `MessageProviderState`→`PartProviderState` (contracts provider_state.rs), `CompletionInputRun` + `turns` serde-rename | merged `2d91e4d2` | **DONE** |
| 8.2 | **T9-B** — session projection contract rename (`SessionProjectedMessage*`→`SessionProjectedRun*`, `list_projected_messages`→`list_projected_runs`, consumers in application/execution.rs, cli cli_render/cli_runtime_helpers/mod) + `PartProviderState` session propagation (processor.rs/helpers.rs/parts.rs/store.rs) + `submit_user_message`→`submit_user_run`-family + CLI `DebugMessageOutput`→`DebugRunOutput` + `MAX_COMPACTOR_MESSAGE_CHARS`→`MAX_COMPACTOR_RUN_CHARS` | merged (T9-B, 50 files pure rename 385/385) | **DONE** |
| 8.3 | **T8 stage 1** — provider `wire_message.rs` PartContent→TypedContent, dropped `decode_part_content`/`part_content_from_typed` from provider (enables stage-4 contracts deletion) | merged (T8-stage1) | **DONE** |
| 8.4 | **T8 stages 2-3** — application.rs:1034 latent notice-banner bug (decode via `decode(&part.kind,&part.content)`), session consumers (store/history/sessions/replies) PartContent→TypedContent | merged (T8 stage2/3) | **DONE** |
| 8.5 | **T8 stage 4** — contracts: delete `PartContent`/`RuntimeActivity`/`decode_part_content`/`part_content_from_typed`, `*_from_*` helpers pub + mirror consolidation, `ExecutionControl<T>`→TypedContent, tree-wide fmt/clippy | merged (T8 stage4) | **DONE** |
| 8.6 | **T8 stage 6** — zero-grep `PartContent`/`RuntimeActivity` workspace-wide + workspace check | **DONE** | 2026-08-10 — `PartContent`/`RuntimeActivity`/`decode_part_content`/`part_content_from_typed` → 0 hits workspace-wide (independently re-verified); check/test/clippy/fmt all green. |
| 8.7 | T8/T9 KEEP list intact (wire shapes, REST routes, SSE/message/submit, serde `turns` key) | **DONE** | JSON-RPC `"message/submit"`+`"messages/list"` (protocol.rs:84/87), REST `/api/v1/sessions/{session_id}/messages` (lib.rs:279), `#[serde(rename="messages")]` turns (contract.rs:1846) — all verified live. |
| 8.8 | `ExecutionControl<T>` carrier (`session/mod.rs:23`) → `TypedContent` once contracts free | **DONE** | Now `agena_runtime_contracts::part_content::TypedContent` (T8 stage 3). |

---

## Open flags (found during pre-audit)

1. **VS Code spawn command mismatch** — extension spawns `agena app-server --transport stdio`; CLI exposes `rpc-server`. Must be reconciled before 6.3.
2. **Web-ui R5 gate residue** — `getSessionState` + `DomainEventRecord`/`/api/v1/events` polling still in `packages/agena-web-ui` (B2 migration remainder).
3. **Downstream crates not compiled by baseline** — dependents of `agena-runtime-session` were skipped; their green status is only provable after T6.
