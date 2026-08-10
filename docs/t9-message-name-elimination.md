# R6-T9 — Message-name elimination plan

Branch: `research/db-design-audit` (HEAD `b0fa18d1`, read-only audit — this document is the
only artifact produced; no source file was modified).

## 1. Purpose

After T7 (contracts v1 `Message`/`MessagePart`/`MessageMetadata` deletion, branch
`parallel/r6-t7-final`, commit `d79da777`, **unmerged**) and T6 (session v1-bridge removal,
in flight), the remaining identifiers carrying the v1 "message" name are naming legacy, not
load-bearing v1. This is the plan to rename them to the v2 vocabulary (**part / run / turn**)
or to explicitly keep them with a reason.

Ground rule: **do not rename legitimate error `.message` fields, third-party protocol terms
(git, MCP, LSP, JSON-RPC, SSE "message" default event, OpenAI/Anthropic/Ollama wire shapes),
or plugin-SDK chat-hook terms.** Those are KEEP with a reason.

Terminology used below: a v2 **run** = a run-marker part + its content parts; the transcript
is a flat list of parts.

---

## 2. Full inventory

Grep basis: `grep -rniE "\bmessages?\b" crates --include="*.rs"` (2021 raw hits), filtered to
non-comment declarations. Families below are the actionable surface.

### 2.1 agena-api REST DTO family — RENAME (primary), DELETE viable

All in `crates/agena-api/src/` — the API's own resource type family (no contracts v1 import).
Production construction is **nil** (verified: no api-server/application code builds these;
only tui crates construct them, and most sites are `#[cfg(test)]`). They are a naming legacy.

| item | location | what | proposed name |
|---|---|---|---|
| `MessageRole` | `resource.rs:1328` | wire role enum (serde snake_case) | `RunRole` |
| `MessageStatus` | `resource.rs:1339` | wire state enum | `RunStatus` |
| `MessageUsage` | `resource.rs:1357` | token/cost | `RunUsage` |
| `MessageSource` | `resource.rs:1369` | origin enum | `RunSource` |
| `MessageMetadata` | `resource.rs:1378` | display/lineage metadata | `RunMetadata` |
| `MessageResource` | `resource.rs:1442` | transcript DTO (id, role, state, …, `parts`) | `RunResource` (or `SessionRunResource`) |
| `MessageSkillReference` | `resource.rs:1460` | skill ref | `RunSkillReference` (or `PartSkillReference`) |
| `MessageAttachment` | `resource.rs:1473` | part attachment DTO | `PartAttachment` |
| `MessageAttachmentKind` | `resource.rs:1499` | kind enum | `PartAttachmentKind` |
| `MessageAttachmentSource` | `resource.rs:1510` | source enum | `PartAttachmentSource` |
| `MessagePartResource` | `message_part.rs:21` | part DTO | `PartResource` |
| `MessagePartKindResource` | `message_part.rs:47` | kind enum | `PartKindResource` |
| `MessagePartDetailResource` | `message_part.rs:75` | part content enum | `PartDetailResource` |
| `MessageTextPartResource` | `message_part.rs:88` | text content | `TextPartResource` |
| `MessageReasoningPartResource` | `message_part.rs:96` | reasoning content | `ReasoningPartResource` |
| `MessageAttachmentPartResource` | `message_part.rs:123` | attachment content | `AttachmentPartResource` |
| `MessageSkillReferencePartResource` | `message_part.rs:130` | skill-ref content | `SkillReferencePartResource` |
| `MessageErrorPartResource` | `message_part.rs:136` | error content | `ErrorPartResource` |
| `MessageHookPartResource` | `message_part.rs:144` | hook content | `HookPartResource` |
| `MessageRequestPartResource` | `message_part.rs:162` | request content | `RequestPartResource` |
| module `message_part` | `lib.rs:36` (`pub mod message_part;`) | module path | `part` (module) — but see collision note below |
| `MessagePartChanged` | `agena-tui-session/src/lib.rs:115` (part of `SessionLiveEvent`) | UI live event | `RunPartChanged`/`PartChanged` (UI-internal, optional) |
| `SegmentProjected…` field `segment_id` | `message_part.rs:36`, `resource/…` | wire field name | keep (field name; see wire note) |

Consumers that must update (all tui):
- `agena-tui-app`: `lib.rs` (test fixture `TranscriptFixture`, `#[cfg(test)]`), `app_tests.rs`
  (~25 sites), `app_types/session.rs:246` (`#[cfg(test)] messages: Vec<MessageResource>`),
  `transcript_state.rs:941` (`#[cfg(test)]` `From<&MessageResource> for TranscriptEntry`).
- `agena-tui-transcript`: `render_model.rs` (test-reachable `From<&MessageResource>` /
  `From<MessagePartDetailResource>`), `renderer/transcript_render/message_render.rs:22,790`
  (`canonical_resource_attachment` — **the only production construction**, builds
  `MessageAttachment`/`MessageAttachmentKind`/`MessageAttachmentSource`), `:944`
  `rewind_message_preview` (exported, currently uncalled).
- `agena-tui-session`: `session_helpers.rs:10,67` `is_rewind_target_message` (exported,
  currently uncalled).

`queries.rs` — **verified clean** (0 `message` hits); the task brief's mention of
"message-named types in src/queries.rs" is stale.

> Recommendation nuance: because production construction is nil and the only live build is
> `canonical_resource_attachment` (tui-transcript), **DELETE** of the whole family (and
> migrating that one helper + the `#[cfg(test)]` fixtures to v2 `SessionTranscriptPart`) is
> the stronger end state. RENAME is the conservative default that preserves the type family.
> Pick one track; do not both.

### 2.2 Session projection contract — RENAME (T6-gated)

`crates/agena-runtime-session/src/session_query_service.rs`:

| item | location | proposed name |
|---|---|---|
| `SessionProjectedMessageHeader` | `:35` | `SessionProjectedRunHeader` |
| `SessionProjectedMessage` | `:67` | `SessionProjectedRun` |
| `SessionProjectedMessagePart` | `:79` | `SessionProjectedPart` |
| trait method `list_projected_messages` | `:369` (+ mock impl `:443`) | `list_projected_runs` |
| manager method `list_projected_messages` | `session/manager/sessions.rs:550` | `list_projected_runs` |
| trait impl | `session/manager/history.rs:893-926` | rename (body is being rewritten parts-native by T6) |
| `project_message_part` (history.rs:1135-1136) | takes v1 `MessagePart` → T6 changes to `Part` | keep name or rename to `project_projected_part` |

Re-exports (must update):
- `agena-runtime/src/lib.rs:197-198` re-exports the three types above.

Full consumer list (every file to touch for the rename — §4 details the 3 external ones):
1. `agena-application/src/service/execution.rs:506` — `session_transcript_parts`
2. `agena-application/src/session.rs:55` — `project_session_transcript(&[SessionProjectedMessage])`
3. `agena-cli/src/cli/cli_render.rs:673` — `render_debug_session_command`
4. `agena-cli/src/cli/cli_render.rs:851` — `render_exec_command` (via `last_assistant_text_from_projection`)
5. `agena-cli/src/cli/cli_runtime_helpers.rs:165,176` — `last_assistant_text_from_projection`,
   `projected_message_visible_text`
6. `agena-cli/src/cli/mod.rs:1360-1372` — `DebugSessionOutput.messages: Vec<DebugMessageOutput>` and
   `DebugMessageOutput` (rename to `DebugRunOutput`; `messages` field is CLI-internal, only
   serialized for `--json` debug output)
7. `agena-runtime-session/src/session/manager/tests.rs:602` — test
8. `agena-runtime/src/lib.rs:197-198` — re-export

### 2.3 Provider completion input — RENAME (independent, can run now)

All defined in `crates/agena-provider/src/contract.rs`; the internal model-API input shape
produced by `project_completion_input(&[Part])` (`agena-runtime-provider/src/provider/wire_message.rs:284`).

| item | location | proposed name |
|---|---|---|
| `CompletionInputMessage` | `contract.rs:1803` | `CompletionInputRun` (a run = role + parts + provider_state) |
| `CompletionInputPart` | `contract.rs:1706` | keep (already part vocabulary) — no change |
| `CompletionInputAttachment` (+ Kind/Source) | `contract.rs:1681/1660/1671` | keep — attachment is part vocabulary |
| `CompletionInputProviderState` | `contract.rs:1772` | keep — already provider_state vocabulary |
| `CompletionInputToolResultStatus` | `contract.rs:1763` | keep |
| `CompletionRequest.messages` field | `contract.rs:1846` | `turns` or `runs` (Rust field; see wire note) |
| `CompletionRequestInputs.messages` | `agena-runtime-session/src/completion_request.rs:9` | `turns` / `runs` |
| `CompletionInputMessageRole` | — | **does not exist**; role is `agena_domain::Role` (verified) |

`CompletionInput*` occurrences: **167 across 16 files**:
`agena-provider` (contract.rs, lib.rs, tool_mode_policy.rs),
`agena-runtime-provider` (chat_wire.rs, wire_message.rs, registry/completion.rs),
`agena-runtime-provider-adapters` (bedrock_adapter.rs, anthropic_requests.rs, gemini_adapter.rs,
ollama.rs, openai_response_builders.rs, openai_response_types.rs),
`agena-runtime-session` (completion_request.rs, manager/permission_service.rs,
manager/compact.rs, processor/helpers.rs).

Helper/fn names to follow the rename (Rust-only):
- `wire_message.rs:81,84,213,245,284,303,335` — `project`, `wire_part_from_completion_input`,
  `attachment_item_from_completion_input`, `project_completion_input`, `completion_input_part_from_wire`, …
- `tool_mode_policy.rs:102-152` — local `message`/`projected_messages`/`result_messages` vars
  and `message: CompletionInputMessage` param → run vocabulary.
- `chat_wire.rs:668,690,696,716,742,752,835,852,863` — `request_to_chat_messages_…`,
  `assistant_messages_from_parts`, `tool_messages_from_parts`, `assistant_reasoning_text(message: &CompletionInputMessage)`, …
- session `completion_request.rs`, `manager/compact.rs` (local `messages`), `permission_service.rs`.

### 2.4 Re-export + module paths (T7-final-B, include for completeness)

| item | location | action |
|---|---|---|
| `pub use agena_runtime_contracts::{authorization, identity, message, permission}` | `agena-runtime-session/src/lib.rs:26` | switch `message` → `part, provider_state` — **only after T6 removes the session's internal `crate::message::` refs** |
| `pub use agena_runtime_contracts::{authorization, message}` | `agena-runtime-session-core/src/lib.rs:10` | switch → `part, provider_state` — **can land immediately** (session-core has zero `crate::message::` refs; verified) |
| doc comment "`message`" | `agena-runtime-session/src/lib.rs:21` | update |

All other re-export crates (`agena-runtime-tools/src/lib.rs:11`, `agena-bundled-plugins/src/lib.rs:32`,
`agena-runtime/src/lib.rs:37`) already point at `part`/`provider_state` (T7-rename, merged) — no change.

Session-internal `crate::message::…` / `crate::message::{…}` imports (doom_loop, processor,
transcript, prompt_window, store, mod.rs, cost, tool_calls, run, parts, compact, manager/mod.rs)
all die with T6/T7 — **not T9 work**; listed here only for completeness.

### 2.5 MessageProviderState — RENAME (recommended) — see §5

### 2.6 Other identifiers — classified

**RENAME (T9 scope, Rust-only):**

| item | location | proposed |
|---|---|---|
| `SessionUserMessageRequest` | `session_requests.rs:60` | `SessionUserRunRequest` (or `SessionPromptRequest`) |
| `submit_user_message` (storage trait + impls) | `agena-storage/src/store/facade.rs:153,1059,3080`, `engine.rs:197`, `in_memory.rs:966`, `agena-storage-sqlite/src/engine.rs:1139`, `manager/runs.rs` | `submit_user_prompt` / `submit_user_run` |
| `SessionMessageRequest` | `agena-application/src/dto/sessions.rs:55` | `SessionRunRequest` (REST body; fields `run`/`document` unchanged → no wire impact) |
| `SubmitMessageParams` / `SubmitMessageResult` / `ReadMessagesParams` / `ReadMessagesResult` | `agena-api-server/src/jsonrpc/protocol.rs:111,124,200,206` | `SubmitRunParams` / `SubmitRunResult` / `ReadPartsParams` / `ReadPartsResult` (Rust types only; the wire method names stay — §2.7) |
| `PendingUserMessage` (tui-app), `AppMessage` (tui-app event) | tui-app | optional UI-internal; KEEP preferred |
| `PromptCompactionMessage` / `PromptCompactionContent` field `recent_messages` | `agena-runtime-session-core/src/model.rs:151,160,165` | KEEP — persisted session-runtime JSON (§3) |
| `MAX_COMPACTOR_MESSAGE_CHARS` | `agena-runtime-session/src/compaction_policy.rs:11` | `MAX_COMPACTOR_RUN_CHARS` (const, Rust-only) |
| `assistant_message_count`/`user_message_count` | `agena-rollout/src/share.rs:36-37,74-100` | `assistant_run_count`/`user_run_count` (Rust-only; optional) |
| `DebugMessageOutput`/`DebugSessionOutput.messages` | `agena-cli/src/cli/mod.rs:1360-1372` | `DebugRunOutput` / field `runs` (CLI-internal) |
| tui-session `SessionLiveEvent` variants `UserMessageAppended`/`MessagePartChanged`/`AssistantMessageFinished` | `agena-tui-session/src/lib.rs:110-116` | optional UI-internal |
| `is_rewind_target_message`, `rewind_message_preview`, `message_render.rs` module, `pending_user_message` helpers | tui crates | optional UI-internal |

**KEEP (legitimate / third-party / protocol / persisted):**

| item | reason |
|---|---|
| every `struct X { message: String }` / error `.message` field (agena-failure, agena-api error.rs, domain doom_loop/provider_retry/background_activity, runtime service errors, api-server error.rs, marketplace.rs, LSP protocol) | error/display-message convention |
| `ChatMessage`, `openai_chat_message` module, `ChatMessagesTransform*`, plugin SDK `hooks/chat.rs` (`ChatMessageInput/Patch`) | OpenAI chat wire + plugin chat-hook contract |
| `AnthropicMessage*`, `OllamaChatMessage*`, `GeminiLive*Message`, `ChatDeltaOrMessage`, `ChatCompletion*` | provider third-party wire shapes |
| `messages: Vec<...>` fields in provider request builders (`openai_chat_completion_request.rs:11`, `ollama_wire.rs:33`, `anthropic_requests.rs`) | model API JSON (models receive `messages`) |
| `InboundMessage` (protocol.rs:61), `ClientMessage`/`ServerMessage` (agena-api ws.rs), JSON-RPC `Request/Response/Notification` | JSON-RPC/WS protocol framing |
| SSE default event `"message"` (`agena-client/src/http.rs:188`, per SSE spec) | standard |
| MCP `PromptMessage`, `messages` (`agena-mcp-client/protocol.rs:280`, `agena-mcp-server/lib.rs:468-487`) | MCP protocol |
| `MessageSubtaskRequest` / host API `message_subtask` (plugin-sdk host_api.rs:87, plugin-host, tasks plugin) | stable plugin host RPC wire method + request; semantically a message injected into a subtask |
| `MessageSource` (domain `message_source.rs:22`) | domain-owned, persisted, used by session-core/compaction |
| `message_activity.rs` filename, `message_count`/`last_message_at`/`last_message_at_ms`/`first_message_at` (domain `session_summary.rs:50,53`, `usage_stats.rs:76-77`, storage `store/types.rs:328-330`, api `resource.rs:1015-1018`) | persisted DB columns + REST wire keys |
| `Message` in `agena-plugin-sdk` / `agena-macro-core` macro support | plugin hook descriptors |
| `tungstenite::Message`, `axum::ws::Message` | third-party |
| `CompletionInputProviderState`, `CompletionInputPart`, `CompletionInputAttachment*` | already part/provider vocabulary |
| `resource/activity.rs:31 message: Option<String>`, `resource.rs:482 message: Option<String>` | display text fields |

### 2.7 Wire protocol — re-verified (mostly KEEP)

`protocol.rs` confirmed v2 (read methods return `SessionTranscriptPart`; notifications are
`PartAdded`/`PartUpdated`/`PartRemoved`/…). The `message`-named wire surface is:

| wire item | location | decision |
|---|---|---|
| JSON-RPC method `message/submit` | protocol.rs:84 | **KEEP** (wire method name; clients call it — vscode extension.ts:56, web-ui agenaApi.ts) |
| JSON-RPC method `messages/list` | protocol.rs:87 | **KEEP** (wire method name) |
| REST route `POST /api/v1/sessions/{id}/messages` | agena-api-server lib.rs:279 | **KEEP** (wire route; sibling v2 route `/parts` exists) |
| REST `SessionResource` fields `message_count`, `last_message_at`, `source_message_id` | resource.rs:1004,1015,1018 | **KEEP** (wire JSON keys; built from domain `SessionSummary`) |
| SSE payloads | agena-api-server/src/sse.rs | clean — v2 `SessionChangeResource`/notification events; only local `message` vars |
| `SessionPresentation.message_count` | session_query_service.rs:23 | KEEP (feeds REST `SessionResource.message_count`) |
| persisted `recent_messages`/`PromptCompactionMessage` | session-core model.rs | KEEP (persisted session-runtime JSON) |
| `MessageResource`/`MessagePartResource` field names (`message_id`, `part_count`, `segment_id`, `parts`) | agena-api | KEEP the field names if renaming types only; renaming fields is a breaking DTO change with **no known client** (never serialized in production) |

Web/TS side: `packages/agena-web-ui` consumes v2 JSON-RPC (`message/submit`, `messages/list`)
and renders via `partsToMessages(...)` (agena-web-ui `chatRenderModel.ts:153`) — a UI-side
transform, **fine, no change**. Its TS `MessageResource` type (`agenaApi.ts:982`) is a
UI-internal reconstruction mirroring the Rust DTO fields; it is not parsed from the Rust
`MessageResource` wire DTO in production, so Rust renames do not break it. No TS change
required unless the field names themselves change.

---

## 3. Wire-impact check (risk items)

| # | rename | touches wire/persisted? | action |
|---|---|---|---|
| 1 | agena-api `Message*` → `Run*`/`Part*` Rust type names | No — serde struct/enum **names** are not emitted; only field/variant names are | Rename types freely. If also renaming fields (`message_id`→`run_id`, `part_count`, `segment_id`, `parts`), it is a breaking DTO contract change; **no production client parses these** (never serialized) — safe to do, but document in the REST changelog. |
| 2 | `SessionProjectedMessage*` → `SessionProjectedRun*`; `list_projected_messages` → `list_projected_runs` | No — runtime Rust projection, never serialized (`SessionPresentation` is the only serialized read contract, unchanged) | Rename freely; update 8 consumers (§2.2). |
| 3 | `CompletionInputMessage` → `CompletionInputRun`; `CompletionRequest.messages` → `turns`/`runs` | **No** — `CompletionRequest`/`CompletionRequestInputs` are Rust-intermediate; adapters convert to provider-native wire (`AnthropicMessage`, `ChatMessage`, …). No `serde_json::to_*` of a live `CompletionRequest` (verified) | Rename freely. Do **not** rename the provider-native `messages` arrays models receive. |
| 4 | `MessageProviderState` → `PartProviderState` | **No** — stored under the `provider_state` column/key; internal fields have no "message" | Rename freely (§5). |
| 5 | `SubmitMessageParams` etc. | No — Rust type names only; wire = method name `message/submit` + fields (`session_id`, `prompt`, …) which stay | Rename freely. |
| 6 | `SessionMessageRequest`, `submit_user_message`, `SessionUserMessageRequest` | No — Rust request/storage types; REST body fields (`run`, `document`) unchanged | Rename freely. |
| 7 | `PromptCompactionMessage` / `recent_messages` / `message_count` / `last_message_at*` / `source_message_id` | **Yes** — persisted session-runtime JSON + REST wire keys | **KEEP** (would require a DB migration and a REST/JSON-RPC contract change). |
| 8 | JSON-RPC method names `message/submit`/`messages/list`, REST route `/messages` | **Yes** — wire method/route | **KEEP** (stable); if a rename is ever wanted (`runs/submit`, `/runs`), it is a breaking change for web-ui + vscode — document and version. |

Net: **zero required wire changes.** Every forced-wire item is a KEEP. All T9 renames are
Rust-identifier-only.

---

## 4. `list_projected_messages` rename — the 3 external consumers in detail

Rename target: `list_projected_messages` → `list_projected_runs`,
`SessionProjectedMessage` → `SessionProjectedRun`, `SessionProjectedMessageHeader` →
`SessionProjectedRunHeader`, `SessionProjectedMessagePart` → `SessionProjectedPart`
(part type; `message_id` field → `run_id`, `segment_id` → keep or drop with T7's part model).

1. **`agena-application`**
   - `src/service/execution.rs:499-511` — `session_transcript_parts`: call
     `session_queries.list_projected_runs(session_id, true)`; local `messages` → `runs`.
   - `src/session.rs:54-85` — `project_session_transcript(runs: &[agena_runtime::SessionProjectedRun])`;
     loop `for run in runs`, `run.id`, `run.role`, `run.state`, `run.metadata`, `run.created_at`,
     `run.parts` (each `part.kind`/`part.status`/`part.summary`/`part.content`). Doc comment
     "Each projected message is one v2 run" → "Each projected run …". Update the call site
     `crate::session::project_session_transcript(&runs)`.
2. **`agena-cli`**
   - `src/cli/cli_render.rs:673` (`render_debug_session_command`): `queries.list_projected_runs(...)`;
     `.map(|run| DebugRunOutput { id: run.id, role: run.role, state: run.state,
     text: projected_run_visible_text(run) })`.
   - `src/cli/cli_render.rs:851` (`render_exec_command`): `last_assistant_text_from_projection(
     queries.list_projected_runs(...).await ...)`.
   - `src/cli/cli_runtime_helpers.rs:165-181` — `last_assistant_text_from_projection(
     Vec<agena_runtime::SessionProjectedRun>)` (rename `messages`→`runs`, `find(|run| ...)`)
     and `projected_message_visible_text` → `projected_run_visible_text(&agena_runtime::SessionProjectedRun)`.
   - `src/cli/mod.rs:1360-1372` — `DebugSessionOutput { session, runs: Vec<DebugRunOutput> }`,
     `DebugRunOutput`; serialized only in `--json` debug output (CLI-internal, no client).
3. **`agena-runtime-session` (trait/impl side, T6-gated)**
   - `session_query_service.rs:369` trait method + `:443` mock impl; `session/manager/sessions.rs:550`
     manager method; `session/manager/history.rs:893-926` impl; `session/manager/tests.rs:602` test.
   - `agena-runtime/src/lib.rs:197-198` re-export list update.

Verify after rename: `grep -rn "list_projected_messages\|SessionProjectedMessage" crates --include="*.rs"`
→ only expected (zero if fully renamed), then `cargo check -p agena-application -p agena-cli -p agena-runtime`.

---

## 5. MessageProviderState — recommendation: RENAME to `PartProviderState`

**Recommendation: rename `MessageProviderState` → `PartProviderState`.**

Rationale:
- It is a **kept v2 contract type** whose name is the only "message" survivor in contracts
  after T7. It is documented as "state attached to a message", but it is persisted on the
  **assistant run marker part** (`store.rs:597-627` reads `marker.provider_state`), so
  `PartProviderState` is the accurate v2 name. (`RunProviderState` is the alternative if the
  team prefers run-centric naming; pick one and be consistent — `PartProviderState` is
  recommended because the JSON column is `provider_state` on a part and the sibling
  `agena_provider::CompletionInputProviderState` already uses the `…ProviderState` shape.)
- **Zero wire/persisted impact**: it serializes as the value under the `provider_state`
  key; its fields (`assistant_reasoning_field`, `response_id`, `gemini_thought_signatures`,
  `anthropic_thinking_blocks`, `openai_reasoning_items`, `openai_chat_reasoning_details`,
  `copilot_reasoning_opaque`) contain no "message". Rust type + path only.

Full consumer list (rename updates):
- **Production — 5 files, 8 occurrences**:
  - `crates/agena-runtime-contracts/src/lib.rs:25` — re-export `pub use provider_state::PartProviderState;`
  - `crates/agena-runtime-session/src/session/store.rs:43,600`
  - `crates/agena-runtime-session/src/session/processor.rs:9`
  - `crates/agena-runtime-session/src/session/processor/parts.rs:2,372`
  - `crates/agena-runtime-session/src/session/processor/helpers.rs:3,60`
  - `crates/agena-runtime-provider/src/provider/wire_message.rs:28,368`
- **Tests — 2 files, 14 occurrences** (`#[cfg(test)]` only):
  - `crates/agena-runtime-provider-adapters/src/provider/gemini/gemini_adapter.rs:870,910,915,973,978`
  - `crates/agena-runtime-provider-adapters/src/provider/openai/openai_response_builders.rs:882,999,1004,1051,1060,1108,1120,1250,1260`
- **Dies with T7 (no action):** `agena-runtime-contracts/src/message/{message.rs:4,26, metadata.rs:6,8,10, mod.rs:12}`.

Note the type is structurally identical to `agena_provider::CompletionInputProviderState`
(agena-provider contract) — consider whether the two should merge into one `PartProviderState`/
`ProviderState` eventually (out of T9 scope, flag only).

---

## 6. Staged implementation plan

**Stage A — independent of T6/T7 (can land on current HEAD):**

1. **Provider completion input** (§2.3): rename `CompletionInputMessage` → `CompletionInputRun`
   and the `messages` fields → `turns`/`runs` across the 16 files. Mechanical.
   Verify: `cargo check -p agena-provider -p agena-runtime-provider -p agena-runtime-provider-adapters -p agena-runtime-session`
   then `cargo clippy --workspace -- -D warnings`.
2. **agena-api DTO family** (§2.1): rename types + module (or delete). Mechanical renames in
   `resource.rs`, `message_part.rs`, `lib.rs`; update tui consumers (`agena-tui-app`,
   `agena-tui-transcript`, `agena-tui-session`).
   Verify: `cargo check -p agena-api -p agena-tui-app -p agena-tui-transcript -p agena-tui-session`.
3. **`MessageProviderState` → `PartProviderState`** (§5).
   Verify: `cargo check -p agena-runtime-contracts -p agena-runtime-session -p agena-runtime-provider -p agena-runtime-provider-adapters`.
4. **JSON-RPC Rust request/result type renames** (§2.6: `SubmitMessageParams` → `SubmitRunParams`,
   `ReadMessagesParams` → `ReadPartsParams`, …) + `agena-client` + `agena-application`
   (`SessionMessageRequest` → `SessionRunRequest`, `SessionUserMessageRequest` →
   `SessionUserRunRequest`, `submit_user_message` → `submit_user_run`).
   Verify: `cargo check -p agena-api-server -p agena-client -p agena-application -p agena-storage -p agena-storage-sqlite`.
5. **session-core re-export** (§2.4): `agena-runtime-session-core/src/lib.rs:10` → `part, provider_state`.
   Verify: `cargo check -p agena-runtime-session-core`.

**Stage B — requires T6 merged (parts-native `list_projected_messages`):**

6. **Session projection contract** (§2.2, §4): `SessionProjectedMessage*` → `SessionProjectedRun*`,
   `list_projected_messages` → `list_projected_runs`, update the 8 consumers (execution.rs,
   session.rs, cli_render.rs ×2, cli_runtime_helpers.rs, cli/mod.rs, tests.rs, runtime lib.rs).
   Verify: `cargo check -p agena-runtime-session -p agena-runtime -p agena-application -p agena-cli`
   then the workspace `grep` in §4.
7. **CLI debug names** (§2.6: `DebugMessageOutput` → `DebugRunOutput`, `messages` → `runs`).
   Verify: `cargo check -p agena-cli`.

**Stage C — requires T7 (contracts v1 deletion) merged:**

8. **Re-exports** (§2.4): `agena-runtime-session/src/lib.rs:26` `message` → `part, provider_state`
   (after all `crate::message::` refs are gone), doc comments updated.
   Verify: workspace `grep` from t7 manifest §5 (no `message` module path refs), `cargo check --workspace`.

**Stage D — optional cleanups (UI-internal / low value, pick per team):**

9. tui-session `SessionLiveEvent` variant renames; tui transcript `message_render.rs` →
   `part_render.rs`; `MAX_COMPACTOR_MESSAGE_CHARS`; rollout count field names;
   `PendingUserMessage` → `PendingUserPrompt`; `project_session_text_lossy`-style leftovers.
   Each is Rust-only and safe; no wire impact.

---

## 7. Verify (workspace-wide, run at the end of each stage)

```bash
# No v1-style "Message" identifiers remain in the T9-renamed surfaces:
grep -rn "CompletionInputMessage\b" crates --include="*.rs"        # → nothing (Stage A1)
grep -rn "MessageProviderState" crates --include="*.rs" | grep -v "src/message/"  # → nothing (A3)
grep -rn "SessionProjectedMessage\|list_projected_messages" crates --include="*.rs" # → nothing (B6)
grep -rn "agena_runtime_contracts::message\|pub use .*message;" crates --include="*.rs" # → nothing (C8)
# Everything compiles:
cargo check --workspace
cargo clippy --workspace -- -D warnings
```

Definition of done: every item in §2 is either renamed (in the stages above), or KEEP with the
reason recorded in §2.6/§2.7; no wire field/method/route and no persisted DB key is changed.

---

## 8. Status of the brief's specific claims (verified against HEAD b0fa18d1)

- `CompletionInputMessageRole` — **does not exist** (role is `agena_domain::Role`).
- `agena-api/src/queries.rs` — **clean** (0 `message` hits).
- `message_has_visible_prompt_payload` (prompt_window.rs:309) — dies with T6 (operates on v1
  `&Message`).
- `as_message`/`to_message` — no live matches; only `From<&MessageResource> for TranscriptEntry`
  (tui, `#[cfg(test)]`-reachable) and `From<MessagePartDetailResource> for TranscriptPartContent`.
- `MessagePartChanged` — tui-session UI event (§2.1), optional rename.
- `partsToMessages` (web-ui) — UI transform, **no change**.
- `openai_chat_message` module, `ChatMessage`, git/mcp/lsp/jsonrpc/sse terms — KEEP.
